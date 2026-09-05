//! 弹体：SoA 表（spec §3.3、§5）。M4 Task 4 填字段与 `advance`——**只实现直线
//! 飞行 + DDA 命中判定 + `Bolt` 结算**；M4 Task 5 在此基础上追加 `Blast`
//! 命中结算（走现有 `explode::apply_explode` + `Bodies::pending_blasts`，
//! 零新增通路）。**M4 Task 6 补齐七项扩展**（spec §5.1/§5.2/§5.4/§5.5，逐条
//! TDD、任务书 cheapest-first 排序）：`displace_liquid` 排开液体/粉末、
//! `pass_through` 穿透掩码、`air_friction`/`liquid_drag` 阻力、
//! `on_lifetime_out_explode` 定时爆、`dig_power`+`max_durability` 侵彻、
//! `bounces`+`bounce_energy` 弹跳、`physics_impulse` 刚体单点冲量——`advance`
//! 因此再放宽一个形参 `phys: &mut PhysicsWorld`（单点冲量要跨过 grid↔physics
//! 边界，`body.rs` 是架构 §5 唯一允许同时接触两者的模块，`advance` 只是把
//! 引用转手给它，自己不碰任何 physics 类型）。
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
use crate::fixed::{isqrt, Fx, HALF_CELL};
use crate::material::{self, Category, MaterialTable};
use crate::physics::PhysicsWorld;
use crate::spell::{SpellDef, SpellKind, SpellTable, BLAST_OP_IDX_BASE};
use crate::world::{SpawnRequest, World};

/// 弹体池容量上限（Global Constraints 表：`MAX_PROJECTILES = 4096`）。超限
/// `Projectiles::spawn` 确定性拒绝、不排队（同粒子池口径）。
pub const MAX_PROJECTILES: usize = 4096;

/// 侵彻删格（`destroy_cell`）的 RNG 盐值起点（M4 Task 6，spec §5.2）：复用
/// `explode.rs::STREAM_EXPLODE` 同一条抖动流（同款"摧毁一格→溅射一份碎屑"
/// 语义），但盐值区间必须与该流已占用的两段不相交——`spell.rs::
/// BLAST_OP_IDX_BASE` 覆盖 `[1<<20, 1<<20 + MAX_PROJECTILES)`，ops 阶段
/// `Op::Explode` 的 `op_idx`（`enumerate()` 下标）远不到 `1<<16`。取
/// `1<<21`：比 `BLAST_OP_IDX_BASE` 的上界（`1<<20 + 4096`）还高出一个数量级，
/// `DIG_OP_IDX_BASE + i`（`i` = 弹体在 SoA 里的下标，本 tick `advance()`
/// 循环内唯一）覆盖 `[1<<21, 1<<21 + MAX_PROJECTILES)`，与另外两段互不相交
/// （`dig_op_idx_base_does_not_collide_with_blast_or_ops_phase` 单测钉死）。
/// 同一颗弹一 tick 可能侵彻多格，但每格的 `(gx, gy)` 本就互不相同（`rng_u32`
/// 的 key 含坐标），salt 只需区分"这是哪颗弹"，不需要再区分"第几格"。
const DIG_OP_IDX_BASE: u32 = 1 << 21;
/// 侵彻删格的 vx/vy 抖动骰子标号（同 `explode.rs::EXPLODE_ROLL_VX`/
/// `_VY` 的编码用途；取值巧合相同不代表可以共用常量——两个调用点各自独立
/// 演化，改一个不该牵连另一个）。
const DIG_ROLL_VX: u32 = 0;
const DIG_ROLL_VY: u32 = 1;

/// 本 tick 弹跳最多重开几次 `dda::CellWalk`（M4 Task 6 spec §5.4，有界重启
/// 纪律）：防止弹体卡在墙角内反复反弹，吃掉无界的单 tick 计算量。正常数值
/// 下（`data/spells.ron` 里 `bounces` 全部 ≤ 2）远用不到这个上限，它是安全网
/// 而非常规路径；撞上限时**当 tick 作废、不欠账到下一 tick**（同 M1 溅射
/// 限流"超限不排队"一个纪律——排队需要跨 tick 状态，会把限流变成状态机，
/// CLAUDE.md §8 反例表已点名过这个坑）：撞满 4 次仍在弹的弹体，第 5 次直接
/// 走终结分支（`resolve_hit`），不管 `bounces` 是否还有余量。
pub(crate) const MAX_BOUNCE_RESTARTS: usize = 4;

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
    /// 架构 §7.1 定序铁律）积分 + DDA 命中判定 + 七项扩展（M4 Task 6）。
    ///
    /// **签名相对 Task 5 再放宽一个形参**（Task 4 头注"各自在真正长出用例的
    /// 那个 Task 里加"兑现的最后一步）：新增 `phys: &mut PhysicsWorld`——
    /// `physics_impulse`（§5.5）命中刚体盖章格时要给它一次单点冲量，这个
    /// 调用必须经 `Bodies` 转手（架构 §5：`body.rs` 是唯一同时接触 grid 与
    /// physics 的模块），`advance` 自己不碰任何 physics 类型，只是把引用
    /// 转手给 `bodies.apply_projectile_impulse`。
    ///
    /// # 七项扩展的判定顺序（本 Task 的显式设计决策，不只是口头约定）
    ///
    /// 每 tick 开头：`vy += gravity` → `(vx,vy) *= air_friction` → 若**本 tick
    /// 起点格**是 Liquid 再 `*= liquid_drag`（只采样起点，不沿途逐格重采，
    /// 见下方"液体阻力采样口径"）。
    ///
    /// 沿 DDA 路径逐格判定，每格：
    /// 1. 出界 → 死。
    /// 2. 命中生物（先到者优先，未变）→ `resolve_hit` → 死。
    /// 3. 该格 `Category` 在 `pass_through` 掩码内 → 直接穿过，**不算命中**，
    ///    也不触发排开——`pass_through` 优先于 `displace_liquid`（brief Step2
    ///    原话："穿过去就不推开"，两者互斥、`pass_through` 赢）。
    /// 4. 否则若 `displace_liquid` 且该格是 Liquid/Powder → 排开成粒子，
    ///    继续飞（与 §4.3 生物排开同一条脱格通路）。
    /// 5. 否则若该格"挡弹体"（[`blocks_projectile`]——**不是** `material::
    ///    is_solid`，见该函数文档）：先试侵彻——门槛（`durability`）与能量
    ///    （`dig_power` 预算）都够，扣 energy、删格（`explode::destroy_cell`），
    ///    继续飞。侵彻失败（门槛免疫或能量不足）才是一次真正的"碰撞事件"：
    ///    若命中格属于某个刚体盖章格且 `physics_impulse != 0`，先给它一次
    ///    单点冲量（与侵彻成功与否正交，§5.5 独立生效，不因为"这一下没能
    ///    穿透"就不给）；再看弹跳——`bounces > 0` 且本 tick 重开次数未达
    ///    `MAX_BOUNCE_RESTARTS` 就反射对应轴速度、用新速度重开一次
    ///    `CellWalk`；否则终结（`resolve_hit`）。
    ///
    /// 路径走完（含全部弹跳重开都未再命中）→ 位置/速度落地，`life -= 1`；
    /// `life == 0` 且 `on_lifetime_out_explode` → 在最终落点补一次
    /// `resolve_hit(None)`（Bolt 天然 no-op，只有 Blast 真的炸）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn advance(
        &mut self,
        world: &mut World,
        table: &MaterialTable,
        spells: &SpellTable,
        creatures: &mut Creatures,
        bodies: &mut Bodies,
        phys: &mut PhysicsWorld,
        stamp: u8,
        fseed: u32,
        spawns: &mut Vec<SpawnRequest>,
    ) {
        for i in 0..self.x.len() {
            let s = spells.get(self.spell[i]);
            self.vy[i] = self.vy[i] + s.gravity;
            self.vx[i] = self.vx[i].mul(s.air_friction);
            self.vy[i] = self.vy[i].mul(s.air_friction);

            // 液体阻力采样口径（本 Task 的显式决策）：只看**本 tick 起点格**
            // 一次，不是沿途逐格重采——① 与 spec §5.1 伪代码逐字对应
            // （"若起点在液体格内"，单数"起点"不是"沿途每格"）；② 沿途重采
            // 需要在 DDA 循环里为每个穿越格再查一次 `table.category`、且要
            // 处理"半路探入/探出水面"的加权，复杂度换不来可观测的手感差异
            // ——`liquid_drag_slows_a_projectile_inside_water_more_than_in_
            // air` 只要求"泡在水里比空气里飞得近"，起点采样一次已经足够
            // 制造这个差异（只要本 tick 仍在水里就会重新触发一次衰减，效果
            // 随 tick 数累积）。
            let start_cell = (self.x[i].to_cell(), self.y[i].to_cell());
            if world.in_bounds(start_cell.0, start_cell.1) {
                let start_m = world.cell(start_cell.0, start_cell.1).material();
                if table.category(start_m) == Category::Liquid {
                    self.vx[i] = self.vx[i].mul(s.liquid_drag);
                    self.vy[i] = self.vy[i].mul(s.liquid_drag);
                }
            }

            let mut cur_pos = (self.x[i], self.y[i]);
            let mut cur_vel = (self.vx[i], self.vy[i]);
            let mut restarts = 0usize;
            let mut alive = true;

            // 弹跳重开的位移口径（本 Task 的显式决策，写在这里而非只在
            // commit message 里）：每次重开都用反射后的新速度，从"撞击格
            // 前一格"的格心位置，重新走一次**完整** `CellWalk`——**不**按
            // "已走比例"折算剩余位移。brief 原文本身也承认"剩余量=原位移-
            // 已走部分"这套精确口径要走一条除法链，转而认可"本 tick 最多
            // 重开 `MAX_BOUNCE_RESTARTS` 次、每次消耗剩余步数"的简化版——
            // 本实现采用其中最直接的一种读法：重开即满速重开，不折算。副作用
            // 是弹跳当 tick 的总位移可能略超过"一个 tick 的原始位移量"（多
            // 弹一次就多算一段 `|cur_vel|` 的路），但 `MAX_BOUNCE_RESTARTS`
            // 把这个近似的代价封顶在"至多 4 段"，且 `bounces` 字段本身通常
            // ≤ 2（`data/spells.ron`），实际游戏里这条近似几乎不可观测；换
            // 一条除法链换来的精确度不值得为它多背一条运行时除法（核心红线：
            // 一切算术走 wrapping_*、不引入非常量除法）。
            'walk: loop {
                let mut last_clear = (cur_pos.0.to_cell(), cur_pos.1.to_cell());
                let mut walker = dda::CellWalk::new(cur_pos, cur_vel);
                let mut bounced = false;

                while let Some((gx, gy)) = walker.next() {
                    if !world.in_bounds(gx, gy) {
                        alive = false; // 出界即销毁（不算阻挡，dda.rs 同一口径）。
                        break 'walk;
                    }

                    // ② 命中生物：先到者优先（未变，仍是每格最先判定的一项）。
                    let owner = self.owner[i];
                    let owner_team = creatures.get(owner).map(|c| c.team).unwrap_or(255);
                    if let Some(cid) = creatures.first_hit_at(gx, gy, owner, self.grace[i], owner_team) {
                        resolve_hit(s, cur_vel, Some(cid), gx, gy, world, table, creatures, bodies, stamp, fseed, i, spawns);
                        alive = false;
                        break 'walk;
                    }

                    let cell = world.cell(gx, gy);
                    let m = cell.material();
                    let cat = table.category(m);

                    // ③ pass_through：不算命中，直接穿过。
                    if cat.bit() & s.pass_through != 0 {
                        last_clear = (gx, gy);
                        continue;
                    }
                    // ④ 排开液体/粉末（§5.5，与 §4.3 生物排开同一条脱格
                    // 通路）：速度取**当前**弹体速度（`cur_vel`，可能已被
                    // 摩擦/阻力/更早的弹跳改过），不是弹体出生时的初速。
                    if s.displace_liquid && matches!(cat, Category::Liquid | Category::Powder) {
                        world.set_cell_stamped(table, gx, gy, material::MAT_AIR, stamp);
                        spawns.push(SpawnRequest {
                            material: m,
                            x: Fx::from_int(gx) + HALF_CELL,
                            y: Fx::from_int(gy) + HALF_CELL,
                            vx: cur_vel.0,
                            vy: cur_vel.1,
                        });
                        last_clear = (gx, gy);
                        continue;
                    }

                    // ⑤ 挡弹体：侵彻 → 冲量 → 弹跳 → 终结（各自独立生效，
                    // 顺序见函数文档"七项扩展的判定顺序"一节）。
                    if blocks_projectile(m, table) {
                        if table.durability(m) <= s.max_durability && self.energy[i] > 0 {
                            let cost = table.hp(m);
                            if (self.energy[i] as u64) >= cost as u64 {
                                self.energy[i] -= cost;
                                let dir = normalize(cur_vel.0, cur_vel.1);
                                explode::destroy_cell(
                                    world,
                                    table,
                                    gx,
                                    gy,
                                    m,
                                    stamp,
                                    fseed,
                                    dir,
                                    self.energy[i],
                                    s.dig_power,
                                    DIG_OP_IDX_BASE + i as u32,
                                    DIG_ROLL_VX,
                                    DIG_ROLL_VY,
                                    spawns,
                                );
                                last_clear = (gx, gy);
                                continue;
                            }
                        }

                        // 侵彻失败（门槛免疫或能量不足）：真正的碰撞事件——
                        // 命中刚体盖章格就给一次单点冲量，与是否接下来弹跳/
                        // 终结正交（§5.5 独立生效）。
                        if cell.is_body() && s.physics_impulse != 0 {
                            bodies.apply_projectile_impulse(phys, gx, gy, s.physics_impulse, cur_vel);
                        }

                        if self.bounces[i] > 0 && restarts < MAX_BOUNCE_RESTARTS {
                            let axis = walker.last_axis();
                            cur_vel = match axis {
                                dda::Axis::X => (-cur_vel.0.mul(s.bounce_energy), cur_vel.1),
                                dda::Axis::Y => (cur_vel.0, -cur_vel.1.mul(s.bounce_energy)),
                            };
                            self.bounces[i] -= 1;
                            restarts += 1;
                            cur_pos =
                                (Fx::from_int(last_clear.0) + HALF_CELL, Fx::from_int(last_clear.1) + HALF_CELL);
                            bounced = true;
                            break; // 跳出内层 while，外层 'walk 用新 cur_pos/cur_vel 重开一段。
                        }

                        resolve_hit(s, cur_vel, None, gx, gy, world, table, creatures, bodies, stamp, fseed, i, spawns);
                        alive = false;
                        break 'walk;
                    }

                    last_clear = (gx, gy);
                }

                if !bounced {
                    break 'walk; // 本段 CellWalk 走完全程无命中：落地。
                }
            }

            if alive {
                // 全程（含全部弹跳重开）无命中：位置 = 最后一段起点 +
                // 该段速度，速度就地落地为 `cur_vel`（可能已被弹跳改过）。
                // **每 tick 恰好积分一次**这条纪律对"最后一段"仍然成立
                // （`projectile_moves_exactly_once_per_tick` 覆盖的是无弹跳
                // 的直线情形，未变）。
                self.x[i] = cur_pos.0 + cur_vel.0;
                self.y[i] = cur_pos.1 + cur_vel.1;
                self.vx[i] = cur_vel.0;
                self.vy[i] = cur_vel.1;
                self.life[i] = self.life[i].saturating_sub(1);
                if self.life[i] == 0 && s.on_lifetime_out_explode {
                    // §5.1"寿命耗尽即销毁（若 on_lifetime_out_explode 则先
                    // 炸）"：复用 `resolve_hit` 的 `Blast` 分支，在最终落点
                    // （不是命中格——这里压根没命中）触发；`cid = None` 与
                    // 命中硬格同一坐标口径，`Bolt` 分支的 `cid = None` 天然
                    // no-op，不需要额外按 `kind` 分支。
                    let (fx_, fy_) = (self.x[i].to_cell(), self.y[i].to_cell());
                    resolve_hit(s, cur_vel, None, fx_, fy_, world, table, creatures, bodies, stamp, fseed, i, spawns);
                }
            } else {
                self.life[i] = 0; // 死亡标记 = life 归零（文件头注）。
            }
            self.grace[i] = self.grace[i].saturating_sub(1);
        }
        self.compact();
    }
}

/// 弹体撞硬格判定：**不复用 `material::is_solid`**——那个谓词是为生物/刚体
/// 的地形碰撞写的，特意把 `Liquid`/`Gas` 排除在外（水面不挡脚、烟雾不挡人），
/// 但弹体对液体/气体的默认态度与地形碰撞相反：§5.1/§5.2 的"沿路径逐格推进、
/// 第一个挡路的格即命中"没有天然豁免液体/气体，`pass_through` 掩码才是唯一
/// 的豁免出口。这不是纸面推论——`wet_bolt` 的存在本身就是这条论证的执法
/// 现场：`liquid_drag_slows_a_projectile_inside_water_more_than_in_air` 的
/// 测试注释写得很直白："必须穿液体，否则它会在入水第一格就命中而不是被
/// 减速"，`data/spells.ron` 里**每一条**现有法术都显式给了 `pass_through:
/// ["gas", ...]`——如果 Gas/Liquid 像 `is_solid` 那样天生免疫，这个字段
/// 对它们就是彻底的死配置，没有任何法术需要费这个心。
///
/// `body_passable` 仍然复用（同 `is_solid` 的既有语义：显式标记"刚体可穿过"
/// 的软材质，弹体同样穿过，不因为整体判定换了谓词就多绕一层特例）。
fn blocks_projectile(m: u8, table: &MaterialTable) -> bool {
    m != material::MAT_AIR && !table.body_passable(m)
}

/// 命中结算（spec §5.3）：`cid = Some(id)` 是命中生物，`cid = None` 是命中
/// 硬格；`(gx, gy)` 是 DDA 撞停的那一格，两种命中共用同一坐标——`Blast` 的
/// 爆心恒取这个坐标，无论撞到的是生物还是墙。
///
/// - `Bolt`：只在命中生物时扣血 + 击退；命中硬格什么都不做——侵彻判定
///   （M4 Task 6 spec §5.2）发生在 `advance` 循环内、走到这里之前：能钻的
///   格早被 `explode::destroy_cell` 删掉、`continue` 到下一格，只有侵彻
///   失败（门槛免疫或能量不足）才会落到这里，此时 `Bolt` 就是单纯消失。
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

    /// `DIG_OP_IDX_BASE` 文档承诺的执法测试：与 `BLAST_OP_IDX_BASE`
    /// （`[1<<20, 1<<20 + MAX_PROJECTILES)`）和 ops 阶段（`op_idx` 远不到
    /// `1<<16`）两段盐值区间都不相交——`spell.rs::
    /// blast_op_idx_base_does_not_collide_with_ops_phase` 同一体例。
    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn dig_op_idx_base_does_not_collide_with_blast_or_ops_phase() {
        const REASONABLE_MAX_OPS_PER_TICK: u32 = 1 << 16; // 远超任何真实场景的单 tick op 数
        assert!(DIG_OP_IDX_BASE > REASONABLE_MAX_OPS_PER_TICK, "Dig 基址必须远高于 ops 阶段 op_idx 上界");
        let blast_upper = BLAST_OP_IDX_BASE as u32 + MAX_PROJECTILES as u32; // BLAST 段的独占上界
        assert!(DIG_OP_IDX_BASE >= blast_upper, "Dig 基址不得落进 BLAST_OP_IDX_BASE 的占用区间");
        let dig_upper = DIG_OP_IDX_BASE as u64 + MAX_PROJECTILES as u64; // Dig 段的独占上界
        assert!(dig_upper <= u32::MAX as u64, "Dig 段上界不得溢出 u32");
    }

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
