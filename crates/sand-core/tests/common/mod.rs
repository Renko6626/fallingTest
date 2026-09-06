//! 测试公用：内联材料表（core 测试不读 data/，保持 crate 自包含）。

use sand_core::{
    input::MAX_SLOTS,
    spell::{SpellDef, SpellKind, SPELL_NONE},
    Category, CreatureTable, Fx, InitConfig, MaterialDef, MaterialTable, Op, ReactionTable, ScanMode, Sim,
    SpellTable,
};

pub const SAND: u8 = 2;
pub const WATER: u8 = 3;
/// `materials()`（M4 Task 2/3 用表）里 `fire` 的固定 id——见该函数文档，
/// `CreatureTable::default_player()` 的 `damage_from` 与此耦合。
#[allow(dead_code)]
pub const FIRE: u8 = 5;

/// 自定义 water 色散距离的材料表（Layer G Task 1，spec §3）。**刻意绕过
/// harness 的加载期校验**——core 侧的 clamp 是 P4 写域论证的最后防线
/// （spec §3.1 评审修订），必须能被直接构表的测试打到。
// 只被 rules_behavior 用；common 被三个测试二进制各编一份，其余两个看不到
// 调用点（与既有 test_table 的注释同一处境）。
#[allow(dead_code)]
pub fn test_table_with_water_dispersion(dispersion: u8) -> MaterialTable {
    let def = |id: u8, name: &str, category: Category, density: u16, hp: u32, disp: u8| {
        MaterialDef { hp, dispersion: disp, ..MaterialDef::base(id, name, category, density) }
    };
    MaterialTable::new(vec![
        def(0, "air", Category::Static, 0, 0, 1),
        MaterialDef { hp: 100, durability: 15, ..MaterialDef::base(1, "wall", Category::Static, 100) },
        def(SAND, "sand", Category::Powder, 40, 2, 1),
        def(WATER, "water", Category::Liquid, 16, 1, dispersion),
    ])
    .unwrap()
}

/// 用指定材料表建 Sim（色散测试专用）。
#[allow(dead_code)]
pub fn sim_with_table(
    width_chunks: usize,
    height_chunks: usize,
    seed: u64,
    threads: usize,
    scan: ScanMode,
    table: MaterialTable,
) -> Sim {
    let cfg = InitConfig { width_chunks, height_chunks, seed, threads, scan };
    let reactions = ReactionTable::empty(&table);
    Sim::new(&cfg, table, reactions, CreatureTable::empty(), SpellTable::empty()).unwrap()
}

/// 带反应表建 Sim（M2 Task 2 反应行为测试用）。
#[allow(dead_code)]
pub fn sim_with_reactions(
    width_chunks: usize,
    height_chunks: usize,
    seed: u64,
    threads: usize,
    scan: ScanMode,
    table: MaterialTable,
    reactions: ReactionTable,
) -> Sim {
    let cfg = InitConfig { width_chunks, height_chunks, seed, threads, scan };
    Sim::new(&cfg, table, reactions, CreatureTable::empty(), SpellTable::empty()).unwrap()
}

/// 自定义 splash_chance 的材料表（Layer G Task 3，spec §6）。`chance` 是
/// **量化后的 u8**（0 = 永不溅射、255 = 必溅射），与 harness 的 `×255 round`
/// 量化域同口径——core 边界只见整数。water 额外给足色散，让"下方被挡 → 色散
/// 走开"（`MovedSide`）这条路径可达，它正是 §6.1① 要观察的触发面。
#[allow(dead_code)]
pub fn test_table_with_splash(water_chance: u8, sand_chance: u8) -> MaterialTable {
    let def = |id: u8, name: &str, category: Category, density: u16, splash: u8, disp: u8| {
        MaterialDef { splash_chance: splash, dispersion: disp, ..MaterialDef::base(id, name, category, density) }
    };
    MaterialTable::new(vec![
        def(0, "air", Category::Static, 0, 0, 1),
        def(1, "wall", Category::Static, 100, 0, 1),
        def(SAND, "sand", Category::Powder, 40, sand_chance, 1),
        def(WATER, "water", Category::Liquid, 16, water_chance, 5),
    ])
    .unwrap()
}

#[allow(dead_code)]
pub fn test_table() -> MaterialTable {
    // hp 取 spec §6 的口径值（air 0 / water 1 / sand 2），wall 走门槛免疫。
    // 爆炸行为测试并未独立成 `explode_behavior.rs` 文件，而是内联在
    // `crates/sand-core/src/world.rs` 的 `#[cfg(test)] mod tests` 里——本表
    // 供本文件之外、复用同一材料语义的测试模块直接调用。
    // dispersion 全取缺省 1：本表是"改动前语义"的基线，既有测试与 golden
    // 的逐位不变性由它守着（spec §3.4 缺省行为条）。
    let def = |id: u8, name: &str, category: Category, density: u16, hp: u32| MaterialDef {
        hp,
        ..MaterialDef::base(id, name, category, density)
    };
    MaterialTable::new(vec![
        def(0, "air", Category::Static, 0, 0),
        MaterialDef { hp: 100, durability: 15, ..MaterialDef::base(1, "wall", Category::Static, 100) },
        def(SAND, "sand", Category::Powder, 40, 2),
        def(WATER, "water", Category::Liquid, 16, 1),
    ])
    .unwrap()
}

/// 基线表 + 一种气体（M2 Task 1，气体行为测试用）：smoke，Gas，密度 2。
// 只被 rules_behavior 用；common 被多个测试二进制各编一份（同上）。
#[allow(dead_code)]
pub const SMOKE: u8 = 4;

#[allow(dead_code)]
pub fn test_table_with_gas() -> MaterialTable {
    let def = |id: u8, name: &str, category: Category, density: u16| MaterialDef::base(id, name, category, density);
    MaterialTable::new(vec![
        def(0, "air", Category::Static, 0),
        def(1, "wall", Category::Static, 100),
        def(SAND, "sand", Category::Powder, 40),
        def(WATER, "water", Category::Liquid, 16),
        def(SMOKE, "smoke", Category::Gas, 2),
    ])
    .unwrap()
}

#[allow(dead_code)]
pub fn sim(width_chunks: usize, height_chunks: usize, seed: u64, threads: usize, scan: ScanMode) -> Sim {
    let cfg = InitConfig { width_chunks, height_chunks, seed, threads, scan };
    let table = test_table();
    let reactions = ReactionTable::empty(&table);
    Sim::new(&cfg, table, reactions, CreatureTable::empty(), SpellTable::empty()).unwrap()
}

/// 基线表 + wood + fire（M4 Task 2/3，生物行为测试用）：air/wall/sand/water 同
/// `test_table`，追加 `wood`（Static，供 `Op::SpawnBody` 验证"刚体盖章格对生物
/// 即地形"）与 `fire`（Gas + `lifetime`）。
/// id 不写死在调用点——测试经 `table.id_by_name(..)` 取（R6）。
///
/// **`fire` 固定落在 id 5**：`CreatureTable::default_player()`（`creature.rs`）
/// 的 `damage_from` 硬编码了这个 id（该函数不接收 `MaterialTable`，查不了
/// 名字）——挪动这张表里 `fire` 的位置必须同步改那边的注释与数值。
///
/// **`rise_chance: 0`，故意不照抄 `data/materials.ron` 的 0.5**：本表只服务
/// 接触伤害/`min_cell_count` 门槛测试，不测气体上浮本身（那是 M2 的地盘）。
/// 缺省/生产口径的 0.5 会让火格随机漂移+扩散，"2 格火反复补给 3600 tick"
/// 这类测试因此会在概率意义上偶发触发"漂出去的旧火格恰好和补给的新火格
/// 同时落在生物 AABB 里、瞬时凑够 4 格"的假阳性——`0` 让火格判定
/// `rng_u32(..) % 255 >= 0` 恒真、`gas_step` 直接原地不动（`rules.rs::gas_step`
/// 文档），把"接触格数"钉死成 `Op::Fill` 显式填的那几格，测试因此是结构性
/// 确定的，不是"多数情况下大概率不撞"。
/// M4 Task 5 追加 `oil`（id 6，Liquid）、`stone`（id 7，Static，durability
/// 8）、`stone_debris`（id 8，Powder）——**追加在表尾**，不挪动既有 id
/// （`FIRE = 5` 的硬编码耦合、`CreatureTable::default_player()` 的
/// `damage_from` 都锚定在旧 id 上，插进中间会全部错位）。`oil` 供
/// `oil_spray` 测试法术（`test_spell_table`）的 `Spray::material` 用；
/// `stone` 供 `blast_spell_explodes_on_impact_and_carves_terrain` 当
/// "炸得动的墙"用（`durability: 8` 与 `data/materials.ron` 的生产值同
/// 口径，`bomb` 测试法术的 `max_durability: 10` 能打穿它）。
///
/// **`stone.debris_to` 必须显式指向 `stone_debris`，不能吃 `MaterialDef::
/// base` 的缺省（缺省 = 自身）**：这条是 TDD 阶段实测撞见的——`stone`
/// 若缺省"碎屑即自身"，`apply_explode` 摧毁的格子会以材质 `stone` 的粒子
/// 形态脱格，重力落回原地/邻近格后照样重新凝固成 `stone`，90 tick 后
/// `world.count_material(stone)` 净变化几乎为零，把"确实炸穿了"的断言
/// 蒙混过去（第一版实现正是这样悄悄失败：中心格瞬间被炸空，但整体计数
/// 未变）。生产表 `data/materials.ron` 本就把 `stone` 的碎屑指到独立的
/// `stone_debris`（Powder），这里对齐同一约定，而不是走缺省值。
#[allow(dead_code)]
pub fn materials() -> MaterialTable {
    let def = |id: u8, name: &str, category: Category, density: u16, hp: u32| MaterialDef {
        hp,
        ..MaterialDef::base(id, name, category, density)
    };
    MaterialTable::new(vec![
        def(0, "air", Category::Static, 0, 0),
        MaterialDef { hp: 100, durability: 15, ..MaterialDef::base(1, "wall", Category::Static, 100) },
        def(SAND, "sand", Category::Powder, 40, 2),
        def(WATER, "water", Category::Liquid, 16, 1),
        def(4, "wood", Category::Static, 12, 10),
        MaterialDef {
            lifetime: 40,
            fire_temp: 100,
            rise_chance: 0,
            ..MaterialDef::base(FIRE, "fire", Category::Gas, 1)
        },
        // `oil` 可燃（M4 Task 7 Step 2 追加，`oil_spray_then_bolt_ignites_a_chain`
        // 需要）：此前本表的 `oil` 只用 `def(..)` 走最简闭包，只带 `hp`，
        // `fire_hp` 吃 `MaterialDef::base` 缺省 0——**不可燃**，`fire.rs::
        // try_ignite` 的 `fuel == 0` 分支恒真、直接 `return`，火从来点不着
        // 它（TDD 阶段实测撞见：`fire_bolt` 落地后 fire 计数如期在寿命 40
        // tick 后归零，但 oil 计数全程纹丝不动）。数值照抄
        // `data/materials.ron` 的生产值（`ignition_temp: 40`、
        // `fire_temp: 100`、`fire_hp: 90`、`fire_chance` 量化自 0.6、
        // `flame_to` 指向本表的 `FIRE`）——此前只有 `stone`/`wood_debris`
        // 这类可燃材质在本表里显式声明过这套字段，`oil` 现在补齐同一套。
        MaterialDef {
            hp: 1,
            dispersion: 2,
            ignition_temp: 40,
            fire_temp: 100,
            fire_hp: 90,
            fire_chance: 153, // ×255 round(0.6) = 153，同 data/materials.ron 口径
            flame_to: FIRE,
            ..MaterialDef::base(6, "oil", Category::Liquid, 12)
        },
        MaterialDef {
            hp: 6,
            durability: 8,
            debris_to: 8, // 显式指向 stone_debris（见函数文档"必须显式指向"那段）。
            ..MaterialDef::base(7, "stone", Category::Static, 40)
        },
        def(8, "stone_debris", Category::Powder, 40, 2),
    ])
    .unwrap()
}

/// 4×2 chunk（256×128）的世界，底行 wall；在 (32, 100) 放一个 controller 0 的生物
/// （M4 Task 2 生物行为测试用，spec `creature_behavior.rs` Step 4）。
///
/// **M4 Task 4 签名变更**：追加 `spells: SpellTable` 形参——`Sim::new` 自 Task 1
/// 起就要求法术表，本 helper 之前一直悄悄塞 `SpellTable::empty()`，Task 4 的
/// 弹体测试需要一张非空表，索性把它交给调用方（"helper 一次定型，后续 Task
/// 只加参数不改语义"，本文件 Task 4 brief Step 3）。既有调用点（Task 2/3 的
/// `creature_behavior.rs::floor_world`）同步改传 `SpellTable::empty()`，语义
/// 不变。
#[allow(dead_code)]
pub fn floor_world_with_creature(tbl: CreatureTable, spells: SpellTable) -> (Sim, u8) {
    let table = materials();
    let wall = table.id_by_name("wall").unwrap();
    let cfg = InitConfig { width_chunks: 4, height_chunks: 2, seed: 42, threads: 1, scan: ScanMode::LiveRect };
    let reactions = ReactionTable::empty(&table);
    let mut sim = Sim::new(&cfg, table, reactions, tbl, spells).unwrap();
    sim.apply_setup(&[
        Op::Fill { material: wall, x0: 0, y0: 127, x1: 255, y1: 127 },
        Op::SpawnCreature { x: 32, y: 100, template: 0, team: 0, controller: 0, loadout: [255; MAX_SLOTS] },
    ]);
    (sim, 0)
}

/// 256×128 空场（4×2 chunk），四周 wall 一圈；无生物（M4 Task 4 弹体行为测试
/// 用，spec `projectile_behavior.rs`）。法术表由调用方给——core 侧程序化构表
/// （`SpellTable::from_defs`），不依赖 `spells.ron`。
#[allow(dead_code)]
pub fn arena_wide_open(spells: SpellTable) -> Sim {
    let table = materials();
    let wall = table.id_by_name("wall").unwrap();
    let cfg = InitConfig { width_chunks: 4, height_chunks: 2, seed: 42, threads: 1, scan: ScanMode::LiveRect };
    let reactions = ReactionTable::empty(&table);
    let mut sim = Sim::new(&cfg, table, reactions, CreatureTable::empty(), spells).unwrap();
    sim.apply_setup(&[
        Op::Fill { material: wall, x0: 0, y0: 0, x1: 255, y1: 0 },
        Op::Fill { material: wall, x0: 0, y0: 127, x1: 255, y1: 127 },
        Op::Fill { material: wall, x0: 0, y0: 0, x1: 0, y1: 127 },
        Op::Fill { material: wall, x0: 255, y0: 0, x1: 255, y1: 127 },
    ]);
    sim
}

/// 两个生物的弹体命中测试场地（M4 Task 4）：id 0 在 (20,64) team `team0`、
/// id 1 在 (200,64) team `team1`，两者 controller 均为 255（不吃输入）。
///
/// **不是简单"`arena_wide_open` + 两条 `Op::SpawnCreature`"**：`arena_wide_open`
/// 内部除了四周边框墙一无所有，生物出生在 y=64 会自由落体几十格才落到
/// y=127 的世界边界，重力累积几个 tick 就能把生物的 AABB 甩出弹体水平
/// 飞行的那一行——"命中生物"系列测试因此会全体假阴性（实测验证过，不是
/// 纸面推测）。这里额外铺一条紧贴生物脚下的地板（y=70，生物 `half_h=5`、
/// 出生 y=64 → 脚跟 69，第一 tick 重力介入时几乎不下坠），把生物钉在
/// 弹体飞行的 y=64 附近，同时仍然验证的是"生物落地静止"这一正常状态，
/// 不是靠关掉重力作弊。
fn two_creature_arena(spells: SpellTable, team0: u8, team1: u8) -> Sim {
    let table = materials();
    let wall = table.id_by_name("wall").unwrap();
    let cfg = InitConfig { width_chunks: 4, height_chunks: 2, seed: 42, threads: 1, scan: ScanMode::LiveRect };
    let reactions = ReactionTable::empty(&table);
    let tpl = CreatureTable::default_player();
    let mut sim = Sim::new(&cfg, table, reactions, tpl, spells).unwrap();
    sim.apply_setup(&[
        Op::Fill { material: wall, x0: 0, y0: 70, x1: 255, y1: 70 },
        Op::SpawnCreature { x: 20, y: 64, template: 0, team: team0, controller: 255, loadout: [255; MAX_SLOTS] },
        Op::SpawnCreature { x: 200, y: 64, template: 0, team: team1, controller: 255, loadout: [255; MAX_SLOTS] },
    ]);
    sim
}

/// `two_creature_arena`，两个生物分属 team 0 / team 1（跨队命中测试用）。
#[allow(dead_code)]
pub fn arena_with_two_creatures(spells: SpellTable) -> Sim {
    two_creature_arena(spells, 0, 1)
}

/// `two_creature_arena`，两个生物同属 team 0（同队免疫测试用）。
#[allow(dead_code)]
pub fn arena_with_two_creatures_same_team(spells: SpellTable) -> Sim {
    two_creature_arena(spells, 0, 0)
}

// ==================== M4 Task 5：施法测试专用地与法术表（R11） ====================

/// 施法测试专用法术表——**本 Task 自建，不依赖 `data/spells.ron`**（core
/// 测试不读 `data/`，文件头注）。四条与 `data/spells.ron` 同名，但两处
/// **刻意偏离**生产数值：
///
/// 1. **全部 `spread_bam = 0`**（`spark_bolt` 生产值 `spread_deg: 2.0`）：
///    `aim_determines_launch_direction` 要求出射方向对瞄准角**精确**相等
///    （断言 `vx == Fx::ZERO`），非零散布骰会偶发引入偏转，与"测试必须
///    结构性确定、不是多数情况下大概率成立"的红线矛盾——同
///    `common::materials()` 里 `fire.rise_chance` 特意取 0 而非生产的 0.5
///    同一先例（该函数文档已有说明）。
/// 2. 其余字段仅取"够测"的量级（`max_durability`/`air_friction`/
///    `liquid_drag` 等 Task 6 才消费的字段随手给个合理默认），不追求与
///    `data/spells.ron` 逐位一致。
#[allow(dead_code)]
fn test_spell_table(table: &MaterialTable) -> SpellTable {
    let oil = table.id_by_name("oil").unwrap();
    let fire = table.id_by_name("fire").unwrap();
    SpellTable::from_defs(vec![
        SpellDef {
            name: "spark_bolt".to_string(),
            kind: SpellKind::Bolt { damage_milli: 5_000, knockback: Fx::from_int(2) },
            mana: 10_000,
            cooldown: 12,
            speed: Fx::from_int(8),
            life: 120,
            gravity: Fx::ZERO,
            spread_bam: 0, // 见函数文档第 1 条——刻意不同于生产值 2.0°。
            grace: 4,
            dig_power: 0,
            max_durability: 10,
            air_friction: Fx::from_int(1),
            liquid_drag: Fx::from_ratio(9, 10),
            pass_through: 0,
            displace_liquid: false,
            bounces: 0,
            bounce_energy: Fx::from_ratio(5, 10),
            physics_impulse: 0,
            on_lifetime_out_explode: false,
        },
        SpellDef {
            name: "bomb".to_string(),
            kind: SpellKind::Blast { power: 1200, radius: 12, max_durability: 10 },
            mana: 35_000,
            cooldown: 60,
            speed: Fx::from_int(5),
            life: 180,
            gravity: Fx::from_ratio(1, 4),
            spread_bam: 0,
            grace: 20,
            dig_power: 0,
            max_durability: 10,
            air_friction: Fx::from_int(1),
            liquid_drag: Fx::from_ratio(8, 10),
            pass_through: 0,
            displace_liquid: true,
            bounces: 2,
            bounce_energy: Fx::from_ratio(4, 10),
            physics_impulse: 0,
            on_lifetime_out_explode: true,
        },
        SpellDef {
            name: "oil_spray".to_string(),
            kind: SpellKind::Spray { material: oil, count: 12, speed: Fx::from_int(4), jitter: Fx::from_ratio(6, 10) },
            mana: 8_000,
            cooldown: 6,
            speed: Fx::ZERO, // 顶层 speed：Spray 分支不读它（用 kind 内的 speed）。
            life: 0,
            gravity: Fx::ZERO,
            spread_bam: 0,
            grace: 0,
            dig_power: 0,
            max_durability: 10,
            air_friction: Fx::from_int(1),
            liquid_drag: Fx::from_int(1),
            pass_through: 0,
            displace_liquid: false,
            bounces: 0,
            bounce_energy: Fx::ZERO,
            physics_impulse: 0,
            on_lifetime_out_explode: false,
        },
        // `physics_impulse: 300`（= 系数 0.3）——**M4 Task 6 TDD 阶段从
        // Task 5 的占位值 20.0 往下调过**：`Bodies::apply_projectile_impulse`
        // 的公式（`body.rs` 文档）与 `apply_blast` 同款"Fx → 引擎格/秒"边界
        // 换算（×60），系数 20 会让单点冲量 = 20 × 10 × 60 = 12000——对一个
        // 12×12 木箱这么小的刚体，这个量级会在一两 tick 内把箱子推到打穿
        // 世界边界、被墙弹回来，观测到的反而是净位移变负（TDD 阶段实测撞见，
        // 不是纸面推演：见 Task 6 报告"物理稳定性"一节）。0.3 是实测验证过
        // 的稳定值——`projectile_pushes_a_rigid_body_it_hits` 断言的 60 tick
        // 窗口内位移方向正确、幅度不炸螺。`data/spells.ron` 同步调整（同一个
        // 参数第一次被真正消费，两处都算"新定值"而非"改既定值"）。
        SpellDef {
            name: "expensive_bolt".to_string(),
            kind: SpellKind::Bolt { damage_milli: 30_000, knockback: Fx::from_int(6) },
            mana: 90_000,
            cooldown: 90,
            speed: Fx::from_int(10),
            life: 120,
            gravity: Fx::ZERO,
            spread_bam: 0,
            grace: 4,
            dig_power: 0,
            max_durability: 10,
            air_friction: Fx::from_int(1),
            liquid_drag: Fx::from_ratio(9, 10),
            pass_through: 0,
            displace_liquid: false,
            bounces: 0,
            bounce_energy: Fx::ZERO,
            physics_impulse: 300,
            on_lifetime_out_explode: false,
        },
        // M4 Task 6 追加：`digger` 供 pass_through（Step 2）与侵彻（Step 5）
        // 两组测试共用同一条法术——两组测试撞的都是 Static 材质（stone/wall），
        // `pass_through` 只对 Gas/Liquid 生效，互不干扰。**`dig_power` 刻意
        // 取比 `data/spells.ron` 生产值（900）小得多的 90**：生产值配
        // `stone.hp=6` 够打穿 150 格，而
        // `digger_bores_into_stone_and_stops_when_energy_is_spent` 的石块只有
        // 41 列宽（x 50..90），会被生产 `dig_power` 一口气打穿，让"能量有限，
        // 不得挖穿整堵墙"这句断言落空——测试表与生产表刻意偏离数值是本文件
        // 一贯体例（见 `test_spell_table` 头注第 2 条），这里是同一先例。
        // `90 / stone.hp(6) = 15` 格，远小于 41 列，两条断言都稳稳成立。
        SpellDef {
            name: "digger".to_string(),
            kind: SpellKind::Bolt { damage_milli: 1_000, knockback: Fx::ZERO },
            mana: 15_000,
            cooldown: 20,
            speed: Fx::from_int(6),
            life: 90,
            gravity: Fx::ZERO,
            spread_bam: 0,
            grace: 4,
            dig_power: 90,
            max_durability: 12,
            air_friction: Fx::from_int(1),
            liquid_drag: Fx::from_int(1),
            pass_through: Category::Gas.bit() | Category::Liquid.bit(),
            displace_liquid: false,
            bounces: 0,
            bounce_energy: Fx::ZERO,
            physics_impulse: 0,
            on_lifetime_out_explode: false,
        },
        // `fire_bolt`（M4 Task 7 Step 2，供 `oil_spray_then_bolt_ignites_a_chain`
        // 端到端测试专用）："打一发火弹点燃"字面上得是 `Bolt`/`Blast` 才像
        // 一发弹体，但两者都碰不到材质：`Bolt` 只对生物 `apply_hit` 扣血，
        // `Blast` 只按 durability 把命中格摧毁成 air（`explode.rs::destroy_cell`
        // 走的是 `set_cell_stamped(.., MAT_AIR, ..)`），核心里**没有一条路径
        // 会把命中格换成 `fire`**——决策记录第 9 条明确"`create_cell_material`
        // 缺口不在 M4 补"。唯一能把 `fire` 材质真正放进世界的原语是
        // `Spray`（`cast_all` 直接 `emit::apply_emit(material, ..)`），
        // 这条法术名字叫 `fire_bolt`（贴合测试名与 duel.ron 头注的叙事），
        // 但 `kind` 是 `Spray(material: fire, ..)`，不是 `Bolt`——与
        // `oil_spray` 同一原语，唯一区别是喷的材质。`count`/`speed`/`jitter`
        // 与 `oil_spray` 同量级，保证喷出的火面积和喷油面积可比，命中率
        // 不靠运气。
        SpellDef {
            name: "fire_bolt".to_string(),
            kind: SpellKind::Spray { material: fire, count: 12, speed: Fx::from_int(4), jitter: Fx::from_ratio(6, 10) },
            mana: 0,
            cooldown: 0,
            speed: Fx::ZERO,
            life: 0,
            gravity: Fx::ZERO,
            spread_bam: 0,
            grace: 0,
            dig_power: 0,
            max_durability: 10,
            air_friction: Fx::from_int(1),
            liquid_drag: Fx::from_int(1),
            pass_through: 0,
            displace_liquid: false,
            bounces: 0,
            bounce_energy: Fx::ZERO,
            physics_impulse: 0,
            on_lifetime_out_explode: false,
        },
        // `slow_bolt`：只用来验证 `air_friction < 1` 让速度逐 tick 衰减
        // （Step 3 第一条测试），其余字段取中性值——不掺重力/散布/侵彻，
        // 避免除摩擦以外的任何因素干扰"vx 单调下降"这条断言。
        SpellDef {
            name: "slow_bolt".to_string(),
            kind: SpellKind::Bolt { damage_milli: 0, knockback: Fx::ZERO },
            mana: 0,
            cooldown: 0,
            speed: Fx::from_int(8),
            life: 120,
            gravity: Fx::ZERO,
            spread_bam: 0,
            grace: 4,
            dig_power: 0,
            max_durability: 10,
            air_friction: Fx::from_ratio(9, 10),
            liquid_drag: Fx::from_int(1),
            pass_through: 0,
            displace_liquid: false,
            bounces: 0,
            bounce_energy: Fx::ZERO,
            physics_impulse: 0,
            on_lifetime_out_explode: false,
        },
    ])
}

/// 四周围墙的空场 + 一个 controller 0 的射手生物在 `(20, 65)`（`materials()`
/// 表下没有额外地板——射手不需要站得住：本文件的施法测试全部只跑一两个
/// tick 就完成断言，或者（`blast_spell_explodes_...`）压根不关心射手自身
/// 后续落到哪；弹体独立于射手飞行，不受射手掉落影响）。瞄准角默认 0
/// （`Creatures::spawn` 令 `aim = 0`；`dir_of(0) == (+1, 0)`，正右方），
/// `InputFrame` 不显式给 `aim_deg` 时天然继承这个默认方向。
///
/// R11：本函数与 `arena_wide_open` 同体例（四周墙 + 无内部地形），额外多
/// 放一个生物；`arena_with_loadout` 建在它之上。
#[allow(dead_code)]
pub fn arena_wide_open_with_shooter(spells: SpellTable, loadout: [u8; MAX_SLOTS]) -> Sim {
    let table = materials();
    let wall = table.id_by_name("wall").unwrap();
    let cfg = InitConfig { width_chunks: 4, height_chunks: 2, seed: 42, threads: 1, scan: ScanMode::LiveRect };
    let reactions = ReactionTable::empty(&table);
    let tpl = CreatureTable::default_player();
    let mut sim = Sim::new(&cfg, table, reactions, tpl, spells).unwrap();
    sim.apply_setup(&[
        Op::Fill { material: wall, x0: 0, y0: 0, x1: 255, y1: 0 },
        Op::Fill { material: wall, x0: 0, y0: 127, x1: 255, y1: 127 },
        Op::Fill { material: wall, x0: 0, y0: 0, x1: 0, y1: 127 },
        Op::Fill { material: wall, x0: 255, y0: 0, x1: 255, y1: 127 },
        Op::SpawnCreature { x: 20, y: 65, template: 0, team: 0, controller: 0, loadout },
    ]);
    sim
}

/// `arena_wide_open_with_shooter` 的便捷包装（R11）：`names` 是要装进
/// loadout 0..N 槽的法术名（经内建 `test_spell_table` 解析），其余槽位
/// `SPELL_NONE`（空槽）。`names` 为空即"全空槽"（`empty_slot_is_a_no_op`
/// 用）。
#[allow(dead_code)]
pub fn arena_with_loadout(names: &[&str]) -> Sim {
    let table = materials();
    let spells = test_spell_table(&table);
    assert!(names.len() <= MAX_SLOTS, "测试 loadout（{} 项）超出 MAX_SLOTS（{MAX_SLOTS}）", names.len());
    let mut loadout = [SPELL_NONE; MAX_SLOTS];
    for (i, name) in names.iter().enumerate() {
        loadout[i] = spells.id_by_name(name).unwrap_or_else(|| panic!("测试法术表没有名为 '{name}' 的法术"));
    }
    arena_wide_open_with_shooter(spells, loadout)
}

// ==================== M4 Task 6：不挂 loadout 的独立法术表 ====================

/// 供 `common::arena_wide_open(spell_table())` + 直接 `queue_projectile`
/// 注入用（不经 `cast_all`/loadout，同 `bolt_table()` 的用法体例）：只装两条
/// 液体阻力（Step 3）与弹跳衰减精度（Step 6 第二条）测试各自需要的法术，
/// **不是 `test_spell_table` 的子集或超集**——两张表刻意独立演化，"bomb"
/// 同名不同值不是笔误（`test_spell_table` 头注第 2 条已有这个先例：同名
/// 不代表同值，各自服务各自的断言）。
///
/// - `wet_bolt`：`pass_through` **必须**含 `liquid`——不给这一位，弹体会在
///   入水第一格当场"命中"消失（`projectile.rs::blocks_projectile` 文档：
///   液体默认挡弹体），`liquid_drag` 压根没有机会生效，
///   `liquid_drag_slows_a_projectile_inside_water_more_than_in_air` 这条
///   断言的因果链就断在这一步。
/// - `bomb`：`gravity` 特意取 **0.1**，比 `test_spell_table` 里的 0.25 小——
///   `bounce_energy_reduces_speed_each_time` 要求"反弹后速度 ≈ 反弹前 ×
///   bounce_energy"精确到 1/16 格，但 spec §5.1 的顺序（`vy += gravity` 先于
///   碰撞判定）意味着"反弹前速度"天然多算了**这一 tick**的重力增量——
///   `gravity × bounce_energy` 必须小于测试容差 `1/16`，`0.25 × 0.4 = 0.1 >
///   0.0625` 会让断言必挂（TDD 阶段实测撞见，不是纸面推演），`0.1 × 0.4 ≈
///   0.04 < 0.0625` 留有余量。`dig_power: 0`（不侵彻，命中即按 durability
///   门槛判定，`wall`/`stone` 都会立即判定"侵彻失败"进入弹跳/终结分支，
///   不会被 digging 岔开）。
#[allow(dead_code)]
pub fn spell_table() -> SpellTable {
    SpellTable::from_defs(vec![
        SpellDef {
            name: "wet_bolt".to_string(),
            kind: SpellKind::Bolt { damage_milli: 0, knockback: Fx::ZERO },
            mana: 0,
            cooldown: 0,
            speed: Fx::from_int(6),
            life: 60,
            gravity: Fx::ZERO,
            spread_bam: 0,
            grace: 0,
            dig_power: 0,
            max_durability: 10,
            air_friction: Fx::from_int(1),
            liquid_drag: Fx::from_ratio(7, 10),
            pass_through: Category::Gas.bit() | Category::Liquid.bit(),
            displace_liquid: false,
            bounces: 0,
            bounce_energy: Fx::ZERO,
            physics_impulse: 0,
            on_lifetime_out_explode: false,
        },
        SpellDef {
            name: "bomb".to_string(),
            kind: SpellKind::Blast { power: 1200, radius: 12, max_durability: 10 },
            mana: 0,
            cooldown: 0,
            speed: Fx::from_int(5),
            life: 180,
            gravity: Fx::from_ratio(1, 10),
            spread_bam: 0,
            grace: 0,
            dig_power: 0,
            max_durability: 10,
            air_friction: Fx::from_int(1),
            liquid_drag: Fx::from_int(1),
            pass_through: Category::Gas.bit(),
            displace_liquid: false,
            bounces: 2,
            bounce_energy: Fx::from_ratio(4, 10),
            physics_impulse: 0,
            on_lifetime_out_explode: true,
        },
        // `timed_bomb`：验证"寿命耗尽也要炸"（Step 4），必须在**从未命中任何
        // 东西**的前提下寿命归零——`life: 5` 配 `gravity: 0.25`、静止出生
        // （调用方给 `vx = vy = 0`），5 tick 内累积下坠只有 3.75 格
        // （0.25+0.5+0.75+1+1.25），实测钉死：落点离出生点不到 4 格，测试把
        // 石块顶面摆在出生点下方 10 格处，安全帧内摸不到、但落在爆炸半径
        // （12）内。`life: 5` 比 `bomb`/`wet_bolt` 短得多是刻意的——寿命耗尽
        // 分支必须在"从未进入过命中判定的碰撞分支"这个前提下触发，值大了
        // 反而增加"半路撞上别的什么"的偶然性。
        SpellDef {
            name: "timed_bomb".to_string(),
            kind: SpellKind::Blast { power: 1200, radius: 12, max_durability: 10 },
            mana: 0,
            cooldown: 0,
            speed: Fx::ZERO,
            life: 5,
            gravity: Fx::from_ratio(1, 4),
            spread_bam: 0,
            grace: 0,
            dig_power: 0,
            max_durability: 10,
            air_friction: Fx::from_int(1),
            liquid_drag: Fx::from_int(1),
            pass_through: 0,
            displace_liquid: false,
            bounces: 0,
            bounce_energy: Fx::ZERO,
            physics_impulse: 0,
            on_lifetime_out_explode: true,
        },
        // `scatter_bolt`（M4 Task 7 Step 3，供 `spread_angle_is_uniform_
        // within_the_declared_cone` 散布分布回归专用）：**只服务本测试**，
        // 不进 `duel.ron`、不进任何行为测试——`spread_bam` 是全表唯一非零
        // 值，`cooldown: 1` + `life: 1` 让它能连续 5000 tick 每 tick 出一发、
        // 出生即可观测、下一 tick 自动销毁，不需要手动清池防重复计数。
        // `spread_bam = 5461`：30° 量化（`round(30/360*65536) = 5461.33 →
        // 5461`），与 `scenario::quantize_bam` 的四舍五入口径一致，测试里
        // 直接写这个整数常量而不复用该函数——core 测试不依赖 harness crate。
        SpellDef {
            name: "scatter_bolt".to_string(),
            kind: SpellKind::Bolt { damage_milli: 0, knockback: Fx::ZERO },
            mana: 0,
            cooldown: 1,
            speed: Fx::from_int(8),
            life: 1,
            gravity: Fx::ZERO,
            spread_bam: 5461,
            grace: 0,
            dig_power: 0,
            max_durability: 10,
            air_friction: Fx::from_int(1),
            liquid_drag: Fx::from_int(1),
            pass_through: 0,
            displace_liquid: false,
            bounces: 0,
            bounce_energy: Fx::ZERO,
            physics_impulse: 0,
            on_lifetime_out_explode: false,
        },
    ])
}
