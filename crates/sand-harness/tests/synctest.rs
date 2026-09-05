//! harness 层 CI SyncTest（M1 Task 5）：加载真实 RON 场景（含 `Op::Emit`），
//! 走 harness 的量化路径（`scenario::quantize_fx`），六配置逐 tick 全哈希比对。
//!
//! 与 `sand-core/tests/synctest_ci.rs`（纯 Rust 字面量 `Op`，crate 自包含，
//! 不碰 harness）互补：这里额外覆盖 RON→Fx 量化本身不会引入任何与线程数/
//! 扫描模式相关的分叉——量化发生在加载期一次性完成，若结果混入了浮点
//! 运算的运行期残留（例如量化写错、每次重新计算），六配置比对会立刻暴露。

use sand_harness::runner;
use sand_harness::scenario::{load_materials, load_reactions, load_scenario};

fn repo_path(rel: &str) -> String {
    format!("{}/../../{rel}", env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn waterfall_ci_six_configs_zero_divergence() {
    let (table, _materials_fp) = load_materials(&repo_path("data/materials.ron")).unwrap();
    let (reactions, _fp) = load_reactions(&repo_path("data/reactions.ron"), &table).unwrap();
    let sc = load_scenario(&repo_path("data/scenarios/waterfall_ci.ron"), &table).unwrap();
    let creature_table = sand_core::CreatureTable::empty();
    let spell_table = sand_core::SpellTable::empty();
    let tables = runner::Tables {
        materials: &table,
        reactions: &reactions,
        creatures: &creature_table,
        spells: &spell_table,
    };
    runner::synctest(&sc, &tables, 4, sc.ticks)
        .unwrap_or_else(|e| panic!("waterfall_ci 六配置 SyncTest 分叉：{e}"));
}

/// `Op::Explode` 版本（M1 Task 6）：与上一条互补——覆盖爆炸射线/圆周生成/
/// 溅射粒子在六配置（线程数 × 扫描模式）下的确定性，RON→整数字段透传路径
/// （`Op::Explode` 无需量化，见 `scenario::resolve_op`）本身也受此测试覆盖。
#[test]
fn explosion_ci_six_configs_zero_divergence() {
    let (table, _materials_fp) = load_materials(&repo_path("data/materials.ron")).unwrap();
    let (reactions, _fp) = load_reactions(&repo_path("data/reactions.ron"), &table).unwrap();
    let sc = load_scenario(&repo_path("data/scenarios/explosion_ci.ron"), &table).unwrap();
    let creature_table = sand_core::CreatureTable::empty();
    let spell_table = sand_core::SpellTable::empty();
    let tables = runner::Tables {
        materials: &table,
        reactions: &reactions,
        creatures: &creature_table,
        spells: &spell_table,
    };
    runner::synctest(&sc, &tables, 4, sc.ticks)
        .unwrap_or_else(|e| panic!("explosion_ci 六配置 SyncTest 分叉：{e}"));
}

/// M2 反应表版本（Task 2）：气体上浮 + water/fire 反应结算在六配置下的
/// 确定性（spec §0 验收第 1/2 项的 SyncTest 面；燃烧行为随 Task 3 长入本场景）。
#[test]
fn fire_oil_chain_six_configs_zero_divergence() {
    let (table, _materials_fp) = load_materials(&repo_path("data/materials.ron")).unwrap();
    let (reactions, _fp) = load_reactions(&repo_path("data/reactions.ron"), &table).unwrap();
    let sc = load_scenario(&repo_path("data/scenarios/fire_oil_chain.ron"), &table).unwrap();
    let creature_table = sand_core::CreatureTable::empty();
    let spell_table = sand_core::SpellTable::empty();
    let tables = runner::Tables {
        materials: &table,
        reactions: &reactions,
        creatures: &creature_table,
        spells: &spell_table,
    };
    runner::synctest(&sc, &tables, 4, sc.ticks)
        .unwrap_or_else(|e| panic!("fire_oil_chain 六配置 SyncTest 分叉：{e}"));
}

/// M3 刚体版本（Task 5）：Rapier 步进 + 盖章/对账在六配置下的确定性；`runner::synctest`
/// 同时每 256 tick 比对引擎 serde 快照 checksum（spec §7，M6 决策门预演）。
#[test]
fn crate_yard_six_configs_zero_divergence() {
    let (table, _materials_fp) = load_materials(&repo_path("data/materials.ron")).unwrap();
    let (reactions, _fp) = load_reactions(&repo_path("data/reactions.ron"), &table).unwrap();
    let sc = load_scenario(&repo_path("data/scenarios/crate_yard.ron"), &table).unwrap();
    let creature_table = sand_core::CreatureTable::empty();
    let spell_table = sand_core::SpellTable::empty();
    let tables = runner::Tables {
        materials: &table,
        reactions: &reactions,
        creatures: &creature_table,
        spells: &spell_table,
    };
    runner::synctest(&sc, &tables, 4, sc.ticks)
        .unwrap_or_else(|e| panic!("crate_yard 六配置 SyncTest 分叉：{e}"));
}
