//! 法术：表 + loadout + 施法闸门（spec §3.4、§6）。M4 Task 4 建**核心类型本体**
//! ——`SpellKind`/`SpellDef`/`SpellTable`——但只填 `Bolt` 一个变体；`Blast`/
//! `Spray` 两个变体、`data/spells.ron` 加载器、cooldown+mana 双闸门留 Task 5，
//! 侵彻/弹跳/阻力等七项扩展字段的**消费**留 Task 6（字段本体已在此按 Task 6
//! 的中性缺省预留，避免 `SpellDef` 结构在后续 Task 反复变更——与 `CreatureTpl`
//! 提前建好 `mana_max`/`swim_*` 字段同一先例，`creature.rs` 头注可参照）。

use crate::fixed::Fx;

/// 法术效果种类（spec §3.4）。本 Task 只有 `Bolt`；`Blast`/`Spray` 由 Task 5 追加。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpellKind {
    /// 直射弹：命中生物一次性扣血 + 沿弹体飞行方向击退；命中硬格消失
    /// （spec §5.3）。`damage_milli`：千分位整数伤害，与 `Creature::hp`
    /// 同一量纲（`creature.rs` 文档："hp 用整数千分位"）；`knockback`：
    /// 击退速度大小（格/tick，Q16.16）。
    Bolt { damage_milli: i32, knockback: Fx },
}

/// 单条法术定义（spec §3.4 `data/spells.ron` 的 Rust 侧对应）。与
/// `MaterialTable`/`CreatureTpl` 同体例：加载期一次性量化好的整数/`Fx`，运行
/// 时纯读取。
#[derive(Clone, Debug)]
pub struct SpellDef {
    pub name: String,
    pub kind: SpellKind,
    /// 出射速度大小（Task 5 施法路径消费：出射方向 × `speed`）。Task 4 的
    /// 弹体注入走 `Sim::queue_projectile`，速度由调用方直接给，**不读本字段**
    /// ——`speed` 提前建好只为 Task 5 接线时不必再改 `SpellDef` 结构。
    pub speed: Fx,
    /// 出生寿命（tick）：`Projectiles::advance` 每 tick 未命中即递减，归零销毁。
    pub life: u16,
    /// 每 tick 施加的重力（spec §5.1 "vy += spell.gravity"）；直射弹通常为 0。
    pub gravity: Fx,
    /// 防自伤宽限帧数（spec §5.3）：`owner` 在此窗口内跳过自身命中判定。
    pub grace: u8,
    /// 侵彻能量预算（Noita `ground_penetration_*`，spec §5.2）——**Task 4 不
    /// 消费**（命中硬格直接消失，无能量结算），只作为 `Projectiles::spawn`
    /// 的 `energy` 列初值来源（`Sim::queue_projectile` 文档）。Task 6 中性
    /// 缺省 = 0（`SpellDef::test_bolt`），意味着"这颗弹打不穿任何东西"，
    /// 与 Task 4 的"命中即消失"语义天然一致，Task 6 接线侵彻判定时不需要
    /// 再回头改这颗测试弹的语义。
    pub dig_power: u32,
    /// 每 tick 速度衰减乘子（Noita `air_friction`，spec §5.1 注释"Task 6 在
    /// 此插入"）——**Task 4 不消费**。中性缺省 = 1（不衰减）。
    pub air_friction: Fx,
    /// 剩余弹跳次数（spec §5.4）——**Task 4 不消费**（命中硬格直接消失，无
    /// 弹跳分支）。中性缺省 = 0，仅作为 `Projectiles::spawn` 的 `bounces`
    /// 列初值来源。
    pub bounces: u8,
}

impl SpellDef {
    /// 测试与程序化构表用（`pub`，不依赖 `spells.ron`）：`Bolt` 变体 + Task 4
    /// 实际消费的字段（`damage_milli`/`knockback`/`speed`/`life`/`grace`），
    /// 其余字段取 Task 6 前的中性缺省——`gravity = 0`（直射，不下坠）、
    /// `dig_power = 0`、`air_friction = 1`、`bounces = 0`，保证这些字段在
    /// 关闭状态下不改变 Task 4 的直线飞行 + 命中判定语义。
    pub fn test_bolt(name: &str, damage_milli: i32, knockback: Fx, speed: Fx, life: u16, grace: u8) -> SpellDef {
        SpellDef {
            name: name.to_string(),
            kind: SpellKind::Bolt { damage_milli, knockback },
            speed,
            life,
            gravity: Fx::ZERO,
            grace,
            dig_power: 0,
            air_friction: Fx::from_int(1),
            bounces: 0,
        }
    }
}

/// 法术表（与 `MaterialTable`/`CreatureTable` 同体例：加载期构造、只读）。
/// **类型住在 core、加载器住在 harness**——`SpellTable::from_defs` 是本 Task
/// 唯一的构造入口，`data/spells.ron` 解析留 `sand-harness`（Task 5）。
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
}
