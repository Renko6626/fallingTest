//! golden replay 回归（spec §5.4 第 3 层）：重放场景，断言哈希流与入库值逐字一致。
//! golden 重生成：`sand-harness replay <scenario> --write-golden <file>`——
//! 仅允许在显式声明"语义变更、hash 序列作废"的变更里更新。

use sand_harness::runner;
use sand_harness::scenario::{load_materials, load_scenario};

fn repo_path(rel: &str) -> String {
    format!("{}/../../{rel}", env!("CARGO_MANIFEST_DIR"))
}

fn check(scenario: &str, golden: &str) {
    let (table, materials_fp) = load_materials(&repo_path("data/materials.ron")).unwrap();
    let sc = load_scenario(&repo_path(scenario), &table).unwrap();
    // LiveRect 跑 golden：与 M0 时代（ChunkSleep）哈希一字不差是 O1 等价性的最硬证据
    let report =
        runner::run(&sc, &table, materials_fp, 4, sand_core::ScanMode::LiveRect, sc.ticks).unwrap();
    let got = report.lines.join("\n") + "\n";
    let want = std::fs::read_to_string(repo_path(golden)).unwrap();
    assert_eq!(got, want, "golden 回归失败：{scenario} 哈希流与 {golden} 不一致");
}

#[test]
fn golden_sand_pile() {
    check("data/scenarios/sand_pile.ron", "crates/sand-harness/tests/golden/sand_pile.golden");
}

#[test]
fn golden_mixed() {
    check("data/scenarios/mixed.ron", "crates/sand-harness/tests/golden/mixed.golden");
}
