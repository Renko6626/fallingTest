//! 刚体行为集成测试（M3 spec §9）：经 `Sim` 公共 API 驱动。
//! Task 2 范围：生成、下落盖章、确定性、出界移除；地形/浮沉/对账随 Task 3/4 补。

mod common;

use common::sim_with_reactions;
use sand_core::{Category, MaterialDef, MaterialTable, Op, ReactionTable, ScanMode, Sim, MAT_AIR};

const WOOD: u8 = 4;
const STONE: u8 = 5;

fn body_table() -> MaterialTable {
    MaterialTable::new(vec![
        MaterialDef::base(0, "air", Category::Static, 0),
        MaterialDef::base(1, "wall", Category::Static, 100),
        MaterialDef::base(2, "sand", Category::Powder, 40),
        MaterialDef::base(3, "water", Category::Liquid, 16),
        MaterialDef::base(WOOD, "wood", Category::Static, 12),
        MaterialDef::base(STONE, "stone", Category::Static, 40),
    ])
    .unwrap()
}

fn body_sim(seed: u64, threads: usize) -> Sim {
    let t = body_table();
    let r = ReactionTable::empty(&t);
    sim_with_reactions(2, 2, seed, threads, ScanMode::LiveRect, t, r)
}

fn body_cells(s: &Sim) -> Vec<(i32, i32)> {
    let mut v = Vec::new();
    for y in 0..128 {
        for x in 0..128 {
            if s.world().cell(x, y).is_body() {
                v.push((x, y));
            }
        }
    }
    v
}

/// 生成后第一个 tick 就盖章；之后自由下落、脚印逐 tick 下移、格数守恒。
#[test]
fn spawned_crate_is_stamped_and_falls() {
    let mut s = body_sim(1, 1);
    s.apply_setup(&[Op::SpawnBody { material: WOOD, x: 40, y: 10, w: 24, h: 16 }]);
    assert_eq!(s.bodies().len(), 1);
    s.step(&[]);
    let c0 = body_cells(&s);
    assert_eq!(c0.len(), 24 * 16, "首 tick 盖章格数 = 面积");
    let top0 = c0.iter().map(|c| c.1).min().unwrap();
    // 重力 0.25 格/tick²：10 tick 约落 12 格，仍在 128 高的世界里
    for _ in 0..10 {
        s.step(&[]);
    }
    let c1 = body_cells(&s);
    assert_eq!(c1.len(), 24 * 16, "下落中格数守恒（无洞、无重复）");
    let top1 = c1.iter().map(|c| c.1).min().unwrap();
    assert!(top1 > top0, "刚体应在下落：顶边 {top0} → {top1}");
    // 旧脚印已反盖章为 air
    assert_eq!(s.world().cell(40, 10).material(), MAT_AIR);
}

/// 确定性：两实例同 ops 同种子 ⇒ 逐 tick `state_hash` 与引擎 checksum 逐位相同，
/// 且与线程数无关（刚体相是串行阶段，本就该如此——这里执法）。
#[test]
fn body_sim_is_deterministic_across_threads() {
    let mut a = body_sim(7, 1);
    let mut b = body_sim(7, 8);
    let setup = [
        Op::SpawnBody { material: WOOD, x: 20, y: 10, w: 24, h: 16 },
        Op::SpawnBody { material: STONE, x: 70, y: 30, w: 12, h: 12 },
    ];
    a.apply_setup(&setup);
    b.apply_setup(&setup);
    for t in 0..120 {
        a.step(&[]);
        b.step(&[]);
        assert_eq!(a.state_hash(), b.state_hash(), "tick {t} 状态哈希分叉");
        assert_eq!(a.physics_checksum(), b.physics_checksum(), "tick {t} 引擎快照分叉");
    }
}

/// 掉出世界超过 OUT_OF_WORLD_MARGIN 的刚体被确定性移除（不永久占用引擎）。
#[test]
fn crate_falling_out_of_world_is_removed() {
    let mut s = body_sim(3, 1);
    s.apply_setup(&[Op::SpawnBody { material: STONE, x: 40, y: 100, w: 12, h: 12 }]);
    for _ in 0..600 {
        s.step(&[]);
    }
    assert_eq!(s.bodies().len(), 0, "掉出世界的刚体应被移除");
    assert!(body_cells(&s).is_empty());
}

/// 上限与契约：第 257 个 SpawnBody 被拒绝并计数。
#[test]
fn spawn_beyond_max_bodies_is_rejected() {
    let mut s = body_sim(5, 1);
    let mut ops = Vec::new();
    for i in 0..(sand_core::MAX_BODIES as i32 + 1) {
        ops.push(Op::SpawnBody { material: WOOD, x: (i % 16) * 8, y: (i / 16) * 8, w: 4, h: 4 });
    }
    s.apply_setup(&ops);
    assert_eq!(s.bodies().len(), sand_core::MAX_BODIES);
    assert_eq!(s.bodies().rejected_total, 1);
}
