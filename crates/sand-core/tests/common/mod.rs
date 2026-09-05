//! 测试公用：内联材料表（core 测试不读 data/，保持 crate 自包含）。

use sand_core::{
    input::MAX_SLOTS, Category, CreatureTable, InitConfig, MaterialDef, MaterialTable, Op, ReactionTable, ScanMode,
    Sim, SpellTable,
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
    ])
    .unwrap()
}

/// 4×2 chunk（256×128）的世界，底行 wall；在 (32, 100) 放一个 controller 0 的生物
/// （M4 Task 2 生物行为测试用，spec `creature_behavior.rs` Step 4）。
#[allow(dead_code)]
pub fn floor_world_with_creature(tbl: CreatureTable) -> (Sim, u8) {
    let table = materials();
    let wall = table.id_by_name("wall").unwrap();
    let cfg = InitConfig { width_chunks: 4, height_chunks: 2, seed: 42, threads: 1, scan: ScanMode::LiveRect };
    let reactions = ReactionTable::empty(&table);
    let mut sim = Sim::new(&cfg, table, reactions, tbl, SpellTable::empty()).unwrap();
    sim.apply_setup(&[
        Op::Fill { material: wall, x0: 0, y0: 127, x1: 255, y1: 127 },
        Op::SpawnCreature { x: 32, y: 100, template: 0, team: 0, controller: 0, loadout: [255; MAX_SLOTS] },
    ]);
    (sim, 0)
}
