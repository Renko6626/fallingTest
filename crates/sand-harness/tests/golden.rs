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
    let report = runner::run(
        &sc,
        &table,
        materials_fp,
        4,
        sand_core::ScanMode::LiveRect,
        sc.ticks,
        runner::HashStream::default(),
    )
    .unwrap();
    let got = report.lines.join("\n") + "\n";
    // 行尾归一化（2026-08-31 双机 hashrun 发现）：harness 输出恒为 LF，而 golden
    // 文件在 Windows 侧可能被 `core.autocrlf` 检出成 CRLF——那样纯文本比对必挂，
    // 但那是**文件怎么落到磁盘上**的问题，不是哈希流不一致。`.gitattributes` 已
    // 钉死 LF 检出，这里是第二道（zip 分发、编辑器改行尾仍能绕过 .gitattributes）。
    let want = std::fs::read_to_string(repo_path(golden)).unwrap().replace('\r', "");
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

#[test]
fn golden_waterfall_ci() {
    check("data/scenarios/waterfall_ci.ron", "crates/sand-harness/tests/golden/waterfall_ci.golden");
}

#[test]
fn golden_explosion_ci() {
    check("data/scenarios/explosion_ci.ron", "crates/sand-harness/tests/golden/explosion_ci.golden");
}
