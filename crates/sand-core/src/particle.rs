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

use crate::cell;
use crate::dda;
use crate::fixed::Fx;
use crate::material::{Category, MaterialTable, MAT_AIR};
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
    /// 落格候选与向上兜底搜索都找不到空位的粒子数：诊断用，**不入哈希**
    /// （Task 4 修复轮 1 C1：替换掉会活锁的"全占转悬浮"路径后，这类粒子
    /// 直接判定为出界般移除，此计数器只供性能/密度调参观测，只有极端拥挤
    /// 的静态堆场景才会非零）。
    buried_total: u64,
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

    /// 落格兜底搜索耗尽的诊断计数（不入哈希，见字段文档）。
    pub fn buried_total(&self) -> u64 {
        self.buried_total
    }

    fn mark_buried(&mut self) {
        self.buried_total += 1;
    }

    /// 按下标写回位置与速度（Task 4 提交阶段：`Fly` 结局原样写回）。材料在
    /// 飞行期间不变，故不在此设置。
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

/// 粒子积分结局（spec §4b、§5）。
///
/// - `Fly.pos` / `Fly.vel`：真正要写回 SoA 的新坐标与新速度。重力+clamp 只在
///   `integrate` 内部算一次，结果随 `vel` 一并传给 `commit`——`commit` 原样
///   写回，不重算（Task 4 修复轮 1 M2：两处各算一次同一件事，曾是潜在的
///   "两份实现必须手动保持一致"隐患，改为单一算点更直接）。
/// - `Land.pos`：**未受阻时的积分终点**（`pos + vel`，若这一步没被挡本该到
///   达的地方），**不是**撞击点、也不是候选格坐标——真正的落格目标看
///   `cx,cy`。`commit` 当前不消费这个字段，暂留作诊断/未来法术命中结算的
///   挂钩点（`kernel-charter.md:65`），语义等 M4 法术层落地时再定型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Land { cx: i32, cy: i32, pos: (Fx, Fx) },
    Fly { pos: (Fx, Fx), vel: (Fx, Fx) },
    Gone,
}

/// 逐轴速度 clamp 到 `±MAX_SPEED`。`pub(crate)`（M1 Task 6 起）：爆炸溅射
/// 速度合成（方向 × 衰减 + 抖动）复用同一 clamp，避免两处独立实现同一条
/// 数值纪律（spec §6 point 3："速度分量最终 clamp 到 ±MAX_SPEED"）。
pub(crate) fn clamp_speed(v: Fx) -> Fx {
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
        dda::Trace::Clear { end_pos } => Outcome::Fly { pos: end_pos, vel: (vx, vy) },
        dda::Trace::Blocked { land_cell } => {
            Outcome::Land { cx: land_cell.0, cy: land_cell.1, pos: (p.x + vx, p.y + vy) }
        }
    }
}

/// 候选格 (cx,cy) 的落格消解（spec §5 提交期冲突，Task 4 修复轮 1 C1 改判）。
///
/// 候选本身是 air 直接用（候选恒是 DDA 已验证过在界内的格子；即便万一传入
/// 越界坐标，`World::cell` 对越界坐标兜底返回 `WALL_SENTINEL`，material 非
/// air，自然落入下面的"非 air"分支而不会误判为可落——M5 提示）。否则按固定
/// 邻格序【上、左、右、左上、右上】搜第一个 air 格降级；五邻格仍全占，则
/// 沿候选正上方继续逐格向上找空位，直到世界顶（Noita 同款方案：写回点被占
/// 时向上找空格，`docs/reference/noita-deep-dive.md:226`）。
///
/// **不再有"全占转悬浮"这条路**——那是原设计的活锁根源：悬浮粒子被重置到
/// 候选格中心后，下 tick DDA 因起点格豁免检查会在同一个已判定"全占"的候选
/// 格上原地复现同一局面，两 tick 死循环（Task 4 评审实测：40 颗同位同速
/// 粒子里 32 颗永久卡死，池不排空）。向上找到世界顶仍无 air，返回 `None`：
/// 调用方按出界处理（不写网格），计入 [`Particles::buried_total`] 诊断计数
/// ——`Outcome::Land` 现在必然终止于"落格"或"出界"，无第三态。
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
    // 五邻格全占：邻格序里 (cx, cy-1) 已经查过且被占，从再上一格（cy-2）起
    // 继续向上找，直到世界顶（`in_bounds` 为 false 时停止，None）。
    let mut y = cy - 2;
    while world.in_bounds(cx, y) {
        if world.cell(cx, y).material() == MAT_AIR {
            return Some((cx, y));
        }
        y -= 1;
    }
    None
}

/// 串行提交（spec §5 c/d）：按下标（= id）序应用 `outcomes`，返回保留掩码
/// （`false` = 已从池中移除——Land 落格写入网格 / Land 兜底搜索耗尽 / Gone
/// 出界，三者都不再是自由粒子；供 `compact` 使用）。拆成独立函数（而非内联
/// 进 [`advance`]）是为了让单测能直接注入合成 `Outcome`，验证冲突消解顺序
/// 而不必依赖真实重力积分的确切落格时机。
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
            Outcome::Fly { pos, vel } => {
                particles.set_state(i, pos.0, pos.1, vel.0, vel.1);
            }
            Outcome::Land { cx, cy, pos } => {
                // Land 必然终止于落格或出界（Task 4 修复轮 1 C1：悬浮路径已废除）。
                keep[i] = false;
                match resolve_landing(world, cx, cy) {
                    Some((lx, ly)) => {
                        world.set_cell_stamped(table, lx, ly, particles.material(i), stamp);
                        // **P→G 撞击动量传递**（Layer G Task 3，用户裁决 2026-08-31）。
                        //
                        // 在此之前，粒子哪怕以 MAX_SPEED（16 格/tick，网格上限的
                        // 4 倍）砸下来，落格时动量也被整个丢弃——网格 cell 跑到
                        // 4 格/tick 会溅射，粒子跑到 16 格/tick 反而不溅。那不是
                        // 裁决而是两个 Task 的时间差：本落格分支写于 M1，彼时网格
                        // 里还没有速度这个概念。
                        //
                        // 补法是**不新开通路**：把撞击速度量化写进 cell 的速度位，
                        // 下一 tick 网格 eval 看到一个满速 cell、立刻撞停，直接复用
                        // Task 3 已建好的整条溅射判定（三条件 + 本地限流 + 起始坐标
                        // 掷骰）。不新增生成源、不动 spec §6.4 的定序论证。
                        //
                        // 速度取本 tick 的**实际**位移量，而不是 `particles.vy(i)`
                        // ——后者是本 tick 重力积分**之前**的值，会少算一档重力，
                        // 在 SPLASH_MIN_SPEED 边界上足以翻转判定。`Outcome::Land`
                        // 的 `pos` 字段按其文档就是"未受阻时的积分终点 = 起点 +
                        // 本 tick 速度"，故差值即实际速度（见 `Outcome` 文档与
                        // `land_impact_velocity_matches_this_ticks_displacement`）。
                        let impact_vy = pos.1 - particles.y(i);
                        let v = cell::fx_to_vel(impact_vy);
                        // Gas 不写速度位（M2 Task 1）：气体规则不读速度位段，
                        // 写入只会在哈希里留一份永不消费的死重量。
                        if v != 0 && table.category(particles.material(i)) != Category::Gas {
                            world.set_cell_vel(lx, ly, v);
                        }
                    }
                    None => particles.mark_buried(),
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
    use crate::cell::V_MAX_CELL;
    use crate::material::{Category, MaterialDef};

    /// 落格撞击速度取法所依赖的契约（Layer G Task 3 的 P→G 通路）：
    /// `Outcome::Land.pos` 按其文档 = "未受阻时的积分终点" = 起点 + 本 tick
    /// 速度，故 `commit` 用 `pos − 起点` 取撞击速度，比读 `particles.vy(i)`
    /// （重力积分**之前**的值）少一档重力误差——在 `SPLASH_MIN_SPEED` 边界上
    /// 那一档足以翻转判定。
    ///
    /// 这条测试把那句文档变成可执行契约：谁把 `pos` 改成"实际落点"，这里立刻红。
    #[test]
    fn land_impact_velocity_matches_this_ticks_displacement() {
        let table = MaterialTable::new(vec![
            MaterialDef { blast_cost: 0, ..MaterialDef::base(0, "air", Category::Static, 0) },
            MaterialDef { blast_cost: crate::material::BLAST_COST_INFINITE, ..MaterialDef::base(1, "wall", Category::Static, 100) },
        ])
        .unwrap();
        let mut w = World::new(1, 1, 0);
        for x in 0..64 {
            w.set_cell_stamped(&table, x, 40, 1, 0); // 地板
        }
        let p = ParticleView { x: Fx::from_int(10), y: Fx::from_int(30), vx: Fx::ZERO, vy: Fx::from_int(12) };
        let Outcome::Land { pos, .. } = integrate(p, &w) else {
            panic!("12 格/tick 撞 10 格外的地板必须落格");
        };
        let (_, vy) = apply_gravity_and_clamp(p.vx, p.vy);
        assert_eq!(pos.1 - p.y, vy, "Land.pos 必须是起点 + 本 tick 速度（commit 据此取撞击速度）");
        assert_eq!(cell::fx_to_vel(pos.1 - p.y), V_MAX_CELL, "高速撞击必须量化到终端速度");
    }

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
        use crate::material::{Category, MaterialDef, BLAST_COST_INFINITE};
        let def = |id: u8, name: &str, category: Category, density: u16, blast_cost: u32| MaterialDef {
            blast_cost,
            ..MaterialDef::base(id, name, category, density)
        };
        MaterialTable::new(vec![
            def(0, "air", Category::Static, 0, 0),
            def(1, "wall", Category::Static, 100, BLAST_COST_INFINITE),
            def(2, "sand", Category::Powder, 40, 2),
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
    fn commit_conflict_all_five_neighbors_occupied_climbs_upward_to_find_air() {
        // C1 回归：候选格 + 全部 5 个邻格都占满，且正上方再高一格（5,3）也占满，
        // 迫使兜底搜索至少跨两步；(5,2) 留空，必须落在那里，不得转悬浮/活锁。
        let t = test_table();
        let mut w = World::new(1, 1, 0);
        for (dx, dy) in [(0, 0), (0, -1), (-1, 0), (1, 0), (-1, -1), (1, -1), (0, -2)] {
            w.set_cell_stamped(&t, 5 + dx, 5 + dy, 1, 0);
        }
        let mut p = spawn_n(1);
        let outcomes = vec![Outcome::Land { cx: 5, cy: 5, pos: (Fx::ZERO, Fx::ZERO) }];
        let keep = commit(&mut p, &mut w, &t, 0, &outcomes);
        assert_eq!(keep, vec![false], "找到空位应正常落格移除，不悬浮");
        assert_eq!(w.cell(5, 2).material(), 2, "应向上爬两格找到唯一的空位");
        assert_eq!(p.buried_total(), 0);
    }

    #[test]
    fn commit_conflict_fully_boxed_near_world_top_becomes_gone_and_counts_buried() {
        // C1 回归：候选格贴着世界顶（cy=1），5 邻格全占，兜底向上搜索一步
        // （y=cy-2=-1）就出界——必须判 Gone（移除、不写网格）且计入 buried_total，
        // 绝不允许悬浮/活锁。
        let t = test_table();
        let mut w = World::new(1, 1, 0);
        for (dx, dy) in [(0, 0), (0, -1), (-1, 0), (1, 0), (-1, -1), (1, -1)] {
            w.set_cell_stamped(&t, 5 + dx, 1 + dy, 1, 0);
        }
        let mut p = spawn_n(1);
        let outcomes = vec![Outcome::Land { cx: 5, cy: 1, pos: (Fx::ZERO, Fx::ZERO) }];
        let keep = commit(&mut p, &mut w, &t, 0, &outcomes);
        assert_eq!(keep, vec![false], "兜底搜索耗尽必须移除粒子（判 Gone），不得悬浮");
        assert_eq!(w.count_material(2), 0, "没有空位可写，不应污染网格");
        assert_eq!(p.buried_total(), 1, "必须计入诊断计数器");
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
    fn commit_fly_writes_position_and_velocity_verbatim_from_outcome() {
        // M2：commit 不再重算 apply_gravity_and_clamp——Outcome::Fly 携带的
        // vel 就是最终要写回的速度，原样落地即可（重力/clamp 只在 integrate
        // 内部算一次）。用一个刻意不满足"vy = 输入vy + GRAVITY"的 vel，证明
        // commit 确实是直接写回而非重新计算。
        let t = test_table();
        let mut w = World::new(1, 1, 0);
        let mut p = Particles::new();
        p.spawn(2, Fx::ZERO, Fx::ZERO, fx(9), fx(9)); // 初始速度对结果无影响
        let outcomes = vec![Outcome::Fly { pos: (fx(1), fx(2)), vel: (fx(-3), fx(7)) }];
        let keep = commit(&mut p, &mut w, &t, 0, &outcomes);
        assert_eq!(keep, vec![true]);
        assert_eq!(p.x(0), fx(1));
        assert_eq!(p.y(0), fx(2));
        assert_eq!(p.vx(0), fx(-3));
        assert_eq!(p.vy(0), fx(7));
    }

    #[test]
    fn integrate_applies_gravity_and_clamp_exactly_once_for_fly() {
        // M2 对应的 integrate 侧覆盖：重力+clamp 只在这里算一次，结果随
        // Outcome::Fly::vel 传出，commit 不再重算（见上一测试）。
        let w = World::new(1, 1, 0);
        let view = ParticleView { x: fx(2), y: fx(2), vx: fx(3), vy: fx(4) };
        match integrate(view, &w) {
            Outcome::Fly { vel, .. } => assert_eq!(vel, (fx(3), fx(4) + GRAVITY)),
            other => panic!("期望 Fly，实际 {other:?}"),
        }
    }

    #[test]
    fn integrate_clamps_fly_velocity_to_max_speed() {
        let w = World::new(1, 1, 0);
        let view = ParticleView { x: fx(2), y: fx(2), vx: MAX_SPEED + fx(5), vy: Fx::ZERO };
        match integrate(view, &w) {
            Outcome::Fly { vel, .. } => assert_eq!(vel.0, MAX_SPEED, "vx 必须被 clamp 到上限"),
            other => panic!("期望 Fly，实际 {other:?}"),
        }
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
