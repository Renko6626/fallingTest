//! 测试公用：内联材料表（core 测试不读 data/，保持 crate 自包含）。

use sand_core::{Category, InitConfig, MaterialDef, MaterialTable, ScanMode, Sim, BLAST_COST_INFINITE};

pub const SAND: u8 = 2;
pub const WATER: u8 = 3;

pub fn test_table() -> MaterialTable {
    // blast_cost 取 spec §6 的口径值（air 0 / water 1 / sand 2 / wall 免疫）。
    // 爆炸行为测试并未独立成 `explode_behavior.rs` 文件，而是内联在
    // `crates/sand-core/src/world.rs` 的 `#[cfg(test)] mod tests` 里——本表
    // 供本文件之外、复用同一材料语义的测试模块直接调用。
    let def = |id: u8, name: &str, category: Category, density: u16, blast_cost: u32| MaterialDef {
        id,
        name: name.into(),
        category,
        density,
        color: (0, 0, 0),
        blast_cost,
    };
    MaterialTable::new(vec![
        def(0, "air", Category::Static, 0, 0),
        def(1, "wall", Category::Static, 100, BLAST_COST_INFINITE),
        def(SAND, "sand", Category::Powder, 40, 2),
        def(WATER, "water", Category::Liquid, 16, 1),
    ])
    .unwrap()
}

pub fn sim(width_chunks: usize, height_chunks: usize, seed: u64, threads: usize, scan: ScanMode) -> Sim {
    let cfg = InitConfig { width_chunks, height_chunks, seed, threads, scan };
    Sim::new(&cfg, test_table()).unwrap()
}
