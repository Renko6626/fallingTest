//! 弹体行为集成测试（M4 Task 4 spec §5、Task 5 spec §6）：经 `Sim` 公共 API
//! 驱动。范围：Task 4 部分——直线飞行、DDA 命中判定（先到者优先：生物 vs
//! 硬格）、grace 防自伤、同队免疫、出界/寿命耗尽销毁、容量限流、逐 tick
//! 恰好一次积分。Task 5 部分——`cast_all` 双闸门（cooldown + mana）、
//! `Blast`/`Spray` 派发、出射方向（aim + 散布）、出生点偏移。侵彻/弹跳/
//! 阻力/穿透/排开/冲量/定时爆（Task 6）不在本文件范围内。

mod common;

use sand_core::{input::BTN_FIRE, spell::*, Fx, InputFrame, Op, Sim, MAX_PROJECTILES};

use common::arena_with_loadout;

/// 本 Task 的测试法术表：0 号 = 普通直射弹（life 长），1 号 = 短命弹（life 5）。
fn bolt_table() -> SpellTable {
    SpellTable::from_defs(vec![
        SpellDef::test_bolt(
            "bolt", /* damage_milli */ 5_000, /* knockback */ Fx::from_int(2),
            /* speed */ Fx::from_int(8), /* life */ 120, /* grace */ 4,
        ),
        SpellDef::test_bolt("shortlived", 5_000, Fx::ZERO, Fx::from_int(8), 5, 4),
    ])
}

/// 弹体注入：走 `Sim::queue_projectile`（内部就是 `Projectiles::spawn`，测试
/// 跑的就是产品代码），速度由调用方直接给格/tick。出生点取格心（+半格）——
/// 与 `rules.rs`/`explode.rs`/`creature.rs` 里其它脱格出生点同一惯例
/// （`Fx::from_ratio(1, 2)` 与 `fixed::HALF_CELL` 位模式相同，见 `fixed.rs`
/// 金值测试；用 `from_ratio` 而非导入 `HALF_CELL` 是因为该常量是
/// `pub(crate)`，不对集成测试开放，没必要为一个测试专门放宽核心可见性）。
fn shoot(sim: &mut Sim, spell: u8, x: i32, y: i32, vx: Fx, vy: Fx, owner: u8) {
    let half = Fx::from_ratio(1, 2);
    let (px, py) = (Fx::from_int(x) + half, Fx::from_int(y) + half);
    sim.queue_projectile(spell, px, py, vx, vy, owner);
}

#[test]
fn projectile_flies_straight_and_dies_on_wall() {
    let mut sim = common::arena_wide_open(bolt_table());
    shoot(&mut sim, 0, 10, 64, Fx::from_int(8), Fx::ZERO, 255);
    for _ in 0..60 {
        sim.step(&[], &[]);
    }
    assert_eq!(sim.projectiles().len(), 0, "撞墙后必须销毁");
}

#[test]
fn projectile_damages_a_creature_it_hits() {
    let mut sim = common::arena_with_two_creatures(bolt_table()); // id 0 在左，id 1 在右
    let hp0 = sim.creatures().get(1).unwrap().hp;
    shoot(&mut sim, 0, 30, 64, Fx::from_int(8), Fx::ZERO, 0);
    for _ in 0..40 {
        sim.step(&[], &[]);
    }
    assert!(sim.creatures().get(1).unwrap().hp < hp0, "命中应当扣血");
    assert_eq!(sim.projectiles().len(), 0, "命中生物后弹体销毁");
}

#[test]
fn projectile_knockback_pushes_the_target() {
    let mut sim = common::arena_with_two_creatures(bolt_table());
    let x0 = sim.creatures().get(1).unwrap().x;
    shoot(&mut sim, 0, 30, 64, Fx::from_int(8), Fx::ZERO, 0);
    for _ in 0..40 {
        sim.step(&[], &[]);
    }
    assert!(sim.creatures().get(1).unwrap().x > x0, "击退应把目标推开");
}

/// 评审 Important（2026-09-06）：`projectile_does_not_hit_its_owner_during_grace`
/// 只跑 3 tick（grace=4），三次 `first_hit_at` 调用全部落在 `grace > 0` 分支，
/// 从未触达"grace 耗尽后允许自伤"这条 else 分支——那条分支恰恰是
/// `Creatures::first_hit_at` 里"owner 与同队拆成两条独立规则"这个设计点唯一
/// 有歧义的地方（`creature.rs::first_hit_at` 文档），必须单独钉死。
///
/// 构造：弹体从 owner 左侧 10 格外以 1 格/tick **缓慢**逼近（不复用
/// `bolt_table()` 0 号的 8 格/tick——那个速度一步就跨完 grace 窗口，撞上的
/// 瞬间 grace 可能还没耗尽，无法把"命中时刻"精确摆在 grace 耗尽之后）。
/// 抵达 owner AABB（半宽 2、中心 x=20，左边缘格 x=18）至少要 8 tick——
/// grace=4 在第 4 tick 结束时就已经饱和归零（`saturating_sub`），命中发生时
/// grace 已经归零至少 4 个 tick，不存在"卡在边界、到底是不是真的过期"的歧义。
/// 路径上（x=10..18，y=64）既无墙也无世界边界，不会在 grace 耗尽前提前销毁。
#[test]
fn projectile_can_hit_its_owner_after_grace_expires() {
    let mut sim = common::arena_with_two_creatures(bolt_table());
    let hp0 = sim.creatures().get(0).unwrap().hp;
    shoot(&mut sim, 0, 10, 64, Fx::from_int(1), Fx::ZERO, 0);
    for _ in 0..15 {
        sim.step(&[], &[]);
    }
    assert!(sim.creatures().get(0).unwrap().hp < hp0, "grace 耗尽后必须允许命中 owner 自身");
    assert_eq!(sim.projectiles().len(), 0, "命中 owner 后弹体同样销毁");
}

#[test]
fn projectile_does_not_hit_its_owner_during_grace() {
    let mut sim = common::arena_with_two_creatures(bolt_table());
    let hp0 = sim.creatures().get(0).unwrap().hp;
    // 出生就在 owner 身上、速度指向它自己：grace = 4，前 3 tick 不得自伤。
    shoot(&mut sim, 0, 20, 64, Fx::from_int(-1), Fx::ZERO, 0);
    for _ in 0..3 {
        sim.step(&[], &[]);
    }
    assert_eq!(sim.creatures().get(0).unwrap().hp, hp0, "grace 帧内不得自伤");
}

#[test]
fn projectile_skips_same_team() {
    let mut sim = common::arena_with_two_creatures_same_team(bolt_table());
    let hp0 = sim.creatures().get(1).unwrap().hp;
    shoot(&mut sim, 0, 30, 64, Fx::from_int(8), Fx::ZERO, 0);
    for _ in 0..40 {
        sim.step(&[], &[]);
    }
    assert_eq!(sim.creatures().get(1).unwrap().hp, hp0, "同队不得命中");
}

#[test]
fn projectile_dies_when_lifetime_runs_out() {
    let mut sim = common::arena_wide_open(bolt_table());
    shoot(&mut sim, 1, 100, 64, Fx::ZERO, Fx::ZERO, 255); // 1 号：life 5，静止不撞墙
    for _ in 0..6 {
        sim.step(&[], &[]);
    }
    assert_eq!(sim.projectiles().len(), 0, "寿命耗尽即销毁");
}

#[test]
fn spawn_beyond_capacity_is_rejected_deterministically() {
    let mut sim = common::arena_wide_open(bolt_table());
    for _ in 0..MAX_PROJECTILES + 10 {
        shoot(&mut sim, 0, 100, 64, Fx::ZERO, Fx::ZERO, 255);
    }
    assert_eq!(sim.projectiles().len(), MAX_PROJECTILES, "超限必须确定性拒绝");
}

#[test]
fn projectile_moves_exactly_once_per_tick() {
    let mut sim = common::arena_wide_open(bolt_table());
    shoot(&mut sim, 0, 10, 64, Fx::from_int(8), Fx::ZERO, 255);
    let x0 = sim.projectiles().x(0);
    sim.step(&[], &[]);
    let x1 = sim.projectiles().x(0);
    assert_eq!(x1 - x0, Fx::from_int(8), "每 tick 恰好走一次速度，不多不少");
}

// ==================== M4 Task 5：法术表与施法（spec §6） ====================
//
// 测试地一律走 `common::arena_with_loadout`（R11 helper，内建独立于
// `data/spells.ron` 的测试法术表——见其文档，`spread_bam` 全部取 0，`bomb`
// 的 `max_durability`/`power` 足以炸穿测试材料表的 `stone`）。
//
// **`Creature::spawn` 现在满蓝出生**（M4 Task 5 修正，`creature.rs::spawn`
// 文档：`mana: t.mana_max`，对称于既有的 `hp: t.hp_max`）——本文件绝大多数
// 测试因此不需要先攒蓝就能立刻验证一次施法；只有
// `mana_gate_blocks_when_insufficient_and_costs_nothing` 需要显式调低蓝量
// 才能触发闸门。

#[test]
fn firing_consumes_mana_and_sets_cooldown() {
    let mut sim = arena_with_loadout(&["spark_bolt"]);
    let m0 = sim.creatures().get(0).unwrap().mana;
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    let c = sim.creatures().get(0).unwrap();
    assert!(c.mana < m0, "施法必须扣 mana");
    assert!(c.cooldowns[0] > 0, "施法必须置冷却");
    assert_eq!(sim.projectiles().len(), 1, "应当出一发");
}

#[test]
fn cooldown_gate_blocks_a_second_shot() {
    let mut sim = arena_with_loadout(&["spark_bolt"]);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    assert_eq!(sim.projectiles().len(), 1, "冷却未好不得再出");
}

/// **对 brief 字面测试的一处修正**（本 Task 实施期发现）：brief 的
/// `mana_gate_blocks_when_insufficient_and_costs_nothing` 断言步后 `mana`
/// 恒等于 0，但 spec §6.1"每 tick 收尾：… mana = min(mana_max, mana +
/// mana_regen/60)"是**无条件**的被动回蓝——即便本 tick 施法被闸门挡下，
/// 回蓝仍照常发生（"不出就不得扣费/不得置冷却"指的是这两项**主动**副
/// 作用，不包含被动回蓝本身）。`default_player` 的 `mana_regen_per_tick =
/// 333 > 0`，一步之后蓝量必然是 `0 + 333`，不可能保持精确的 0——若真按
/// 字面断言 `mana == 0`，等于要求回蓝速率为 0，那样反过来会让
/// `mana_regenerates_up_to_max`（600 tick 内回满）永远不可能通过：两条
/// 测试用的是同一个生物模板（`arena_with_loadout` 内部固定
/// `CreatureTable::default_player()`），回蓝速率只能有一个值。故这里改为
/// 断言"步后蓝量恰好等于回蓝增量"（而非扣费后的值），既验证了"没有被
/// 施法扣费"（如果扣了费，`0 + regen - s.mana` 会是负值，或者说不会等于
/// `0 + regen` 这个精确值），也不与被动回蓝的既有语义冲突。
#[test]
fn mana_gate_blocks_when_insufficient_and_costs_nothing() {
    let mut sim = arena_with_loadout(&["expensive_bolt"]);
    sim.creatures_mut().set_mana(0, 0);
    let regen = sim.creature_table().get(sim.creatures().get(0).unwrap().template).mana_regen_per_tick;
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    assert_eq!(sim.projectiles().len(), 0, "mana 不足不得出");
    assert_eq!(sim.creatures().get(0).unwrap().mana, regen, "不出就只应有被动回蓝，不得扣费");
    assert_eq!(sim.creatures().get(0).unwrap().cooldowns[0], 0, "不出就不得置冷却");
}

#[test]
fn empty_slot_is_a_no_op() {
    let mut sim = arena_with_loadout(&[]); // 全空槽
    let m0 = sim.creatures().get(0).unwrap().mana;
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    // m0 出生即满蓝（spawn 修正），被动回蓝会被 min(mana_max, ..) 原样夹回
    // 满蓝，故这里恰好也等于 m0——不是巧合，是"已在蓝量上限"这个具体场景
    // 下回蓝无可观测效果，与"空槽无副作用"这条断言并不冲突。
    assert_eq!(sim.projectiles().len(), 0);
    assert_eq!(sim.creatures().get(0).unwrap().mana, m0, "空槽无任何副作用");
}

#[test]
fn mana_regenerates_up_to_max() {
    let mut sim = arena_with_loadout(&["spark_bolt"]);
    sim.creatures_mut().set_mana(0, 0);
    for _ in 0..600 {
        sim.step(&[], &[]);
    }
    let c = sim.creatures().get(0).unwrap();
    let mana_max = sim.creature_table().get(c.template).mana_max;
    assert_eq!(c.mana, mana_max, "10 秒后应当回满且不越界");
}

#[test]
fn slot_selects_which_spell_is_cast() {
    let mut sim = arena_with_loadout(&["spark_bolt", "bomb"]);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, /* slot */ 1)]);
    assert_eq!(sim.projectiles().spell(0), sim.spell_id("bomb"), "应当放 1 号槽的法术");
}

#[test]
fn aim_determines_launch_direction() {
    let mut sim = arena_with_loadout(&["spark_bolt"]); // 测试表里 spread_bam = 0
    sim.step(&[], &[InputFrame::new(BTN_FIRE, /* 90° 向下 */ 16384, 0)]);
    assert!(sim.projectiles().vy(0) > Fx::ZERO && sim.projectiles().vx(0) == Fx::ZERO);
}

#[test]
fn blast_spell_explodes_on_impact_and_carves_terrain() {
    let mut sim = arena_with_loadout(&["bomb"]);
    let stone = sim.table().id_by_name("stone").unwrap();
    sim.apply_setup(&[Op::Fill { material: stone, x0: 60, y0: 40, x1: 70, y1: 90 }]);
    let before = sim.world().count_material(stone);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    for _ in 0..90 {
        sim.step(&[], &[]);
    }
    assert!(sim.world().count_material(stone) < before, "Blast 必须炸出洞");
}

#[test]
fn spray_spell_emits_particles_without_creating_a_projectile() {
    let mut sim = arena_with_loadout(&["oil_spray"]);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    assert_eq!(sim.projectiles().len(), 0, "Spray 不产生弹体");
    assert!(!sim.particles().is_empty(), "Spray 当帧就应产出粒子");
}

/// 评审 Important（2026-09-06）：不变量必须守在 `queue_projectile` 这个 `pub`
/// 入口，不能指望 `Projectiles::advance` 命中判定里的 `resolve_hit` 远端兜底
/// （那条路径原本是 `unreachable!()`，release 下同样 panic）。直接对
/// `Sim::queue_projectile` 喂一个 `Spray` 法术 id：必须确定性拒绝（返回
/// `false`、弹体池长度不变），不能 panic、不能真产出一颗"Spray 弹体"。
#[test]
fn queue_projectile_rejects_spray_spell_at_the_entry() {
    let mut sim = arena_with_loadout(&["oil_spray"]);
    let spray_id = sim.spell_id("oil_spray");
    let ok = sim.queue_projectile(spray_id, Fx::from_int(50), Fx::from_int(64), Fx::ZERO, Fx::ZERO, 255);
    assert!(!ok, "Spray 法术必须在 queue_projectile 入口就被拒绝，不能进弹体池");
    assert_eq!(sim.projectiles().len(), 0, "拒绝必须是零副作用——弹体池不得多出一条");
}

#[test]
fn projectile_spawns_outside_the_shooter_hitbox() {
    // muzzle_offset 保证不在自己身体里出生（否则第一帧就自撞）
    let mut sim = arena_with_loadout(&["spark_bolt"]);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    let c = sim.creatures().get(0).unwrap();
    let dx = (sim.projectiles().x(0).to_cell() - c.x.to_cell()).abs();
    assert!(dx > c.half_w, "出生点必须在自身 AABB 之外");
}
