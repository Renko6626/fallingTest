//! `duel.ron` 内容自检（M4 spec §0.2 验收第 1 项的"五项行为"半边）。
//!
//! **为什么单开一条测试**：`golden_duel` 与 `duel_six_configs_zero_divergence`
//! 钉的是**确定性**——同一份输入永远算出同一串哈希。它们对场景**内容**一无所知：
//! 把 `duel.ron` 的输入时间线整段删掉，两者照样绿（只需重录一次 golden）。
//! 而验收要求的是"五项行为都真的发生过"，这件事此前只由场景文件里的注释
//! 声称、无人验证。本测试把那五条声称变成断言，未来谁改坏时间线、改动
//! 法术数值或改动生物运动学导致某一项不再发生，这里立刻变红。
//!
//! 断言一律取**方向性**判据（"石头少了"而非"石头恰好少 37 格"），避免把手感
//! 旋钮的正常调整变成测试维护负担——精确数值归 golden 管。

use sand_core::ScanMode;
use sand_harness::runner;
use sand_harness::scenario::{
    load_creatures, load_materials, load_reactions, load_scenario, load_spells,
};

fn repo_path(rel: &str) -> String {
    format!("{}/../../{rel}", env!("CARGO_MANIFEST_DIR"))
}

/// 统计矩形区域内某材质的格数。
fn count_in(sim: &sand_core::Sim, mat: u8, x0: i32, y0: i32, x1: i32, y1: i32) -> usize {
    let mut n = 0;
    for y in y0..=y1 {
        for x in x0..=x1 {
            if sim.world().cell(x, y).material() == mat {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn duel_scenario_actually_exercises_all_five_acceptance_behaviours() {
    let (table, _) = load_materials(&repo_path("data/materials.ron")).unwrap();
    let (reactions, _) = load_reactions(&repo_path("data/reactions.ron"), &table).unwrap();
    let (creature_table, _) = load_creatures(&repo_path("data/creatures.ron"), &table).unwrap();
    let (spell_table, _) = load_spells(&repo_path("data/spells.ron"), &table).unwrap();
    let sc = load_scenario(&repo_path("data/scenarios/duel.ron"), &table, &spell_table).unwrap();
    let tables = runner::Tables {
        materials: &table,
        reactions: &reactions,
        creatures: &creature_table,
        spells: &spell_table,
    };
    let mut sim = runner::build_sim(&sc, &tables, 1, ScanMode::LiveRect).unwrap();

    let stone = table.id_by_name("stone").unwrap();
    let oil = table.id_by_name("oil").unwrap();

    // ② 矮台阶 / ③ 悬空石柱：两块石头分别统计，避免一块的碎屑掩盖另一块没被打中。
    let step_before = count_in(&sim, stone, 100, 124, 108, 126);
    let pillar_before = count_in(&sim, stone, 200, 60, 230, 100);
    assert!(step_before > 0 && pillar_before > 0, "场景 setup 应当先摆好两块石头");

    // 逐 tick 跑完，途中记录三个只能在中途观测到的量。
    let mut max_x0 = i32::MIN; // ① 0 号向右走到过的最远处
    let mut oil_peak = 0usize; // ④ 浇油摊开后的峰值（点火前）
    for t in 0..sc.ticks {
        sim.step(&sc.ops_for_tick(t), sc.inputs_for_tick(t));
        if let Some(c) = sim.creatures().get(0) {
            max_x0 = max_x0.max(c.x.to_cell());
        }
        if t < 1300 {
            oil_peak = oil_peak.max(sim.world().count_material(oil));
        }
    }

    // ① 蹚水：0 号出生在 x=30、水池铺在 x∈[15,75]，越过 x=75 就意味着它真的
    //    从水里趟了过去（而不是卡在池边或被水挡住）。
    assert!(max_x0 > 75, "① 0 号应当趟过水池（x∈[15,75]），实际最远只到 x={max_x0}");

    // ② 炸台阶：bomb 朝正下方炸，台阶石头必须少掉。
    let step_after = count_in(&sim, stone, 100, 124, 108, 126);
    assert!(
        step_after < step_before,
        "② bomb 应当炸掉一部分台阶石头：{step_before} → {step_after}"
    );

    // ③ 挖石柱：digger 朝正上方钻，石柱必须少掉；同时不该被整根打光
    //    （能量池有限，spec §5.2 的侵彻语义就是"钻一段就停"）。
    let pillar_after = count_in(&sim, stone, 200, 60, 230, 100);
    assert!(
        pillar_after < pillar_before,
        "③ digger 应当钻穿一段石柱：{pillar_before} → {pillar_after}"
    );
    assert!(
        pillar_after > 0,
        "③ 侵彻能量有限，不该把整根石柱打光（dig_power 语义退化）"
    );

    // ④ 油火连锁：浇油摊开后点火，油应当被烧掉大半——只烧掉零星几格说明
    //    连锁没真正跑起来（TDD 阶段实测撞见过这个失败模式，见 duel.ron 注释）。
    let oil_after = sim.world().count_material(oil);
    assert!(oil_peak > 20, "④ 两次 oil_spray 应当摊出一片油，实际峰值只有 {oil_peak}");
    assert!(
        oil_after * 2 < oil_peak,
        "④ 点火后油应当被连锁烧掉大半：峰值 {oil_peak} → 终态 {oil_after}"
    );

    // ⑤ 一方被打死：0 号持续点射 spark_bolt，1 号 hp 归零走墓碑（id 保留、
    //    alive=false）。同时断言 0 号还活着——否则"死了一个"可能是自伤或
    //    环境伤害造成的，不是对射的结果。
    let c0 = sim.creatures().get(0).expect("0 号生物应当仍在表里（id 永不回收）");
    let c1 = sim.creatures().get(1).expect("1 号生物应当仍在表里（墓碑不移除）");
    assert!(c0.alive, "⑤ 0 号不该死——它是开枪的一方");
    assert!(!c1.alive, "⑤ 1 号应当被 spark_bolt 打死，实际 hp={}", c1.hp);
}
