//! 测试公用：内联材料表（core 测试不读 data/，保持 crate 自包含）。

use sand_core::{Category, InitConfig, MaterialDef, MaterialTable, ScanMode, Sim};

pub const SAND: u8 = 2;
pub const WATER: u8 = 3;

pub fn test_table() -> MaterialTable {
    let def = |id: u8, name: &str, category: Category, density: u16| MaterialDef {
        id,
        name: name.into(),
        category,
        density,
        color: (0, 0, 0),
    };
    MaterialTable::new(vec![
        def(0, "air", Category::Static, 0),
        def(1, "wall", Category::Static, 100),
        def(SAND, "sand", Category::Powder, 40),
        def(WATER, "water", Category::Liquid, 16),
    ])
    .unwrap()
}

pub fn sim(width_chunks: usize, height_chunks: usize, seed: u64, threads: usize, scan: ScanMode) -> Sim {
    let cfg = InitConfig { width_chunks, height_chunks, seed, threads, scan };
    Sim::new(&cfg, test_table()).unwrap()
}
