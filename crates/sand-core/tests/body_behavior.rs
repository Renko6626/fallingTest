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

// ==================== Task 3：地形（B′）与浮沉 ====================

fn floor(w: i32, h: i32) -> Op {
    Op::Fill { material: 1, x0: 0, y0: h - 4, x1: w - 1, y1: h - 4 }
}

fn all_chunks_asleep(s: &Sim) -> bool {
    s.world().chunks.iter().all(|c| c.dirty.is_empty() && c.next_dirty.snapshot().is_empty())
}

/// 木箱落到墙上静止、入睡，且**全图入睡**（spec §3 零写入执法 `resting_body_lets_chunk_sleep`）。
#[test]
fn resting_body_lets_chunk_sleep() {
    let mut s = body_sim(11, 1);
    s.apply_setup(&[floor(128, 128), Op::SpawnBody { material: WOOD, x: 40, y: 60, w: 24, h: 16 }]);
    for _ in 0..400 {
        s.step(&[]);
    }
    let cells = body_cells(&s);
    assert_eq!(cells.len(), 24 * 16, "箱子完整");
    let bottom = cells.iter().map(|c| c.1).max().unwrap();
    assert!((121..=123).contains(&bottom), "箱子应停在地板（y=124）之上：底边 {bottom}");
    assert!(all_chunks_asleep(&s), "静止刚体必须零写入、全图入睡");
}

/// B′：沙堆托得住木箱（不陷进去）。
#[test]
fn crate_rests_on_sand_pile() {
    let mut s = body_sim(13, 1);
    s.apply_setup(&[
        floor(128, 128),
        Op::Fill { material: 2, x0: 30, y0: 100, x1: 90, y1: 123 }, // 沙层 24 深
        Op::SpawnBody { material: WOOD, x: 50, y: 40, w: 24, h: 16 },
    ]);
    for _ in 0..500 {
        s.step(&[]);
    }
    let cells = body_cells(&s);
    assert_eq!(cells.len(), 24 * 16);
    let bottom = cells.iter().map(|c| c.1).max().unwrap();
    assert!(bottom < 100, "箱子应停在沙面（y=100）之上、不陷入：底边 {bottom}");
}

/// 木箱（密度 12）落水上浮、石箱（密度 40）下沉——采样式阿基米德。
#[test]
fn wood_crate_floats_stone_crate_sinks() {
    let mut s = body_sim(17, 1);
    // 盆：底 y=120..123，壁 x=10..11 与 x=116..117，水面 y=80
    s.apply_setup(&[
        Op::Fill { material: 1, x0: 0, y0: 120, x1: 127, y1: 123 },
        Op::Fill { material: 1, x0: 10, y0: 60, x1: 11, y1: 119 },
        Op::Fill { material: 1, x0: 116, y0: 60, x1: 117, y1: 119 },
        Op::Fill { material: 3, x0: 12, y0: 80, x1: 115, y1: 119 },
        Op::SpawnBody { material: WOOD, x: 30, y: 40, w: 16, h: 12 },
        Op::SpawnBody { material: STONE, x: 80, y: 40, w: 16, h: 12 },
    ]);
    for _ in 0..900 {
        s.step(&[]);
    }
    let cells = body_cells(&s);
    let wood: Vec<_> = cells.iter().filter(|&&(x, _)| x < 64).collect();
    let stone: Vec<_> = cells.iter().filter(|&&(x, _)| x >= 64).collect();
    assert_eq!(wood.len(), 16 * 12, "木箱完整");
    assert_eq!(stone.len(), 16 * 12, "石箱完整");
    let wood_bottom = wood.iter().map(|c| c.1).max().unwrap();
    let wood_top = wood.iter().map(|c| c.1).min().unwrap();
    let stone_bottom = stone.iter().map(|c| c.1).max().unwrap();
    assert!(wood_top < 84 && wood_bottom > 78, "木箱应浮在水面附近：top {wood_top} bottom {wood_bottom}");
    assert!(stone_bottom >= 117, "石箱应沉到盆底：bottom {stone_bottom}");
}

/// 满池入箱水漫出：池外出现液体，且水总量（格 + 粒子）守恒。
#[test]
fn full_pool_overflows_when_crate_drops() {
    let mut s = body_sim(19, 1);
    // 盆壁只到 y=90，水灌满到壁顶；箱子从上方落入
    s.apply_setup(&[
        Op::Fill { material: 1, x0: 0, y0: 120, x1: 127, y1: 123 },
        Op::Fill { material: 1, x0: 40, y0: 90, x1: 41, y1: 119 },
        Op::Fill { material: 1, x0: 86, y0: 90, x1: 87, y1: 119 },
        Op::Fill { material: 3, x0: 42, y0: 90, x1: 85, y1: 119 },
        Op::SpawnBody { material: STONE, x: 56, y: 30, w: 16, h: 12 },
    ]);
    let water_cells0 = s.world().count_material(3);
    for _ in 0..600 {
        s.step(&[]);
    }
    let mut outside = 0;
    for y in 0..128 {
        for x in 0..128 {
            if s.world().cell(x, y).material() == 3 && !(42..=85).contains(&x) {
                outside += 1;
            }
        }
    }
    assert!(outside > 0, "满池入箱必须有水漫到盆外");
    let particles = (0..s.particles().len()).filter(|&i| s.particles().material(i) == 3).count();
    let water_now = s.world().count_material(3) + particles;
    assert_eq!(water_now, water_cells0, "水总量守恒（格 + 粒子）");
}
