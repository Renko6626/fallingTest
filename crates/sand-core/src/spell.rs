//! 法术：表 + loadout + 施法闸门（spec §3.4、§6）。M4 Task 4 建**核心类型本体**
//! ——`SpellKind`/`SpellDef`/`SpellTable`——但只填 `Bolt` 一个变体；`Blast`/
//! `Spray` 两个变体、`data/spells.ron` 加载器、cooldown+mana 双闸门是本 Task
//! （5）补齐的：三原语派发（`cast_all`）+ 加载器（`sand-harness::scenario::
//! load_spells`）。侵彻/弹跳/阻力等七项扩展字段的**消费**仍留 Task 6（字段
//! 本体本 Task 已按 Task 6 的中性缺省全部预留，避免 `SpellDef` 结构再变一次）。

use crate::creature::{CreatureTable, Creatures, MAX_CREATURES};
use crate::emit;
use crate::fixed::{self, Bam, Fx};
use crate::input::{InputFrame, BTN_FIRE};
use crate::projectile::Projectiles;
use crate::rng;
use crate::world::SpawnRequest;

/// 空槽哨兵（`Creature::loadout` 数组的"这一槽没有法术"取值）。法术表上限
/// 因此是 255（id 是 `u8`，255 保留给这个哨兵，`load_spells` 加载期校验）。
pub const SPELL_NONE: u8 = 255;

/// `Blast` 在 `Projectiles::advance` 里派给 `explode::apply_explode` 的
/// `op_idx` 起点（spec §5.3，与 ops 阶段 `Op::Explode` 共用 `STREAM_EXPLODE`
/// 的 salt 维度——两者的 `op_idx` 值域必须不重叠，否则掷出同值，正是总纲
/// §11 翻案第 4 条"同帧同源多次掷骰须区分"点名的反例）。
///
/// `apply_explode` 内部把 `op_idx` **原样转 `u32` 当 salt 用**（无位宽折叠，
/// `explode.rs::apply_explode` 文档 `let salt = op_idx as u32;`），故只需
/// 远高于 ops 阶段 `op_idx`（`enumerate()` 下标，任何合理场景远不到
/// `1<<16`）即可，取 `1<<20` 留足余量。`BLAST_OP_IDX_BASE + i`（`i` = 弹体
/// 在 `Projectiles` SoA 里的下标，本 tick `advance()` 循环内唯一）覆盖
/// `[1<<20, 1<<20 + MAX_PROJECTILES)`，与 ops 阶段区间不相交
/// （`blast_op_idx_base_does_not_collide_with_ops_phase` 单测钉死）。
pub const BLAST_OP_IDX_BASE: usize = 1 << 20;

/// `cast_all` 里 `Spray` 分支派给 `emit::apply_emit` 的 `op_idx` 起点（spec
/// §6，与 ops 阶段 `Op::Emit` 共用 `STREAM_EMIT` 的 salt 维度，理由同
/// [`BLAST_OP_IDX_BASE`]）。
///
/// **不能照抄 `BLAST_OP_IDX_BASE` 的量级**：`emit::apply_emit` 走的是
/// `emit_salt(op_idx, i) = (op_idx << 16) | (i & 0xFFFF)`——`op_idx` 被
/// 折进 **32 位里的高 16 位**，`emit_salt` 自带 `debug_assert!(op_idx <=
/// u16::MAX as usize)`（越界在 debug/test 构建直接 panic，这条硬约束是
/// TDD 阶段实测发现的：`1<<21` 起步直接把 debug_assert 打爆，不是纸面
/// 推测）。`Op::Emit` 是既有共享代码，不能为了这一个新调用点改宽它的
/// 位布局——那会改变全部既有场景 `Op::Emit` 的抖动序列，牵连一堆本不该
/// 变的 golden。故改用**16 位空间顶端的一小段保留区**：
/// `u16::MAX + 1 - MAX_CREATURES`（`MAX_CREATURES = 16`，起点 `65520`），
/// `SPRAY_OP_IDX_BASE + id`（`id` = 施法生物 id，`0..MAX_CREATURES`）覆盖
/// `[65520, 65536)`，恰好卡在 `u16::MAX`（含）以内，同时远高于任何真实
/// 场景单 tick 的 `Op::Emit` 计数（`spray_op_idx_base_fits_emit_salts_16bit_budget`
/// 单测钉死上界与"远大于合理 ops 计数"两条）。
pub const SPRAY_OP_IDX_BASE: usize = u16::MAX as usize + 1 - MAX_CREATURES;

/// `cast_all` 里 `Spray` 分支调用 `emit::apply_emit` 时填的 `stamp`。
///
/// **不是偷懒占位**：`emit_attempt` 需要 `stamp` 的唯一场景是"`apply_setup`
/// 与紧接的 tick 0 首个 `step()` 共享同一 `fseed`"（`emit.rs::emit_attempt`
/// 文档）——`cast_all` 只在 `Sim::step` 内被调用、从不在 `apply_setup` 里跑，
/// 这条边界条件对本调用点不成立；这里的碰撞防线完全靠 `fseed`（每个真实
/// tick 天然不同）与 `SPRAY_OP_IDX_BASE`（与 ops 阶段的 `Op::Emit` op_idx
/// 值域隔离），`stamp` 传固定值不削弱防线，只是 `apply_emit` 签名要求这个
/// 形参、这里给一个不等于 `SETUP_STAMP`（255）的任意常量占位。
const CAST_STAMP: u8 = 0;

/// 法术效果种类（spec §3.4）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpellKind {
    /// 直射弹：命中生物一次性扣血 + 沿弹体飞行方向击退；命中硬格消失
    /// （spec §5.3）。`damage_milli`：千分位整数伤害，与 `Creature::hp`
    /// 同一量纲（`creature.rs` 文档："hp 用整数千分位"）；`knockback`：
    /// 击退速度大小（格/tick，Q16.16）。
    Bolt { damage_milli: i32, knockback: Fx },
    /// 爆炸弹：命中（生物或硬格）时在命中点触发一次**现有** `explode::
    /// apply_explode` 全套（网格射线 + 溅射）+ 追加 `Bodies::pending_blasts`
    /// （spec §5.3）——零新增通路，与 `Op::Explode` 完全同一口径。
    ///
    /// **没有独立的 damage/knockback 字段**：命中生物时不叠加 `Bolt` 式的
    /// 直接扣血/击退，伤害与击退完全来自爆炸本身。这是对 spec §5.3 表
    /// "命中生物：同上 + 触发爆炸"一句措辞的显式澄清——`data/spells.ron`
    /// 的 `bomb` 样例只给了 `power`/`radius`/`max_durability` 三项，字段
    /// 形状即事实；若未来真要求独立的直接命中伤害，需要先给 `kind` 加字段，
    /// 不是本 Task 该做的翻案。
    Blast { power: u32, radius: i32, max_durability: u8 },
    /// 喷射：**不产生弹体**，施法当帧直接走既有 `emit::apply_emit` 通路塞
    /// 进 `spawn_queue`，与 `Op::Emit` 同一语义（spec §3.4/§6）。`material`
    /// 已在加载期经 `MaterialTable::id_by_name` 解析成 id。
    Spray { material: u8, count: u16, speed: Fx, jitter: Fx },
}

/// 单条法术定义（spec §3.4 `data/spells.ron` 的 Rust 侧对应）。与
/// `MaterialTable`/`CreatureTpl` 同体例：加载期一次性量化好的整数/`Fx`，运行
/// 时纯读取。
#[derive(Clone, Debug)]
pub struct SpellDef {
    pub name: String,
    pub kind: SpellKind,
    /// 施法蓝耗（千分位整数，`quantize_milli`）——`cast_all` 双闸门之一
    /// （spec §6.1）。
    pub mana: i32,
    /// 施法冷却（tick）——双闸门之二。
    pub cooldown: u16,
    /// 出射速度大小：`cast_all` 用出射方向 × `speed` 算初速度。
    pub speed: Fx,
    /// 出生寿命（tick）：`Projectiles::advance` 每 tick 未命中即递减，归零销毁。
    pub life: u16,
    /// 每 tick 施加的重力（spec §5.1 "vy += spell.gravity"）；直射弹通常为 0。
    pub gravity: Fx,
    /// 出射散布半幅（BAM，`quantize_bam` 量化，加载期校验 `0..=180`
    /// 对应度数域）。`cast_all`：`> 0` 才掷 `STREAM_SPREAD` 骰，`== 0` 时
    /// 出射方向恒等于瞄准角，不掺任何随机——零散布法术必须完全可预测。
    pub spread_bam: Bam,
    /// 防自伤宽限帧数（spec §5.3）：`owner` 在此窗口内跳过自身命中判定。
    pub grace: u8,
    /// 侵彻能量预算（Noita `ground_penetration_*`，spec §5.2）——Task 5 不
    /// 消费（命中硬格直接消失，无能量结算），只作为 `Projectiles::spawn`
    /// 的 `energy` 列初值来源。Task 6 中性缺省 = 0，意味着"这颗弹打不穿
    /// 任何东西"，与"命中即消失"语义天然一致。
    pub dig_power: u32,
    /// 侵彻门槛（spec §5.2，`data/spells.ron` 的顶层 `max_durability`，
    /// 与 `SpellKind::Blast::max_durability`——爆炸自身的破坏门槛——是两个
    /// 独立字段：前者管这颗弹自己撞墙能不能钻进去，后者管它炸出来的坑）。
    /// Task 5 不消费，Task 6 起 `resolve_hit` 的侵彻判定才会读。
    pub max_durability: u8,
    /// 每 tick 速度衰减乘子（Noita `air_friction`）——Task 5 不消费。中性
    /// 缺省 = 1（不衰减）。
    pub air_friction: Fx,
    /// 液体内每 tick 额外速度衰减乘子（spec §5.1"若起点在液体格内: (vx,vy)
    /// *= liquid_drag"）——Task 5 不消费，Task 6 起 `advance` 的液体判定
    /// 才会读。中性缺省 = 1（不衰减）。
    pub liquid_drag: Fx,
    /// 穿透掩码（`Category::bit()` 位或，spec §3.4"`pass_through` 掩码里的
    /// `Category` 不算命中格，直接穿过"）——Task 5 不消费，Task 6 起
    /// `advance` 的 DDA 判定才会读。中性缺省 = 0（不穿透任何类别）。
    pub pass_through: u8,
    /// 飞行路径上的液体格是否脱格（spec §5.5，复用 §4.3 排开同一通路）——
    /// Task 5 不消费。中性缺省 = false。
    pub displace_liquid: bool,
    /// 剩余弹跳次数（spec §5.4）——Task 5 不消费（命中硬格直接消失，无
    /// 弹跳分支）。中性缺省 = 0，仅作为 `Projectiles::spawn` 的 `bounces`
    /// 列初值来源。
    pub bounces: u8,
    /// 弹跳后速度保留比例（spec §5.4"对应轴速度取反并乘 bounce_energy"）
    /// ——Task 5 不消费。中性缺省 = 0（配 `bounces = 0` 时从不读取）。
    pub bounce_energy: Fx,
    /// 命中刚体时的单点冲量系数（Noita `physics_impulse_coeff`，spec
    /// §5.5，千分位整数 `quantize_milli`）——Task 5 不消费。中性缺省 = 0。
    pub physics_impulse: i32,
    /// 寿命耗尽时是否触发一次爆炸（spec §5.1"life == 0 时若
    /// on_lifetime_out_explode 则先炸"）——Task 5 不消费。中性缺省 = false。
    pub on_lifetime_out_explode: bool,
}

impl SpellDef {
    /// 测试与程序化构表用（`pub`，不依赖 `spells.ron`）：`Bolt` 变体 + Task 4
    /// 实际消费的字段（`damage_milli`/`knockback`/`speed`/`life`/`grace`），
    /// 双闸门字段取"总打得出"的中性值（`mana = 0`、`cooldown = 0`），Task 6
    /// 前的其余字段取中性缺省——`gravity = 0`（直射，不下坠）、`spread_bam
    /// = 0`（不掺散布骰）、`dig_power = 0`、`max_durability = 0`、
    /// `air_friction = 1`、`liquid_drag = 1`、`pass_through = 0`、
    /// `displace_liquid = false`、`bounces = 0`、`bounce_energy = 0`、
    /// `physics_impulse = 0`、`on_lifetime_out_explode = false`，保证这些
    /// 字段在关闭状态下不改变直线飞行 + 命中判定语义。
    pub fn test_bolt(name: &str, damage_milli: i32, knockback: Fx, speed: Fx, life: u16, grace: u8) -> SpellDef {
        SpellDef {
            name: name.to_string(),
            kind: SpellKind::Bolt { damage_milli, knockback },
            mana: 0,
            cooldown: 0,
            speed,
            life,
            gravity: Fx::ZERO,
            spread_bam: 0,
            grace,
            dig_power: 0,
            max_durability: 0,
            air_friction: Fx::from_int(1),
            liquid_drag: Fx::from_int(1),
            pass_through: 0,
            displace_liquid: false,
            bounces: 0,
            bounce_energy: Fx::ZERO,
            physics_impulse: 0,
            on_lifetime_out_explode: false,
        }
    }
}

/// 法术表（与 `MaterialTable`/`CreatureTable` 同体例：加载期构造、只读）。
/// **类型住在 core、加载器住在 harness**——`SpellTable::from_defs` 是程序化
/// 构表入口，`data/spells.ron` 解析走 `sand-harness::scenario::load_spells`。
#[derive(Clone, Debug, Default)]
pub struct SpellTable {
    defs: Vec<SpellDef>,
}

impl SpellTable {
    pub fn empty() -> SpellTable {
        SpellTable::default()
    }

    pub fn from_defs(defs: Vec<SpellDef>) -> SpellTable {
        SpellTable { defs }
    }

    /// 法术号越界即调用方漏配置——与 `CreatureTable::get`/`MaterialTable::category`
    /// 同一体例，脏值防御在加载期做，这里直接索引。
    pub fn get(&self, id: u8) -> &SpellDef {
        &self.defs[id as usize]
    }

    /// 按名字查 id（测试/harness 用，同 `MaterialTable::id_by_name` 体例）。
    /// 线性扫描：法术数上限 255，且只在加载期/测试构表用，不在任何逐 tick
    /// 热路径上——不值得为它引入一张额外的名字索引结构。
    pub fn id_by_name(&self, name: &str) -> Option<u8> {
        self.defs.iter().position(|d| d.name == name).map(|i| i as u8)
    }
}

/// 32-bit 随机数 → 均匀落在 `[-half, +half]`（BAM 环绕域，spec §6.2）。
///
/// 乘法-右移法（同 `emit::emit_jitter`/`explode::ray_fluct` 的整数缩放数学，
/// 无取模偏置）。设 `width = 2*half + 1`（闭区间元素数）；缩放量
/// `d = (r as u64 * width) 右移 32 位`落在 `[0, width)`（`r = 0` 给下界 0，
/// `r = u32::MAX` 逼近但小于 `width`，给上界 `width - 1 = 2*half`），再减
/// `half` 平移居中即落入 `[-half, +half]` 闭区间，两端均可达。
///
/// `(d as i64 - half as i64) as u16`：差值域 `[-half, +half]` ⊂ `[-65535,
/// 65535]`，`as u16` 走二补码环绕表达负角（BAM 本就是无符号环绕域，
/// `-1i64 as u16 == 65535` 正是"逆时针转 1 单位"，与 `Bam` 全部算术走
/// `wrapping_add` 的既有约定一致）。`half == 0` 提前返回 0，不掷骰、不做
/// `width = 1` 的退化路径（`cast_all` 本就只在 `spread_bam > 0` 时调用本
/// 函数，这层判断是纯防御，函数本身仍需自洽）。
fn bam_in_range(r: u32, half: Bam) -> Bam {
    if half == 0 {
        return 0;
    }
    let width = 2u64 * half as u64 + 1;
    let d = ((r as u64) * width) >> 32;
    (d as i64 - half as i64) as u16
}

/// 施法结算（架构 §4 第 2d 步，spec §6）：按 creature id 序，每生物每 tick
/// 至多一发。
///
/// **不接收 `world`/`table`/`bodies`**（对 brief 原始签名的一处收窄，R8
/// 裁决）：`cast_all` 只做两件事——`Bolt`/`Blast` 落 `Projectiles::spawn`
/// （命中结算延后到 `Projectiles::advance` 里的 `resolve_hit`，那才是真正
/// 触碰网格/刚体的地方）、`Spray` 直接 `emit::apply_emit` 塞 `spawn_queue`
/// （只读发射点坐标、不碰网格），三条路径都不需要 `world`/`table`/`bodies`
/// ——加了就是纯粹的未用形参，`cargo clippy -D warnings` 的
/// `unused_variables`/`needless_pass_by_ref_mut` 零容忍，编译直接失败。
#[allow(clippy::too_many_arguments)]
pub(crate) fn cast_all(
    creatures: &mut Creatures,
    projectiles: &mut Projectiles,
    spells: &SpellTable,
    tpl: &CreatureTable,
    inputs: &[InputFrame],
    fseed: u32,
    spawns: &mut Vec<SpawnRequest>,
) {
    for i in 0..creatures.len() {
        let id = i as u8;
        // R9 裁决：`creatures.input_of(&self, ..)` 与 `&mut Creature`
        // 不能同时持有——存活性判定 + 按键读取必须先做完（借用 `&self`），
        // 再取 `get_mut`（借用 `&mut`），不能像 brief 原始伪代码那样两者
        // 交叉持有。
        let alive = creatures.get(id).map(|c| c.alive).unwrap_or(false);
        if !alive {
            continue;
        }
        let inp = creatures.input_of(i, inputs);

        let Some(c) = creatures.get_mut(id) else { continue };
        let t = tpl.get(c.template);
        // ① 收尾类更新先做：本 tick 冷却好了就能放，回蓝同理（spec §6.1
        // "每 tick 收尾：cooldowns 全体饱和递减 1；mana = min(mana_max,
        // mana + mana_regen/60)"——这一步与是否本 tick 按下开火键无关，
        // 无条件对每个存活生物执行。
        for cd in c.cooldowns.iter_mut() {
            *cd = cd.saturating_sub(1);
        }
        c.mana = (c.mana + t.mana_regen_per_tick).min(t.mana_max);

        if !inp.held(BTN_FIRE) {
            continue;
        }
        let slot = inp.slot as usize; // InputFrame::new 已在构造点 clamp 进 0..MAX_SLOTS。
        let sid = c.loadout[slot];
        if sid == SPELL_NONE {
            continue;
        }
        let s = spells.get(sid);
        // 双闸门：任一不满足即不出，且**在此之前不得有任何写**（本函数到
        // 这里为止唯一写过的状态是①的冷却/回蓝，那是无条件的"收尾"，不算
        // "施法的副作用"——commit message 与 brief 都明确"不出就不得扣费、
        // 不得置冷却"，指的是 `mana -= s.mana`/`cooldowns[slot] = s.cooldown`
        // 这两步，不包含被动回蓝）。
        if c.cooldowns[slot] > 0 || c.mana < s.mana {
            continue;
        }
        c.mana -= s.mana;
        c.cooldowns[slot] = s.cooldown;

        // ② 方向：aim + 散布骰（spread_bam == 0 时不掷骰，spec §6.2）。
        let mut a = c.aim;
        if s.spread_bam > 0 {
            let r = rng::rng_u32(fseed, rng::STREAM_SPREAD, id as i32, 0, slot as u32, 0);
            a = a.wrapping_add(bam_in_range(r, s.spread_bam));
        }
        let (dx, dy) = fixed::dir_of(a);
        let muzzle_x = c.x + dx.mul_int(t.muzzle_offset);
        let muzzle_y = c.y + dy.mul_int(t.muzzle_offset);

        // ③ 派发。
        match s.kind {
            SpellKind::Spray { material, count, speed, jitter } => {
                // 不产生弹体：直接走既有 emit 通路，与 Op::Emit 同一队列
                // 同一语义（spec §3.4）。
                emit::apply_emit(
                    material,
                    muzzle_x,
                    muzzle_y,
                    dx.mul(speed),
                    dy.mul(speed),
                    count,
                    jitter,
                    CAST_STAMP,
                    fseed,
                    SPRAY_OP_IDX_BASE + id as usize,
                    spawns,
                );
            }
            _ => {
                projectiles.spawn(
                    sid,
                    muzzle_x,
                    muzzle_y,
                    dx.mul(s.speed),
                    dy.mul(s.speed),
                    s.life,
                    s.dig_power,
                    id,
                    s.grace,
                    s.bounces,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bam_in_range_stays_within_bounds_and_reaches_both_ends() {
        let half: Bam = 1000;
        assert_eq!(bam_in_range(0, half), (0i64 - half as i64) as u16, "r=0 触达下界 -half");
        assert_eq!(bam_in_range(u32::MAX, half), half, "r=u32::MAX 触达上界 +half");
        // 中间若干采样点必须全部落在 [-half, +half]（用 i16 位模式解读环绕值）。
        for r in [1u32, 12345, 1 << 20, u32::MAX / 2, u32::MAX - 1] {
            let d = bam_in_range(r, half) as i16;
            assert!(
                (-(half as i16)..=(half as i16)).contains(&d),
                "r={r} 产出 {d} 越出 [-{half}, {half}]"
            );
        }
    }

    #[test]
    fn bam_in_range_half_zero_never_rolls_away_from_zero() {
        for r in [0u32, 1, u32::MAX / 2, u32::MAX] {
            assert_eq!(bam_in_range(r, 0), 0, "half=0 必须恒返回 0（不掺散布）");
        }
    }

    /// `BLAST_OP_IDX_BASE` 与 ops 阶段 `Op::Explode` 的 `op_idx`（`enumerate()`
    /// 下标，任何合理场景远不到 `1<<16`）不相交——两者共用 `STREAM_EXPLODE`
    /// 的 salt 维度，`apply_explode` 把 `op_idx` 原样转 `u32` 当 salt，无
    /// 位宽约束，只要基址够大就行。
    #[test]
    // 断言操作数全是编译期常量——这正是本测试要钉死的东西（op_idx 区间的
    // 相对大小是设计时决定的常量关系），clippy 默认假设这种写法是笔误，
    // 这里显式关掉。
    #[allow(clippy::assertions_on_constants)]
    fn blast_op_idx_base_does_not_collide_with_ops_phase() {
        const REASONABLE_MAX_OPS_PER_TICK: usize = 1 << 16; // 远超任何真实场景的单 tick op 数
        assert!(BLAST_OP_IDX_BASE > REASONABLE_MAX_OPS_PER_TICK, "Blast 基址必须远高于 ops 阶段 op_idx 上界");
    }

    /// `SPRAY_OP_IDX_BASE` 必须同时满足两条（`SPRAY_OP_IDX_BASE` 文档"不能
    /// 照抄 `BLAST_OP_IDX_BASE` 量级"那段的执法测试）：
    /// ① 落在 `emit_salt` 的 16 位折叠预算内（`+ (MAX_CREATURES-1)` 后仍
    ///   `<= u16::MAX`，否则 `emit.rs::emit_salt` 的 `debug_assert` 直接
    ///   panic——这是本 Task TDD 阶段实测撞见的失败，不是纸面推演）；
    /// ② 仍然远高于任何真实场景单 tick 的 `Op::Emit` 计数，与 ops 阶段的
    ///   op_idx 值域不相交。
    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn spray_op_idx_base_fits_emit_salts_16bit_budget_and_stays_clear_of_ops_phase() {
        const REASONABLE_MAX_EMITS_PER_TICK: usize = 4096; // 远超任何真实场景的单 tick Op::Emit 数
        assert!(
            SPRAY_OP_IDX_BASE + (crate::creature::MAX_CREATURES - 1) <= u16::MAX as usize,
            "Spray 段最高一个 id 的 op_idx 必须仍落在 emit_salt 的 16 位预算内"
        );
        assert!(
            SPRAY_OP_IDX_BASE > REASONABLE_MAX_EMITS_PER_TICK,
            "Spray 基址必须远高于 ops 阶段 Op::Emit 的 op_idx 上界"
        );
    }
}
