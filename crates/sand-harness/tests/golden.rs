//! golden replay 回归（spec §5.4 第 3 层）：重放场景，断言哈希流与入库值逐字一致。
//! golden 重生成：`sand-harness replay <scenario> --write-golden <file>`——
//! 仅允许在显式声明"语义变更、hash 序列作废"的变更里更新。

use sand_harness::runner;
use sand_harness::scenario::{load_creatures, load_materials, load_reactions, load_scenario, load_spells};

fn repo_path(rel: &str) -> String {
    format!("{}/../../{rel}", env!("CARGO_MANIFEST_DIR"))
}

fn check(scenario: &str, golden: &str) {
    let (table, materials_fp) = load_materials(&repo_path("data/materials.ron")).unwrap();
    let (reactions, reactions_fp) = load_reactions(&repo_path("data/reactions.ron"), &table).unwrap();
    // M4 Task 5 起真实加载（与 main.rs 同路径）：这两张表本身对本文件覆盖的
    // 6 个既有场景无行为影响（它们都不含 `Op::SpawnCreature`，`entity_hash`
    // 恒为空表早退的 0），但指纹必须跟 `main.rs hashrun` 打出来的 golden 逐字
    // 一致，才能通过下面的文本比对——这正是本 Task 把 golden 重录一次的理由。
    let (creature_table, creatures_fp) = load_creatures(&repo_path("data/creatures.ron"), &table).unwrap();
    let (spell_table, spells_fp) = load_spells(&repo_path("data/spells.ron"), &table).unwrap();
    let sc = load_scenario(&repo_path(scenario), &table, &spell_table).unwrap();
    let tables = runner::Tables {
        materials: &table,
        reactions: &reactions,
        creatures: &creature_table,
        spells: &spell_table,
    };
    // LiveRect 跑 golden：与 M0 时代（ChunkSleep）哈希一字不差是 O1 等价性的最硬证据
    let report = runner::run(
        &sc,
        &tables,
        runner::Fingerprints {
            materials: materials_fp,
            reactions: reactions_fp,
            creatures: creatures_fp,
            spells: spells_fp,
        },
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

#[test]
fn golden_fire_oil_chain() {
    // M2 主验收场景（spec §0）：反应 + 燃烧 + 气体全链条的 golden 回归。
    check("data/scenarios/fire_oil_chain.ron", "crates/sand-harness/tests/golden/fire_oil_chain.golden");
}

#[test]
fn golden_crate_yard() {
    // M3 主验收场景（spec §0）：刚体落地/沙托/浮沉/切割/燃烧散架的 golden 回归。
    check("data/scenarios/crate_yard.ron", "crates/sand-harness/tests/golden/crate_yard.golden");
}
