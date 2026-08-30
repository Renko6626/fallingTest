//! 粒子池（spec §3，SoA）：脱格自由飞行粒子的确定性状态。
//!
//! **顺序即 id 序**：`spawn` 按调用序 append，下标即遍历序；移除走保序压缩
//! （[`Particles::compact`]，`retain` 语义）——"串行按 id 提交" = 按下标顺序遍历。
//!
//! **容量限流**：`len == MAX_PARTICLES` 时 `spawn` 确定性拒绝（丢弃 + `rejected_total`
//! 计数），计数器**不入哈希**，只供诊断。
//!
//! **无 lifetime 字段**：重力保证要么落格要么出界，出界即确定性销毁（Task 4）。
//! 本任务（Task 3）只提供数据结构本体 + 生成/压缩骨架，运动积分留 Task 4。

use rayon::prelude::*;
use xxhash_rust::xxh3::Xxh3;

use crate::dda;
use crate::fixed::Fx;
use crate::material::{MaterialTable, MAT_AIR};
use crate::world::World;

/// 粒子池容量上限（总纲初值，`kernel-charter.md:64`）。
pub const MAX_PARTICLES: usize = 65536;

/// 重力加速度（spec §2 常量表，调参项）：0.25 格/tick²，每 tick 加到 `vy`。
/// 位模式与 `Fx::from_ratio(1, 4)` 一致（`fixed.rs` 金值测试钉死该位模式），
/// 这里写成字面量而非调用 `from_ratio` 是为了在 `const` 位置直接可用
/// （`from_ratio` 非 `const fn`）；`gravity_matches_from_ratio` 单测钉死等价性。
pub const GRAVITY: Fx = Fx(0x0000_4000);

/// 单 tick 逐轴速度上限（spec §2 常量表，调参项）：16 格/tick，同时是 DDA 单
/// tick 最坏步数的上界依据（`dda.rs` 的 `MAX_STEPS` 安全余量按此推算）。
/// 位模式与 `Fx::from_int(16)` 一致，理由同 [`GRAVITY`]。
pub const MAX_SPEED: Fx = Fx(16 << 16);

/// 粒子池：SoA 布局，下标即 id 序（架构 §3 state 条目既定）。
#[derive(Clone, Debug, Default)]
pub struct Particles {
    x: Vec<Fx>,
    y: Vec<Fx>,
    vx: Vec<Fx>,
    vy: Vec<Fx>,
    material: Vec<u8>,
    /// 单调计数：每次成功 `spawn` 递增一次，入状态哈希；不做索引、不回收。
    next_id: u32,
    /// 容量拒绝次数：诊断用，**不入哈希**。
    rejected_total: u64,
}

impl Particles {
    pub fn new() -> Particles {
        Particles::default()
    }

    pub fn len(&self) -> usize {
        self.x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }

    /// 按下标（id 序）只读访问单个粒子字段，供 Task 4 积分/提交阶段复用。
    pub fn x(&self, i: usize) -> Fx {
        self.x[i]
    }
    pub fn y(&self, i: usize) -> Fx {
        self.y[i]
    }
    pub fn vx(&self, i: usize) -> Fx {
        self.vx[i]
    }
    pub fn vy(&self, i: usize) -> Fx {
        self.vy[i]
    }
    pub fn material(&self, i: usize) -> u8 {
        self.material[i]
    }

    /// 容量拒绝诊断计数（不入哈希）。
    pub fn rejected_total(&self) -> u64 {
        self.rejected_total
    }

    /// 按下标写回位置与速度（Task 4 提交阶段：Fly 位置推进、悬浮降级重置用）。
    /// 材料在飞行/悬浮期间不变，故不在此设置。
    fn set_state(&mut self, i: usize, x: Fx, y: Fx, vx: Fx, vy: Fx) {
        self.x[i] = x;
        self.y[i] = y;
        self.vx[i] = vx;
        self.vy[i] = vy;
    }

    /// 追加一个粒子；容量满时确定性拒绝（丢弃 + 计数，返回 `false`）。
    /// 成功追加的粒子下标 = 追加前的 `len()`，即 id 序中的位置。
    pub fn spawn(&mut self, material: u8, x: Fx, y: Fx, vx: Fx, vy: Fx) -> bool {
        if self.len() >= MAX_PARTICLES {
            self.rejected_total += 1;
            return false;
        }
        self.x.push(x);
        self.y.push(y);
        self.vx.push(vx);
        self.vy.push(vy);
        self.material.push(material);
        self.next_id = self.next_id.wrapping_add(1);
        true
    }

    /// 保序压缩：按下标序保留 `keep[i] == true` 的粒子，其余移除
    /// （`retain` 语义，相对顺序不变）。`keep.len()` 必须等于 `len()`。
    /// Task 3 骨架无移除判据（无运动），Task 4 起用 Land/Gone 判定结果驱动。
    pub fn compact(&mut self, keep: &[bool]) {
        debug_assert_eq!(keep.len(), self.len(), "keep 掩码长度必须与粒子数一致");
        let mut w = 0usize;
        for (r, &k) in keep.iter().enumerate() {
            if k {
                if w != r {
                    self.x[w] = self.x[r];
                    self.y[w] = self.y[r];
                    self.vx[w] = self.vx[r];
                    self.vy[w] = self.vy[r];
                    self.material[w] = self.material[r];
                }
                w += 1;
            }
        }
        self.x.truncate(w);
        self.y.truncate(w);
        self.vx.truncate(w);
        self.vy.truncate(w);
        self.material.truncate(w);
    }

    /// 粒子层哈希（spec §9）：xxh3 按下标序（= id 序）折叠 `(x, y, vx, vy, material)`
    /// 原始位，末尾并入 `next_id` 与粒子数。空池也有稳定值（`next_id=0, len=0`）。
    pub fn hash_into(&self) -> u64 {
        let mut h = Xxh3::new();
        for i in 0..self.len() {
            h.update(&self.x[i].0.to_le_bytes());
            h.update(&self.y[i].0.to_le_bytes());
            h.update(&self.vx[i].0.to_le_bytes());
            h.update(&self.vy[i].0.to_le_bytes());
            h.update(&[self.material[i]]);
        }
        h.update(&self.next_id.to_le_bytes());
        h.update(&(self.len() as u64).to_le_bytes());
        h.digest()
    }
}

// ==================== 积分 + 落格提交（Task 4，spec §4b/c/d、§5）====================

/// 单粒子只读视图：`integrate` 的入参，脱离池 SoA 借用，纯函数友好、便于单测。
#[derive(Clone, Copy, Debug)]
struct ParticleView {
    x: Fx,
    y: Fx,
    vx: Fx,
    vy: Fx,
}

/// 粒子积分结局（spec §4b、§5）。`Land`/`Fly` 都携带 `pos`：`Fly.pos` 是真正要
/// 写回 SoA 的新坐标；`Land.pos` 是撞击前的连续坐标（`pos + vel`，未夹到格心），
/// 提交阶段只认 `cx,cy` 作为候选格，`pos` 留作诊断/未来法术命中结算复用
/// （`kernel-charter.md:65`），当前不影响提交逻辑。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Land { cx: i32, cy: i32, pos: (Fx, Fx) },
    Fly { pos: (Fx, Fx) },
    Gone,
}

/// 逐轴速度 clamp 到 `±MAX_SPEED`。
fn clamp_speed(v: Fx) -> Fx {
    if v > MAX_SPEED {
        MAX_SPEED
    } else if v < -MAX_SPEED {
        -MAX_SPEED
    } else {
        v
    }
}

/// 重力 → 逐轴 clamp（spec §4b 第一步）。纯函数：`integrate` 内部与
/// [`advance`] 写回 SoA 前各调用一次，两处结果必须一致（都是同一份实现）。
fn apply_gravity_and_clamp(vx: Fx, vy: Fx) -> (Fx, Fx) {
    (clamp_speed(vx), clamp_speed(vy + GRAVITY))
}

/// 纯函数：重力 → 逐轴 clamp → 对只读网格快照做 DDA（spec §4b、§5）。粒子间
/// 零交互、网格只读 ⇒ 任意线程数/调度同结果（并行安全论证与四相同型）。
fn integrate(p: ParticleView, world: &World) -> Outcome {
    let (vx, vy) = apply_gravity_and_clamp(p.vx, p.vy);
    match dda::trace((p.x, p.y), (vx, vy), world) {
        dda::Trace::Gone => Outcome::Gone,
        dda::Trace::Clear { end_pos } => Outcome::Fly { pos: end_pos },
        dda::Trace::Blocked { land_cell } => {
            Outcome::Land { cx: land_cell.0, cy: land_cell.1, pos: (p.x + vx, p.y + vy) }
        }
    }
}

/// 格 (cx,cy) 的中心连续坐标（Q16.16：格左上角 + 半格）。
fn cell_center(cx: i32, cy: i32) -> (Fx, Fx) {
    let half = Fx(1 << 15);
    (Fx::from_int(cx) + half, Fx::from_int(cy) + half)
}

/// 候选格 (cx,cy) 的落格消解（spec §5 提交期冲突）：候选仍是 air 直接用；
/// 否则按固定邻格序【上、左、右、左上、右上】（相对候选格）搜第一个 air 格
/// 降级；全占返回 `None`（调用方转悬浮）。出界邻格视为不可用（既非候选也不
/// 计入搜索命中）。
fn resolve_landing(world: &World, cx: i32, cy: i32) -> Option<(i32, i32)> {
    if world.cell(cx, cy).material() == MAT_AIR {
        return Some((cx, cy));
    }
    const NEIGHBOR_ORDER: [(i32, i32); 5] = [
        (0, -1), // 上
        (-1, 0), // 左
        (1, 0),  // 右
        (-1, -1), // 左上
        (1, -1), // 右上
    ];
    for (dx, dy) in NEIGHBOR_ORDER {
        let (nx, ny) = (cx + dx, cy + dy);
        if world.in_bounds(nx, ny) && world.cell(nx, ny).material() == MAT_AIR {
            return Some((nx, ny));
        }
    }
    None
}

/// 串行提交（spec §5 c/d）：按下标（= id）序应用 `outcomes`，返回保留掩码
/// （`false` = Land 已写入网格 / Gone 出界销毁，供 `compact` 使用）。拆成独立
/// 函数（而非内联进 [`advance`]）是为了让单测能直接注入合成 `Outcome`，验证
/// 冲突消解顺序而不必依赖真实重力积分的确切落格时机。
fn commit(
    particles: &mut Particles,
    world: &mut World,
    table: &MaterialTable,
    stamp: u8,
    outcomes: &[Outcome],
) -> Vec<bool> {
    assert_eq!(outcomes.len(), particles.len(), "outcomes 必须与粒子数一一对应");
    let mut keep = vec![true; outcomes.len()];
    for (i, outcome) in outcomes.iter().enumerate() {
        match *outcome {
            Outcome::Gone => keep[i] = false,
            Outcome::Fly { pos } => {
                let (vx, vy) = apply_gravity_and_clamp(particles.vx(i), particles.vy(i));
                particles.set_state(i, pos.0, pos.1, vx, vy);
            }
            Outcome::Land { cx, cy, .. } => {
                if let Some((lx, ly)) = resolve_landing(world, cx, cy) {
                    world.set_cell_stamped(table, lx, ly, particles.material(i), stamp);
                    keep[i] = false;
                } else {
                    // 全占：继续飞——候选格中心、速度清零（spec §5）。下 tick
                    // 重力重启寻底，每 tick 成本一次短 DDA，有界。
                    let center = cell_center(cx, cy);
                    particles.set_state(i, center.0, center.1, Fx::ZERO, Fx::ZERO);
                }
            }
        }
    }
    keep
}

/// 粒子相主入口（M1 spec §4 第 3 步 b/c/d）：并行积分 → 串行提交 → 保序压缩。
/// `stamp` = 本 tick 世代戳（调用方与网格四相同一口径传入）。
///
/// 并行积分只读 `world`（本 tick 网格四相之后的终态快照）与各粒子自身字段，
/// 粒子间零交互 ⇒ 产出的 `outcomes` 与线程数/调度顺序无关，只与 `world` 的
/// 内容和粒子池当前状态有关（并行安全论证与四相 P4 同型）；线程池复用调度器
/// 同一 `rayon::ThreadPool`（`pool.install`），不额外起池子。
pub(crate) fn advance(
    particles: &mut Particles,
    world: &mut World,
    table: &MaterialTable,
    pool: &rayon::ThreadPool,
    stamp: u8,
) {
    if particles.is_empty() {
        return;
    }
    let n = particles.len();
    let world_ref: &World = world;
    let particles_ref: &Particles = particles;

    let outcomes: Vec<Outcome> = pool.install(|| {
        (0..n)
            .into_par_iter()
            .map(|i| {
                let view = ParticleView {
                    x: particles_ref.x(i),
                    y: particles_ref.y(i),
                    vx: particles_ref.vx(i),
                    vy: particles_ref.vy(i),
                };
                integrate(view, world_ref)
            })
            .collect()
    });

    let keep = commit(particles, world, table, stamp, &outcomes);
    particles.compact(&keep);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx(v: i32) -> Fx {
        Fx::from_int(v)
    }

    #[test]
    fn spawn_order_is_traversal_order() {
        let mut p = Particles::new();
        for i in 0..5 {
            assert!(p.spawn(7, fx(i), fx(i * 2), fx(1), fx(0)));
        }
        assert_eq!(p.len(), 5);
        for i in 0..5 {
            assert_eq!(p.x(i as usize), fx(i));
            assert_eq!(p.y(i as usize), fx(i * 2));
        }
    }

    #[test]
    fn capacity_rejection_is_deterministic_and_repeatable() {
        let run = || {
            let mut p = Particles::new();
            let mut accepted = 0usize;
            let mut last_rejected = false;
            for i in 0..(MAX_PARTICLES + 1) {
                let ok = p.spawn(1, fx(i as i32), Fx::ZERO, Fx::ZERO, Fx::ZERO);
                if ok {
                    accepted += 1;
                } else {
                    last_rejected = true;
                }
            }
            (accepted, last_rejected, p.len(), p.rejected_total())
        };
        let (a1, r1, len1, rej1) = run();
        let (a2, r2, len2, rej2) = run();
        assert_eq!(a1, MAX_PARTICLES, "前 65536 个 spawn 必须全部成功");
        assert!(r1, "第 65537 个 spawn 必须被拒绝");
        assert_eq!(len1, MAX_PARTICLES);
        assert_eq!(rej1, 1);
        assert_eq!((a1, r1, len1, rej1), (a2, r2, len2, rej2), "重跑结果必须一致");
    }

    #[test]
    fn empty_pool_hash_is_stable() {
        let a = Particles::new();
        let b = Particles::new();
        assert_eq!(a.hash_into(), b.hash_into(), "空池哈希必须稳定");
    }

    #[test]
    fn hash_is_sensitive_to_particle_fields() {
        let mut a = Particles::new();
        a.spawn(3, fx(1), fx(2), fx(3), fx(4));
        let mut b = a.clone();
        // 改一个 vx，哈希必须变
        b.compact(&[false]); // 先清空 b……
        assert_eq!(b.len(), 0);
        b.spawn(3, fx(1), fx(2), fx(9), fx(4)); // …重建但 vx 不同
        assert_ne!(a.hash_into(), b.hash_into(), "vx 差异必须反映到哈希");
    }

    #[test]
    fn compact_preserves_order_of_kept_particles() {
        let mut p = Particles::new();
        for i in 0..4 {
            p.spawn(1, fx(i), Fx::ZERO, Fx::ZERO, Fx::ZERO);
        }
        p.compact(&[true, false, true, false]);
        assert_eq!(p.len(), 2);
        assert_eq!(p.x(0), fx(0));
        assert_eq!(p.x(1), fx(2));
    }

    // ==================== 常量金值：字面量位模式与 from_ratio/from_int 一致 ====================

    #[test]
    fn gravity_matches_from_ratio_one_quarter() {
        assert_eq!(GRAVITY, Fx::from_ratio(1, 4));
    }

    #[test]
    fn max_speed_matches_from_int_16() {
        assert_eq!(MAX_SPEED, Fx::from_int(16));
    }

    #[test]
    fn clamp_speed_respects_both_bounds() {
        assert_eq!(clamp_speed(MAX_SPEED + fx(5)), MAX_SPEED);
        assert_eq!(clamp_speed(-MAX_SPEED - fx(5)), -MAX_SPEED);
        assert_eq!(clamp_speed(fx(3)), fx(3));
    }

    // ==================== 串行提交：冲突消解（spec §5）====================

    fn test_table() -> MaterialTable {
        use crate::material::{Category, MaterialDef};
        let def = |id: u8, name: &str, category: Category, density: u16| MaterialDef {
            id,
            name: name.into(),
            category,
            density,
            color: (0, 0, 0),
        };
        MaterialTable::new(vec![
            def(0, "air", Category::Static, 0),
            def(1, "wall", Category::Static, 100),
            def(2, "sand", Category::Powder, 40),
        ])
        .unwrap()
    }

    fn spawn_n(n: usize) -> Particles {
        let mut p = Particles::new();
        for _ in 0..n {
            p.spawn(2, Fx::ZERO, Fx::ZERO, Fx::ZERO, Fx::ZERO);
        }
        p
    }

    #[test]
    fn commit_conflict_smaller_id_wins_candidate_cell() {
        let t = test_table();
        let mut w = World::new(1, 1, 0);
        let mut p = spawn_n(2);
        let outcomes = vec![
            Outcome::Land { cx: 5, cy: 5, pos: (Fx::ZERO, Fx::ZERO) },
            Outcome::Land { cx: 5, cy: 5, pos: (Fx::ZERO, Fx::ZERO) },
        ];
        let keep = commit(&mut p, &mut w, &t, 0, &outcomes);
        assert_eq!(keep, vec![false, false], "两者都应落格（候选 + 邻格降级），无一悬浮");
        assert_eq!(w.cell(5, 5).material(), 2, "id 0（更小）应占据候选格本身");
        assert_eq!(w.cell(5, 4).material(), 2, "id 1 应降级到邻格序第一位——候选正上方");
    }

    #[test]
    fn commit_conflict_falls_through_neighbor_order_when_first_slot_taken() {
        let t = test_table();
        let mut w = World::new(1, 1, 0);
        w.set_cell_stamped(&t, 5, 4, 1, 0); // 邻格序第一位（上）预先被占，强制降级到第二位（左）
        let mut p = spawn_n(2);
        let outcomes = vec![
            Outcome::Land { cx: 5, cy: 5, pos: (Fx::ZERO, Fx::ZERO) },
            Outcome::Land { cx: 5, cy: 5, pos: (Fx::ZERO, Fx::ZERO) },
        ];
        let keep = commit(&mut p, &mut w, &t, 0, &outcomes);
        assert_eq!(keep, vec![false, false]);
        assert_eq!(w.cell(5, 5).material(), 2);
        assert_eq!(w.cell(4, 5).material(), 2, "上邻格已占，应落到左邻格（邻格序第二位）");
    }

    #[test]
    fn commit_conflict_all_five_neighbors_occupied_downgrades_to_fly() {
        let t = test_table();
        let mut w = World::new(1, 1, 0);
        // 候选格 + 全部 5 个邻格（上/左/右/左上/右上）预先占满。
        for (dx, dy) in [(0, 0), (0, -1), (-1, 0), (1, 0), (-1, -1), (1, -1)] {
            w.set_cell_stamped(&t, 5 + dx, 5 + dy, 1, 0);
        }
        let mut p = Particles::new();
        // 非零初值，验证悬浮重置确实覆盖旧状态而非保留。
        p.spawn(2, fx(99), fx(99), fx(3), fx(-2));
        let outcomes = vec![Outcome::Land { cx: 5, cy: 5, pos: (Fx::ZERO, Fx::ZERO) }];
        let keep = commit(&mut p, &mut w, &t, 0, &outcomes);
        assert_eq!(keep, vec![true], "全占应转悬浮（继续飞），不得移除");
        let (ccx, ccy) = cell_center(5, 5);
        assert_eq!(p.x(0), ccx, "悬浮位置必须是候选格中心");
        assert_eq!(p.y(0), ccy);
        assert_eq!(p.vx(0), Fx::ZERO, "悬浮必须清零速度");
        assert_eq!(p.vy(0), Fx::ZERO);
    }

    #[test]
    fn commit_land_writes_material_and_stamp() {
        let t = test_table();
        let mut w = World::new(1, 1, 0);
        let mut p = spawn_n(1);
        let outcomes = vec![Outcome::Land { cx: 3, cy: 3, pos: (Fx::ZERO, Fx::ZERO) }];
        let keep = commit(&mut p, &mut w, &t, 42, &outcomes);
        assert_eq!(keep, vec![false]);
        let c = w.cell(3, 3);
        assert_eq!(c.material(), 2);
        assert_eq!(c.stamp(), 42);
    }

    #[test]
    fn commit_fly_advances_position_and_applies_gravity_to_velocity() {
        let t = test_table();
        let mut w = World::new(1, 1, 0);
        let mut p = Particles::new();
        p.spawn(2, Fx::ZERO, Fx::ZERO, Fx::ZERO, fx(2));
        let outcomes = vec![Outcome::Fly { pos: (fx(1), fx(2)) }];
        let keep = commit(&mut p, &mut w, &t, 0, &outcomes);
        assert_eq!(keep, vec![true]);
        assert_eq!(p.x(0), fx(1));
        assert_eq!(p.y(0), fx(2));
        // commit 内重算的重力+clamp 必须与 outcome 生成时用的是同一份实现：
        // vy 从 2 加上 GRAVITY(0.25) 应为 2.25。
        assert_eq!(p.vy(0), fx(2) + GRAVITY);
        assert_eq!(p.vx(0), Fx::ZERO);
    }

    #[test]
    fn commit_gone_removes_particle_without_touching_world() {
        let t = test_table();
        let mut w = World::new(1, 1, 0);
        let mut p = spawn_n(1);
        let outcomes = vec![Outcome::Gone];
        let keep = commit(&mut p, &mut w, &t, 0, &outcomes);
        assert_eq!(keep, vec![false]);
        assert_eq!(w.count_material(2), 0);
    }
}
