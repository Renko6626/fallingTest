//! harness 层 CI SyncTest（M1 Task 5）：加载真实 RON 场景（含 `Op::Emit`），
//! 走 harness 的量化路径（`scenario::quantize_fx`），六配置逐 tick 全哈希比对。
//!
//! 与 `sand-core/tests/synctest_ci.rs`（纯 Rust 字面量 `Op`，crate 自包含，
//! 不碰 harness）互补：这里额外覆盖 RON→Fx 量化本身不会引入任何与线程数/
//! 扫描模式相关的分叉——量化发生在加载期一次性完成，若结果混入了浮点
//! 运算的运行期残留（例如量化写错、每次重新计算），六配置比对会立刻暴露。

use sand_harness::runner;
use sand_harness::scenario::{load_materials, load_scenario};

fn repo_path(rel: &str) -> String {
    format!("{}/../../{rel}", env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn waterfall_ci_six_configs_zero_divergence() {
    let (table, _materials_fp) = load_materials(&repo_path("data/materials.ron")).unwrap();
    let sc = load_scenario(&repo_path("data/scenarios/waterfall_ci.ron"), &table).unwrap();
    runner::synctest(&sc, &table, 4, sc.ticks)
        .unwrap_or_else(|e| panic!("waterfall_ci 六配置 SyncTest 分叉：{e}"));
}
