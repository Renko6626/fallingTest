//! 测试公用：内联材料表（core 测试不读 data/，保持 crate 自包含）。

use sand_core::{Category, InitConfig, MaterialDef, MaterialTable, ReactionTable, ScanMode, Sim};

pub const SAND: u8 = 2;
pub const WATER: u8 = 3;

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
    Sim::new(&cfg, table, reactions).unwrap()
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
    Sim::new(&cfg, table, reactions).unwrap()
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
    Sim::new(&cfg, table, reactions).unwrap()
}
