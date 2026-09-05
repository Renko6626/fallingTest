//! 弹体：SoA 表（spec §3.3、§5）。M4 Task 4 填字段与 `advance`——**只实现直线
//! 飞行 + DDA 命中判定 + `Bolt` 结算**；侵彻（能量消耗）、弹跳、阻力、穿透、
//! 排开液体、刚体冲量、定时爆全部是 Task 6 的地盘（spec §5.6 明列），本文件
//! 里凡是它们的插入点都留了注释锚点，不提前实现半成品。M4 Task 5 在 Task 4
//! 的骨架上追加 `Blast` 命中结算（走现有 `explode::apply_explode` +
//! `Bodies::pending_blasts`，零新增通路）；`advance` 因此从 Task 4 的窄签名
//! （`world: &World`、无 `bodies`/`stamp`/`fseed`/`spawns`）**按需**放宽到
//! `world: &mut World` + 四个新形参——Task 4 头注早已预告"各自在真正长出
//! 用例的那个 Task 里加"，这里就是那个 Task。
//!
//! **体例完全照抄 `particle.rs`**：SoA、下标即 id、`compact` 走 `retain`
//! 语义的保序压缩、容量拒绝计数器**不入哈希**只供诊断。**与 `Particles` 刻意
//! 不共用一个池**——弹体是事件载体（命中即结算、有寿命与归属），粒子是材质
//! 搬运器（落格即变 cell、无 lifetime），语义不同不能塞进同一张 SoA
//! （`docs/superpowers/specs/2026-09-05-m4-player-and-spells-design.md` §1.2
//! 的措辞澄清）。复用的是 `dda.rs`/`fixed.rs` 两个模块，不是 `Particles` 本体。
//!
//! **死亡标记不额外开列**：一律 `life[i] = 0`，`compact()` 只看这一列。命中
//! 而死与寿命耗尽走同一条出口，少一列进哈希。

use xxhash_rust::xxh3::Xxh3;

use crate::body::Bodies;
use crate::creature::Creatures;
use crate::dda;
use crate::explode;
use crate::fixed::{isqrt, Fx};
use crate::material::{self, MaterialTable};
use crate::spell::{SpellDef, SpellKind, SpellTable, BLAST_OP_IDX_BASE};
use crate::world::{SpawnRequest, World};

/// 弹体池容量上限（Global Constraints 表：`MAX_PROJECTILES = 4096`）。超限
/// `Projectiles::spawn` 确定性拒绝、不排队（同粒子池口径）。
pub const MAX_PROJECTILES: usize = 4096;

/// 弹体池：SoA 布局，下标即 id 序（`particle.rs::Particles` 同体例）。
#[derive(Clone, Debug, Default)]
pub struct Projectiles {
    x: Vec<Fx>,
    y: Vec<Fx>,
    vx: Vec<Fx>,
    vy: Vec<Fx>,
    /// 指回 `SpellTable`。
    spell: Vec<u8>,
    /// 剩余帧数；0 = 死亡（`compact` 据此判定去留，见文件头注）。
    life: Vec<u16>,
    /// 剩余侵彻能量池（spec §5.2）；Task 4 不消费，仅入哈希占位，Task 6 起
    /// `resolve_hit_cell` 的能量结算才会读写它。
    energy: Vec<u32>,
    /// 发射者 creature id；`255` = 无归属（测试注入弹体，`first_hit_at` 对
    /// 越界 id 的查询天然落空，不特判）。
    owner: Vec<u8>,
    /// 防自伤宽限剩余帧（spec §5.3）：`> 0` 时 `owner` 自身不算命中。
    grace: Vec<u8>,
    /// 剩余弹跳次数（spec §5.4）；Task 4 不消费，仅入哈希占位。
    bounces: Vec<u8>,
    /// 容量拒绝次数：诊断用，**不入哈希**（`particle.rs::rejected_total` 同一口径）。
    rejected_total: u64,
}

impl Projectiles {
    pub fn new() -> Projectiles {
        Projectiles::default()
    }

    pub fn len(&self) -> usize {
        self.x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }

    /// 按下标（id 序）只读访问单个弹体字段（`particle.rs::Particles::x` 同体例）。
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
    pub fn spell(&self, i: usize) -> u8 {
        self.spell[i]
    }
    pub fn life(&self, i: usize) -> u16 {
        self.life[i]
    }
    pub fn energy(&self, i: usize) -> u32 {
        self.energy[i]
    }
    pub fn owner(&self, i: usize) -> u8 {
        self.owner[i]
    }
    pub fn bounces(&self, i: usize) -> u8 {
        self.bounces[i]
    }

    /// 容量拒绝诊断计数（不入哈希）。
    pub fn rejected_total(&self) -> u64 {
        self.rejected_total
    }

    /// 追加一个弹体；容量满时确定性拒绝（丢弃 + 计数，返回 `false`）。成功
    /// 追加的弹体下标 = 追加前的 `len()`，即 id 序中的位置——`Sim::queue_projectile`
    /// 与 Task 5 的施法结算走同一个入口，测试跑的就是产品代码。
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        &mut self,
        spell: u8,
        x: Fx,
        y: Fx,
        vx: Fx,
        vy: Fx,
        life: u16,
        energy: u32,
        owner: u8,
        grace: u8,
        bounces: u8,
    ) -> bool {
        if self.len() >= MAX_PROJECTILES {
            self.rejected_total += 1;
            return false;
        }
        self.x.push(x);
        self.y.push(y);
        self.vx.push(vx);
        self.vy.push(vy);
        self.spell.push(spell);
        self.life.push(life);
        self.energy.push(energy);
        self.owner.push(owner);
        self.grace.push(grace);
        self.bounces.push(bounces);
        true
    }

    /// 保序压缩：按下标序移除 `life[i] == 0` 的弹体，其余保留、相对顺序不变
    /// （`retain` 语义，`particle.rs::Particles::compact` 同形，唯一区别是
    /// 去留判据直接读内部 `life` 列而非外部传入的 `keep` 掩码——弹体的死亡
    /// 判据本就是"life 归零"这一件事，不需要额外一层间接）。
    fn compact(&mut self) {
        let mut w = 0usize;
        for r in 0..self.life.len() {
            if self.life[r] > 0 {
                if w != r {
                    self.x[w] = self.x[r];
                    self.y[w] = self.y[r];
                    self.vx[w] = self.vx[r];
                    self.vy[w] = self.vy[r];
                    self.spell[w] = self.spell[r];
                    self.life[w] = self.life[r];
                    self.energy[w] = self.energy[r];
                    self.owner[w] = self.owner[r];
                    self.grace[w] = self.grace[r];
                    self.bounces[w] = self.bounces[r];
                }
                w += 1;
            }
        }
        self.x.truncate(w);
        self.y.truncate(w);
        self.vx.truncate(w);
        self.vy.truncate(w);
        self.spell.truncate(w);
        self.life.truncate(w);
        self.energy.truncate(w);
        self.owner.truncate(w);
        self.grace.truncate(w);
        self.bounces.truncate(w);
    }

    /// 实体层哈希的弹体部分（R1 裁决）：空池恒返回 0（早退，不跑空 fold——
    /// 每一份既有 golden 都靠这条：目前没有任何场景生成弹体）；非空按下标序
    /// （= id 序）折叠全部列，含 `energy`/`grace`/`bounces`（即便 Task 4
    /// 尚不消费它们，哈希结构也一次到位，避免 Task 6 再变一次）。
    pub fn hash_into(&self) -> u64 {
        if self.is_empty() {
            return 0;
        }
        let mut h = Xxh3::new();
        for i in 0..self.len() {
            h.update(&self.x[i].0.to_le_bytes());
            h.update(&self.y[i].0.to_le_bytes());
            h.update(&self.vx[i].0.to_le_bytes());
            h.update(&self.vy[i].0.to_le_bytes());
            h.update(&[self.spell[i]]);
            h.update(&self.life[i].to_le_bytes());
            h.update(&self.energy[i].to_le_bytes());
            h.update(&[self.owner[i]]);
            h.update(&[self.grace[i]]);
            h.update(&[self.bounces[i]]);
        }
        h.update(&(self.len() as u64).to_le_bytes());
        h.digest()
    }

    /// 弹体相主入口（架构 §4 第 2c 步，spec §5.1）：按下标序（= id 序，
    /// 架构 §7.1 定序铁律）积分 + DDA 命中判定。
    ///
    /// **签名相对 Task 4 放宽**（Task 4 头注"各自在真正长出用例的那个 Task
    /// 里加"兑现的第一步）：`world` 从 `&World` 改回 `&mut World`，新增
    /// `bodies`/`stamp`/`fseed`/`spawns` 四个参数——`Blast` 命中结算需要
    /// 删格（`world`）、判材质（`table` 不变）、推刚体（`bodies`）、走
    /// `apply_explode` 的抖动骰（`stamp`/`fseed`）、把溅射粒子交回队列
    /// （`spawns`）。`Bolt` 分支不读这四个新参数，但函数整体（因 `Blast`
    /// 分支的存在）确实用到了它们，不算未用形参。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn advance(
        &mut self,
        world: &mut World,
        table: &MaterialTable,
        spells: &SpellTable,
        creatures: &mut Creatures,
        bodies: &mut Bodies,
        stamp: u8,
        fseed: u32,
        spawns: &mut Vec<SpawnRequest>,
    ) {
        for i in 0..self.x.len() {
            let s = spells.get(self.spell[i]);
            self.vy[i] = self.vy[i] + s.gravity;
            // Task 6 在此插入 air_friction / liquid_drag（spec §5.1）。

            let pos = (self.x[i], self.y[i]);
            let vel = (self.vx[i], self.vy[i]);
            let mut alive = true;
            // 先到者优先：沿路径逐格走，生物与硬格在同一次遍历里按遇到的先后
            // 判定，不是"先测完全部生物再测格子"（spec §5.1"沿 dda::cell_walk
            // 逐格推进，按'先到者优先'判定"）。
            for (gx, gy) in dda::CellWalk::new(pos, vel) {
                if !world.in_bounds(gx, gy) {
                    alive = false; // 出界即销毁（不算阻挡，dda.rs 同一口径）。
                    break;
                }
                let owner = self.owner[i];
                // owner 越界（255 = 无归属的测试注入弹体）时 `get` 天然返回
                // `None`：用一个不等于任何真实 team 的哨兵顶上（同 `owner`/
                // `controller` 字段的 255 = 无归属惯例），`first_hit_at`
                // 因此不会误判"无主弹体"与某个 team 0 的生物同队。
                let owner_team = creatures.get(owner).map(|c| c.team).unwrap_or(255);
                if let Some(cid) = creatures.first_hit_at(gx, gy, owner, self.grace[i], owner_team) {
                    resolve_hit(s, vel, Some(cid), gx, gy, world, table, creatures, bodies, stamp, fseed, i, spawns);
                    alive = false;
                    break;
                }
                if material::is_solid(world.cell(gx, gy), table, true) {
                    // 命中硬格：Bolt 无侵彻判定（能量/门槛免疫留 Task 6 的
                    // §5.2），直接消失、不删格；Blast 在此触发爆炸——见
                    // `resolve_hit`。
                    resolve_hit(s, vel, None, gx, gy, world, table, creatures, bodies, stamp, fseed, i, spawns);
                    alive = false;
                    break;
                }
            }

            if alive {
                // 路径走完无命中：整个速度原样落地，寿命 -1（Task 6 在归零处
                // 加"寿命耗尽即爆炸"分支）。**每 tick 恰好积分一次**——DDA
                // 只用来做逐格判定，不改变本 tick 实际位移量
                // （`projectile_moves_exactly_once_per_tick` 钉死）。
                self.x[i] = self.x[i] + self.vx[i];
                self.y[i] = self.y[i] + self.vy[i];
                self.life[i] = self.life[i].saturating_sub(1);
            } else {
                self.life[i] = 0; // 死亡标记 = life 归零（文件头注）。
            }
            self.grace[i] = self.grace[i].saturating_sub(1);
        }
        self.compact();
    }
}

/// 命中结算（spec §5.3）：`cid = Some(id)` 是命中生物，`cid = None` 是命中
/// 硬格；`(gx, gy)` 是 DDA 撞停的那一格，两种命中共用同一坐标——`Blast` 的
/// 爆心恒取这个坐标，无论撞到的是生物还是墙。
///
/// - `Bolt`：只在命中生物时扣血 + 击退；命中硬格什么都不做（本 Task 无
///   侵彻，Task 6 地盘）。
/// - `Blast`：命中生物与命中硬格走**完全相同**的一支——都在 `(gx, gy)`
///   触发一次现有 `explode::apply_explode` + 追加 `bodies.pending_blasts`
///   （spec §5.3"命中生物：同上 + 触发爆炸"，`SpellKind::Blast` 文档已
///   澄清"同上"不含独立的直接扣血/击退，伤害完全来自爆炸本身）。
/// - `Spray`：`Projectiles` 从不为它 spawn 弹体（`spell::cast_all` 直接
///   emit），这条分支在产品路径上不可达。
#[allow(clippy::too_many_arguments)]
fn resolve_hit(
    s: &SpellDef,
    vel: (Fx, Fx),
    cid: Option<u8>,
    gx: i32,
    gy: i32,
    world: &mut World,
    table: &MaterialTable,
    creatures: &mut Creatures,
    bodies: &mut Bodies,
    stamp: u8,
    fseed: u32,
    op_idx: usize,
    spawns: &mut Vec<SpawnRequest>,
) {
    match s.kind {
        SpellKind::Bolt { damage_milli, knockback } => {
            if let Some(cid) = cid {
                let dir = normalize(vel.0, vel.1);
                creatures.apply_hit(cid, damage_milli, knockback, dir);
            }
        }
        SpellKind::Blast { power, radius, max_durability } => {
            explode::apply_explode(
                world,
                table,
                gx,
                gy,
                radius,
                power,
                max_durability,
                stamp,
                fseed,
                BLAST_OP_IDX_BASE + op_idx,
                spawns,
            );
            bodies.pending_blasts.push((gx, gy, radius));
        }
        SpellKind::Spray { .. } => {
            // 真正不可达：`cast_all` 直接走 emit 通路从不 spawn 弹体，唯一
            // 的外部入口 `Sim::queue_projectile` 也在**入口**显式拒绝
            // `SpellKind::Spray`（M4 Task 5 评审 Important，2026-09-06——
            // 不变量守在入口而非在此远端兜底，理由见该方法文档）。
            unreachable!("Spray 不产生弹体：queue_projectile 入口已拒绝、cast_all 直接走 emit 通路")
        }
    }
}

/// 弹体命中生物的击退方向：把本 tick 弹体速度 `(vx, vy)` 归一化成单位向量
/// （评审遗留项收紧，Task 4 头注承诺"真正跑到斜向弹道场景时再按需收紧"，
/// Task 5 的施法出射方向查 BAM 表正是那个场景）。用 `isqrt` 定点归一——
/// `explode.rs::fire_ray` 对爆炸射线方向做的是同一件事，这里原样复用同一套
/// 数学，不引入新的近似，核心也不因此多一处超越函数依赖。
///
/// `mag == 0`（零速命中：生产路径弹体初速来自 `spell.speed`，恒 > 0，这里
/// 纯属防御）返回零向量，同 `fire_ray` 的处理。
fn normalize(vx: Fx, vy: Fx) -> (Fx, Fx) {
    let mag_sq = (vx.0 as i64) * (vx.0 as i64) + (vy.0 as i64) * (vy.0 as i64);
    let mag = isqrt(mag_sq as u64) as i32;
    if mag == 0 {
        (Fx::ZERO, Fx::ZERO)
    } else {
        (Fx::from_ratio(vx.0, mag), Fx::from_ratio(vy.0, mag))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_projectiles_hash_into_is_zero() {
        assert_eq!(Projectiles::new().hash_into(), 0);
    }

    #[test]
    fn spawn_rejects_beyond_capacity_and_counts_diagnostically() {
        let mut p = Projectiles::new();
        for _ in 0..MAX_PROJECTILES {
            assert!(p.spawn(0, Fx::ZERO, Fx::ZERO, Fx::ZERO, Fx::ZERO, 10, 0, 255, 0, 0));
        }
        assert!(!p.spawn(0, Fx::ZERO, Fx::ZERO, Fx::ZERO, Fx::ZERO, 10, 0, 255, 0, 0));
        assert_eq!(p.len(), MAX_PROJECTILES);
        assert_eq!(p.rejected_total(), 1);
    }

    #[test]
    fn hash_into_is_stable_and_sensitive_to_content() {
        let mut a = Projectiles::new();
        a.spawn(1, Fx::from_int(3), Fx::from_int(4), Fx::from_int(1), Fx::ZERO, 10, 0, 255, 2, 0);
        let mut b = Projectiles::new();
        b.spawn(1, Fx::from_int(3), Fx::from_int(4), Fx::from_int(1), Fx::ZERO, 10, 0, 255, 2, 0);
        assert_eq!(a.hash_into(), b.hash_into(), "同内容必须同哈希");
        b.spawn(1, Fx::from_int(9), Fx::from_int(9), Fx::ZERO, Fx::ZERO, 5, 0, 255, 0, 0);
        assert_ne!(a.hash_into(), b.hash_into(), "内容差异必须反映进哈希");
    }
}
