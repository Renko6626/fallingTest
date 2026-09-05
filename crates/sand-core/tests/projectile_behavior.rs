//! 弹体行为集成测试（M4 Task 4 spec §5、Task 5 spec §6、Task 6 spec §5.1/
//! §5.2/§5.4/§5.5）：经 `Sim` 公共 API 驱动。范围：Task 4 部分——直线飞行、
//! DDA 命中判定（先到者优先：生物 vs 硬格）、grace 防自伤、同队免疫、
//! 出界/寿命耗尽销毁、容量限流、逐 tick 恰好一次积分。Task 5 部分——
//! `cast_all` 双闸门（cooldown + mana）、`Blast`/`Spray` 派发、出射方向
//! （aim + 散布）、出生点偏移。**Task 6 部分**（本文件末尾一段）——排开
//! 液体/粉末、`pass_through` 穿透掩码、`air_friction`/`liquid_drag` 阻力、
//! 定时爆、侵彻（`dig_power` + `max_durability`）、弹跳（`bounces` +
//! `bounce_energy`）、刚体单点冲量（`physics_impulse`），以及"先到者优先"
//! 组合路径（生物先、硬格后）的执法回归。

mod common;

use sand_core::{input::BTN_FIRE, spell::*, Fx, InputFrame, Op, Sim, MAT_WALL, MAX_PROJECTILES};

use common::{arena_with_loadout, spell_table, WATER};

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

/// 评审遗留项（Task 4 评审，Task 6 补齐——见 Task 6 报告"Carried finding"）：
/// 既有测试只覆盖"只撞生物"（`projectile_damages_a_creature_it_hits`）与
/// "只撞硬格"（`projectile_flies_straight_and_dies_on_wall`）两种孤立场景，
/// 从未验证一条路径上**先遇到生物、后面还有硬格**时"先到者优先"是否真的
/// 按遇到顺序生效。`advance` 循环体的控制流（每格先测生物、`break 'walk`
/// 立即终止）结构性地保证了这一点，但 Task 6 往这同一个循环里插了穿透/
/// 排开/侵彻/弹跳四个新分支——这条路径测试现在才补上，正是为了在改动
/// 那一刻就把"生物优先"钉死，不依赖"看代码觉得像是对的"。
#[test]
fn projectile_prioritizes_a_creature_over_a_wall_further_down_the_path() {
    let mut sim = common::arena_with_two_creatures(bolt_table()); // id0 x=20, id1 x=200，队伍不同
    let wall = sim.table().id_by_name("wall").unwrap();
    // 墙夹在 id0 与 id1 之间：路径上先遇到 id0，再遇到墙——若"先到者优先"
    // 判错顺序，或生物判定被 Task 6 新插入的分支意外短路，子弹会穿过 id0
    // 直接撞墙，id0 不掉血。
    sim.apply_setup(&[Op::Fill { material: wall, x0: 100, y0: 55, x1: 102, y1: 75 }]);
    let hp0 = sim.creatures().get(0).unwrap().hp;
    shoot(&mut sim, 0, 10, 64, Fx::from_int(8), Fx::ZERO, 255);
    for _ in 0..40 {
        sim.step(&[], &[]);
    }
    assert!(sim.creatures().get(0).unwrap().hp < hp0, "路径上生物在前、硬格在后时必须优先命中生物");
    assert_eq!(sim.projectiles().len(), 0, "命中生物后弹体销毁，不会继续飞到墙那");
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

/// **几何在 M4 Task 6 落地时改过一次**（原版水平射向 x60-70 的石块侧面）：
/// `bomb` 的 `bounces: 2` 字段是 Task 5 就写进 `test_spell_table` 但**一直
/// 未被消费**的中性缺省——Task 6 首次让 `advance` 读它，水平弹道命中侧面
/// 会先弹开（X 轴反射），带着残余水平速度飘回射手附近，`grace`（20 帧）
/// 耗尽后正好撞上自己的射手，爆炸落点因此在射手脚下而不是石块里，`stone`
/// 计数纹丝不动（实测撞见，不是纸面推测：见 Task 6 报告"根 causes"一节）。
/// 改成**垂直下落砸石块顶面**：`vx ≈ 0` 时每次弹跳都近乎原地起落，重力把
/// 它拽回同一列，最终（第 3 次命中，`bounces` 耗尽）几乎必然砸在同一个
/// 洞口——与水平弹道相比对"会不会飘走"免疫，是本测试要验证的东西（"Blast
/// 命中即炸"）本身该有的鲁棒性，不是绕开 bug 的权宜之计。
#[test]
fn blast_spell_explodes_on_impact_and_carves_terrain() {
    let mut sim = arena_with_loadout(&["bomb"]);
    let stone = sim.table().id_by_name("stone").unwrap();
    sim.apply_setup(&[Op::Fill { material: stone, x0: 15, y0: 75, x1: 25, y1: 120 }]);
    let before = sim.world().count_material(stone);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, /* 90° 向下 */ 16384, 0)]);
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

// ==================== M4 Task 6：弹体七项扩展（spec §5.1/§5.2/§5.4/§5.5） ====================
//
// 逐条 TDD，brief 排序（cheapest-first）：排开 → 穿透 → 阻力 → 定时爆 →
// 侵彻 → 弹跳 → 冲量。测试法术全部来自 `common::test_spell_table`
// （`arena_with_loadout` 路径）或 `common::spell_table`（`arena_wide_open` +
// 直接 `queue_projectile` 路径）——两张表互相独立、同名不同值不是笔误
// （体例见 `common::spell_table` 头注）。

/// **测量窗口在 M4 Task 6 落地时从 30 tick 收窄到 12 tick**：水池是连通体，
/// 排开的那一两格一旦脱格成粒子、又在附近落回，池面很快抹平重新填满
/// （TDD 阶段实测：本例大水池在 tick8 排开 2 格、tick22 前已回填干净）——
/// 30 tick 测的其实是"排开效果是否**已经消失**"，不是"排开有没有发生"。
/// 12 tick 卡在"刚排开、还没回填"的窗口内（实测 8–21 tick 区间水量/粒子数
/// 都还看得见差异），断言测的才是这条特性字面宣称的东西。
#[test]
fn displacing_projectile_pushes_liquid_out_of_its_path() {
    let mut sim = arena_with_loadout(&["bomb"]); // displace_liquid: true
    sim.apply_setup(&[Op::Fill { material: WATER, x0: 50, y0: 60, x1: 60, y1: 90 }]);
    let before = sim.world().count_material(WATER);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    for _ in 0..12 {
        sim.step(&[], &[]);
    }
    assert!(
        sim.world().count_material(WATER) < before || !sim.particles().is_empty(),
        "飞过水面应当把水推成粒子"
    );
}

#[test]
fn pass_through_liquid_lets_the_projectile_cross_a_pool() {
    let mut sim = arena_with_loadout(&["digger"]); // pass_through: gas + liquid
    sim.apply_setup(&[Op::Fill { material: WATER, x0: 40, y0: 55, x1: 45, y1: 75 }]);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    for _ in 0..20 {
        sim.step(&[], &[]);
    }
    assert!(
        sim.projectiles().len() == 1 && sim.projectiles().x(0).to_cell() > 45,
        "穿液体的弹体应当越过水池"
    );
}

#[test]
fn air_friction_below_one_decelerates_the_projectile() {
    let mut sim = arena_with_loadout(&["slow_bolt"]); // air_friction: 0.9
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    sim.step(&[], &[]);
    let v1 = sim.projectiles().vx(0);
    sim.step(&[], &[]);
    assert!(sim.projectiles().vx(0) < v1, "air_friction < 1 应当减速");
}

#[test]
fn liquid_drag_slows_a_projectile_inside_water_more_than_in_air() {
    // 两发独立开局，避免互相干扰：同法术同初速，一发穿空气、一发穿水池。
    let travel = |flood: bool| {
        let mut sim = common::arena_wide_open(spell_table());
        if flood {
            sim.apply_setup(&[Op::Fill { material: WATER, x0: 10, y0: 40, x1: 240, y1: 90 }]);
        }
        let wet_bolt = sim.spell_id("wet_bolt");
        shoot(&mut sim, wet_bolt, 12, 64, Fx::from_int(6), Fx::ZERO, 255);
        let x0 = sim.projectiles().x(0);
        for _ in 0..20 {
            sim.step(&[], &[]);
        }
        assert_eq!(sim.projectiles().len(), 1, "20 tick 内不该撞到东西");
        sim.projectiles().x(0) - x0
    };
    assert!(travel(true) < travel(false), "水里应当飞得更近");
}

/// 场景在 M4 Task 6 落地时改过一次——原版水平抛射向石块中段，`bomb` 的
/// `bounces: 2` 一旦真被消费，命中侧面会先弹开、飘出老远，180 tick 的寿命
/// 早就在别处（甚至世界边界）耗尽，断言随机落空（TDD 阶段实测撞见）。
/// 改成静止出生 + 短寿命（`timed_bomb`，`life: 5`）：石块顶面摆在出生点
/// 正下方 10 格，5 tick 的重力累积下坠只有 3.75 格，寿命耗尽那一刻确定性
/// 地仍悬在半空、从未进入过命中判定分支，但落在爆炸半径（12）内——"寿命
/// 耗尽也要炸"这条断言因此测的就是它字面宣称的那件事，不掺任何"顺便撞上
/// 什么"的偶然性。
#[test]
fn timed_blast_explodes_when_lifetime_runs_out_even_without_hitting() {
    let mut sim = common::arena_wide_open(spell_table());
    let stone = sim.table().id_by_name("stone").unwrap();
    sim.apply_setup(&[Op::Fill { material: stone, x0: 55, y0: 60, x1: 75, y1: 100 }]);
    let before = sim.world().count_material(stone);
    let timed_bomb = sim.spell_id("timed_bomb");
    shoot(&mut sim, timed_bomb, 65, 50, Fx::ZERO, Fx::ZERO, 255);
    for _ in 0..10 {
        sim.step(&[], &[]);
    }
    assert_eq!(sim.projectiles().len(), 0, "寿命耗尽即销毁");
    assert!(sim.world().count_material(stone) < before, "寿命耗尽也要炸");
}

#[test]
fn digger_bores_into_stone_and_stops_when_energy_is_spent() {
    let mut sim = arena_with_loadout(&["digger"]);
    let stone = sim.table().id_by_name("stone").unwrap();
    sim.apply_setup(&[Op::Fill { material: stone, x0: 50, y0: 0, x1: 90, y1: 127 }]);
    let before = sim.world().count_material(stone);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    for _ in 0..60 {
        sim.step(&[], &[]);
    }
    let dug = before - sim.world().count_material(stone);
    assert!(dug > 0, "挖掘弹必须挖穿一段");
    // 射手 y=65（`arena_with_loadout` 固定出生点），digger 无重力、水平飞行，
    // 侵彻沿途都在这一行。
    assert!(sim.world().cell(89, 65).material() == stone, "能量有限，不得挖穿整堵墙");
    assert_eq!(sim.projectiles().len(), 0, "能量耗尽即销毁");
}

#[test]
fn wall_durability_gate_stops_the_digger_immediately() {
    // wall durability 15 > digger 的 max_durability 12 ⇒ 一格都挖不动。
    let mut sim = arena_with_loadout(&["digger"]);
    sim.apply_setup(&[Op::Fill { material: MAT_WALL, x0: 50, y0: 0, x1: 60, y1: 127 }]);
    let before = sim.world().count_material(MAT_WALL);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    for _ in 0..60 {
        sim.step(&[], &[]);
    }
    assert_eq!(sim.world().count_material(MAT_WALL), before, "门槛免疫，一格都不掉");
}

#[test]
fn bouncing_projectile_reflects_off_the_floor_and_dies_after_its_last_bounce() {
    let mut sim = arena_with_loadout(&["bomb"]); // bounces: 2
    sim.step(&[], &[InputFrame::new(BTN_FIRE, /* 45° 下斜 */ 8192, 0)]);
    let mut sign_flips = 0;
    let mut prev = sim.projectiles().vy(0);
    for _ in 0..400 {
        sim.step(&[], &[]);
        if sim.projectiles().is_empty() {
            break;
        }
        let v = sim.projectiles().vy(0);
        if prev > Fx::ZERO && v < Fx::ZERO {
            sign_flips += 1;
        }
        prev = v;
    }
    assert_eq!(sign_flips, 2, "应当恰好弹 2 次");
    assert_eq!(sim.projectiles().len(), 0, "弹完即销毁（bomb 会在此炸开）");
}

/// **`bomb`（Step 6）与 `bomb`（Step 3 用的 `spell_table()`）同名不同值**：
/// 后者的 `gravity` 特意取 0.1（`common::spell_table` 头注有完整推导）——
/// spec §5.1 的顺序（`vy += gravity` 先于碰撞判定）意味着"反弹前速度"天然
/// 多算了这一 tick 的重力增量，`gravity × bounce_energy` 必须小于本测试的
/// 容差 `1/16`；`test_spell_table` 里配 `arena_with_loadout` 用的那个
/// `bomb`（gravity 0.25）不满足这条，只用于"弹几次就死"这种不看精确数值
/// 的断言（上面那条测试）。
#[test]
fn bounce_energy_reduces_speed_each_time() {
    // 每次反弹后该轴速度大小 ≈ 前一次 × bounce_energy（容差 1/16 格）。
    let mut sim = common::arena_wide_open(spell_table());
    let bomb = sim.spell_id("bomb");
    shoot(&mut sim, bomb, 40, 20, Fx::ZERO, Fx::from_int(4), 255);
    let mut speeds = Vec::new();
    let mut prev = sim.projectiles().vy(0);
    for _ in 0..400 {
        sim.step(&[], &[]);
        if sim.projectiles().is_empty() {
            break;
        }
        let v = sim.projectiles().vy(0);
        if prev > Fx::ZERO && v < Fx::ZERO {
            speeds.push((prev, v)); // (撞前向下速度, 反弹后向上速度)
        }
        prev = v;
    }
    assert_eq!(speeds.len(), 2, "bomb 的 bounces = 2");
    let e = Fx::from_ratio(4, 10); // bounce_energy 0.4
    let tol = Fx::from_ratio(1, 16);
    for (before, after) in speeds {
        let want = before.mul(e);
        let got = Fx(-after.0); // 取绝对值（反弹后是负的）
        assert!((got - want).0.abs() < tol.0, "反弹衰减不符：{got:?} vs {want:?}");
    }
}

#[test]
fn projectile_pushes_a_rigid_body_it_hits() {
    let mut sim = arena_with_loadout(&["expensive_bolt"]); // physics_impulse: 0.3
    let wood = sim.table().id_by_name("wood").unwrap();
    sim.apply_setup(&[Op::SpawnBody { material: wood, x: 60, y: 60, w: 12, h: 12, angle_deg: 0 }]);
    let x0 = sim.body_state(0).unwrap().0 .0;
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    for _ in 0..60 {
        sim.step(&[], &[]);
    }
    assert!(sim.body_state(0).unwrap().0 .0 > x0, "射中的箱子应当被推走");
}
