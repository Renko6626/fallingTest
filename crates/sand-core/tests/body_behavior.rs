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
    s.apply_setup(&[Op::SpawnBody { material: WOOD, x: 40, y: 10, w: 24, h: 16, angle_deg: 0 }]);
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
        Op::SpawnBody { material: WOOD, x: 20, y: 10, w: 24, h: 16, angle_deg: 0 },
        Op::SpawnBody { material: STONE, x: 70, y: 30, w: 12, h: 12, angle_deg: 0 },
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
    s.apply_setup(&[Op::SpawnBody { material: STONE, x: 40, y: 100, w: 12, h: 12, angle_deg: 0 }]);
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
        ops.push(Op::SpawnBody { material: WOOD, x: (i % 16) * 8, y: (i / 16) * 8, w: 4, h: 4, angle_deg: 0 });
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
    s.apply_setup(&[floor(128, 128), Op::SpawnBody { material: WOOD, x: 40, y: 60, w: 24, h: 16, angle_deg: 0 }]);
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
        Op::SpawnBody { material: WOOD, x: 50, y: 40, w: 24, h: 16, angle_deg: 0 },
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
        Op::SpawnBody { material: WOOD, x: 30, y: 40, w: 16, h: 12, angle_deg: 0 },
        Op::SpawnBody { material: STONE, x: 80, y: 40, w: 16, h: 12, angle_deg: 0 },
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
        Op::SpawnBody { material: STONE, x: 56, y: 30, w: 16, h: 12, angle_deg: 0 },
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

// ==================== Task 4：破坏对账、重提取、燃烧散架 ====================

const FIRE: u8 = 6;
const SMOKE: u8 = 7;

fn burn_body_table() -> MaterialTable {
    MaterialTable::new(vec![
        MaterialDef::base(0, "air", Category::Static, 0),
        MaterialDef::base(1, "wall", Category::Static, 100),
        MaterialDef::base(2, "sand", Category::Powder, 40),
        MaterialDef::base(3, "water", Category::Liquid, 16),
        // fire_chance 必须给：实心块靠燃烧格向新暴露的空气格产火才能向内推进
        // （外壳烧尽后新一层的 4 邻里没有燃烧格，只有火气体能把它点着）。
        MaterialDef {
            fire_hp: 50,
            ignition_temp: 80,
            fire_temp: 100,
            fire_chance: 153,
            flame_to: FIRE,
            ..MaterialDef::base(WOOD, "wood", Category::Static, 12)
        },
        MaterialDef::base(STONE, "stone", Category::Static, 40),
        MaterialDef {
            lifetime: 40,
            fire_temp: 100,
            decay_to: SMOKE,
            rise_chance: 128,
            ..MaterialDef::base(FIRE, "fire", Category::Gas, 1)
        },
        MaterialDef { lifetime: 200, ..MaterialDef::base(SMOKE, "smoke", Category::Gas, 2) },
    ])
    .unwrap()
}

fn burn_body_sim(seed: u64) -> Sim {
    let t = burn_body_table();
    let r = ReactionTable::empty(&t);
    sim_with_reactions(2, 2, seed, 1, ScanMode::LiveRect, t, r)
}

fn total_body_pixels(s: &Sim) -> usize {
    s.bodies().list.iter().map(|b| b.mask.iter().filter(|&&m| m).count()).sum()
}

/// 爆炸切割木箱：静止的 24×16 箱子被中心半径 10 的爆炸切成两块（body 数 1 → ≥ 2）。
#[test]
fn explosion_splits_crate_in_two() {
    let mut s = body_sim(23, 1);
    s.apply_setup(&[floor(128, 128), Op::SpawnBody { material: WOOD, x: 40, y: 100, w: 24, h: 16, angle_deg: 0 }]);
    for _ in 0..200 {
        s.step(&[]);
    }
    assert_eq!(s.bodies().len(), 1);
    // 箱子已落到地板上：找到当前脚印中心
    let cells = body_cells(&s);
    let cx = (cells.iter().map(|c| c.0).min().unwrap() + cells.iter().map(|c| c.0).max().unwrap()) / 2;
    let cy = (cells.iter().map(|c| c.1).min().unwrap() + cells.iter().map(|c| c.1).max().unwrap()) / 2;
    s.step(&[Op::Explode { x: cx, y: cy, r: 10, power: 400, max_durability: 10 }]);
    for _ in 0..10 {
        s.step(&[]);
    }
    assert!(s.bodies().len() >= 2, "爆炸后应切成 ≥ 2 块，实际 {}", s.bodies().len());
    assert!(total_body_pixels(&s) < 24 * 16, "像素应有损失");
}

/// 盖章格照常参与 CA 燃烧（spec §3）：火贴着箱顶能点燃盖章格，counter > 0。
#[test]
fn stamped_cells_ignite_like_their_material() {
    let mut s = burn_body_sim(29);
    s.apply_setup(&[
        floor(128, 128),
        Op::SpawnBody { material: WOOD, x: 40, y: 108, w: 24, h: 16, angle_deg: 0 },
    ]);
    for _ in 0..60 {
        s.step(&[]);
    }
    // 箱子已静止；在箱顶上方一行放一排火（气体、rise 0.5 会逗留）
    let top = body_cells(&s).iter().map(|c| c.1).min().unwrap();
    s.step(&[Op::Fill { material: FIRE, x0: 44, y0: top - 1, x1: 59, y1: top - 1 }]);
    let mut lit = false;
    for _ in 0..200 {
        s.step(&[]);
        if body_cells(&s).iter().any(|&(x, y)| {
            let c = s.world().cell(x, y);
            c.is_body() && c.counter() > 0
        }) {
            lit = true;
            break;
        }
    }
    assert!(lit, "刚体盖章格必须能被点燃（counter > 0）");
}

/// 燃烧散架（验收 2）：木箱处在持续火场里（每 60 tick 在当前箱顶上方补一排火），
/// 像素单调减少、最终散架（body 全部消失）。
///
/// 为什么要持续供火：M2 的燃烧在**实心大块**上逐层推进时有概率断火（外壳烧尽后新一层
/// 只能靠火气体点燃，火寿命 40 + 单方向点燃骰），薄木构才自持烧净——那是 M2 燃烧
/// 参数的事；本测试证的是"烧掉像素 → 对账 → 重提取 → 散架"这条 M3 链路。
#[test]
fn burning_crate_shrinks_and_collapses() {
    let mut s = burn_body_sim(31);
    s.apply_setup(&[
        floor(128, 128),
        Op::SpawnBody { material: WOOD, x: 40, y: 108, w: 24, h: 16, angle_deg: 0 },
    ]);
    for _ in 0..60 {
        s.step(&[]);
    }
    let initial = total_body_pixels(&s);
    let mut last = initial;
    for t in 0..6000u64 {
        let ops = if t % 60 == 0 && !body_cells(&s).is_empty() {
            let cells = body_cells(&s);
            let top = cells.iter().map(|c| c.1).min().unwrap();
            let (x0, x1) = (cells.iter().map(|c| c.0).min().unwrap(), cells.iter().map(|c| c.0).max().unwrap());
            vec![Op::Fill { material: FIRE, x0, y0: top - 1, x1, y1: top - 1 }]
        } else {
            vec![]
        };
        s.step(&ops);
        let now = total_body_pixels(&s);
        assert!(now <= last, "tick {t}：刚体像素只减不增（{last} → {now}）");
        last = now;
        if now == 0 {
            break;
        }
    }
    assert_eq!(last, 0, "持续火场里的木箱应完全散架：{initial} → {last}");
    assert!(s.bodies().is_empty());
}

// ==================== Task 5：快照往返（验收 4）====================

/// `snapshot → restore → 继续 N tick` 与不恢复的孪生实例逐 tick `state_hash` 与引擎
/// checksum 逐位相同——恢复是无损的（M6 rollback 决策门的引擎侧前提）。
#[test]
fn physics_snapshot_restore_is_lossless() {
    let mut a = body_sim(37, 1);
    let mut b = body_sim(37, 1);
    let setup = [
        floor(128, 128),
        Op::SpawnBody { material: WOOD, x: 20, y: 40, w: 24, h: 16, angle_deg: 0 },
        Op::SpawnBody { material: STONE, x: 70, y: 20, w: 12, h: 12, angle_deg: 0 },
    ];
    a.apply_setup(&setup);
    b.apply_setup(&setup);
    for _ in 0..150 {
        a.step(&[]);
        b.step(&[]);
    }
    let snap = a.physics_snapshot();
    a.restore_physics(&snap).unwrap();
    for t in 0..300 {
        a.step(&[]);
        b.step(&[]);
        assert_eq!(a.state_hash(), b.state_hash(), "tick {t}：恢复后状态哈希分叉");
        assert_eq!(a.physics_checksum(), b.physics_checksum(), "tick {t}：恢复后引擎快照分叉");
    }
}

// ==================== 目检修订（2026-09-02）====================

/// 半悬空的箱子必须翻倒掉下（旋转发生：下落中 bbox 明显偏离 24×16）并离开台面。
#[test]
fn overhanging_crate_topples_off_ledge() {
    let mut s = body_sim(41, 1);
    s.apply_setup(&[
        floor(128, 128),
        Op::Fill { material: 1, x0: 20, y0: 90, x1: 60, y1: 123 }, // 台子
        Op::SpawnBody { material: WOOD, x: 50, y: 60, w: 24, h: 16, angle_deg: 0 }, // 一半悬在台沿外
    ]);
    let mut rotated = false;
    for _ in 0..300 {
        s.step(&[]);
        let cells = body_cells(&s);
        if cells.is_empty() {
            continue;
        }
        let (x0, x1) = (cells.iter().map(|c| c.0).min().unwrap(), cells.iter().map(|c| c.0).max().unwrap());
        let (y0, y1) = (cells.iter().map(|c| c.1).min().unwrap(), cells.iter().map(|c| c.1).max().unwrap());
        if x1 - x0 + 1 >= 27 && y1 - y0 + 1 >= 20 {
            rotated = true;
        }
    }
    assert!(rotated, "翻倒过程中必须发生旋转（bbox 明显变形）");
    let cells = body_cells(&s);
    let x0 = cells.iter().map(|c| c.0).min().unwrap();
    assert!(x0 > 60, "箱子应掉出台面（x0={x0}）");
}

/// 浮体最终静止入睡：稳定后 300 tick 内无粒子产生、脚印不变（不再周期性把水弹上箱顶）。
#[test]
fn floating_crate_settles_and_stops_ejecting_water() {
    let mut s = body_sim(17, 1);
    s.apply_setup(&[
        Op::Fill { material: 1, x0: 0, y0: 120, x1: 127, y1: 123 },
        Op::Fill { material: 1, x0: 10, y0: 60, x1: 11, y1: 119 },
        Op::Fill { material: 1, x0: 116, y0: 60, x1: 117, y1: 119 },
        Op::Fill { material: 3, x0: 12, y0: 80, x1: 115, y1: 119 },
        Op::SpawnBody { material: WOOD, x: 30, y: 40, w: 16, h: 12, angle_deg: 0 },
    ]);
    for _ in 0..900 {
        s.step(&[]);
    }
    let footprint = body_cells(&s);
    for t in 0..300 {
        s.step(&[]);
        assert_eq!(s.particles().len(), 0, "tick {}：稳定后不应再弹出粒子", 900 + t);
        assert_eq!(body_cells(&s), footprint, "tick {}：稳定后脚印不应变化", 900 + t);
    }
}

const WOOD_DEBRIS: u8 = 8;

/// 爆炸碎屑按 `debris_to` 以粉末落地：稳定后不存在任何"非刚体的静态木格"（那会悬空、
/// 粘在箱子上、还变成卡住刚体的地形），且确有木屑粉末落下。
#[test]
fn explosion_debris_lands_as_powder_not_static() {
    let t = MaterialTable::new(vec![
        MaterialDef::base(0, "air", Category::Static, 0),
        MaterialDef::base(1, "wall", Category::Static, 100),
        MaterialDef::base(2, "sand", Category::Powder, 40),
        MaterialDef::base(3, "water", Category::Liquid, 16),
        MaterialDef { debris_to: WOOD_DEBRIS, ..MaterialDef::base(WOOD, "wood", Category::Static, 12) },
        MaterialDef::base(STONE, "stone", Category::Static, 40),
        MaterialDef::base(FIRE, "fire", Category::Gas, 1),
        MaterialDef::base(SMOKE, "smoke", Category::Gas, 2),
        MaterialDef::base(WOOD_DEBRIS, "wood_debris", Category::Powder, 12),
    ])
    .unwrap();
    let r = ReactionTable::empty(&t);
    let mut s = sim_with_reactions(2, 2, 43, 1, ScanMode::LiveRect, t, r);
    s.apply_setup(&[floor(128, 128), Op::SpawnBody { material: WOOD, x: 40, y: 100, w: 24, h: 16, angle_deg: 0 }]);
    for _ in 0..200 {
        s.step(&[]);
    }
    let cells = body_cells(&s);
    let cx = (cells.iter().map(|c| c.0).min().unwrap() + cells.iter().map(|c| c.0).max().unwrap()) / 2;
    let cy = (cells.iter().map(|c| c.1).min().unwrap() + cells.iter().map(|c| c.1).max().unwrap()) / 2;
    s.step(&[Op::Explode { x: cx, y: cy, r: 10, power: 400, max_durability: 10 }]);
    for _ in 0..400 {
        s.step(&[]);
    }
    let body_set: std::collections::BTreeSet<(i32, i32)> = body_cells(&s).into_iter().collect();
    let mut static_wood_outside = 0;
    for y in 0..128 {
        for x in 0..128 {
            if s.world().cell(x, y).material() == WOOD && !body_set.contains(&(x, y)) {
                static_wood_outside += 1;
            }
        }
    }
    assert_eq!(static_wood_outside, 0, "不得留下悬空/粘连的静态木格");
    assert!(s.world().count_material(WOOD_DEBRIS) > 0, "碎屑应以粉末落地");
}

/// 斜着入水的长条被浮力扶正：32×6 木条以 35° 落进水塘，稳定后近乎水平漂浮
/// （脚印高度 ≤ 9 行；初始斜放 bbox 高约 22 行）。浮力施于淹没质心 ⇒ 扶正力矩（spec §5）。
#[test]
fn tilted_plank_rights_itself_in_water() {
    let mut s = body_sim(47, 1);
    s.apply_setup(&[
        Op::Fill { material: 1, x0: 0, y0: 120, x1: 127, y1: 123 },
        Op::Fill { material: 1, x0: 10, y0: 60, x1: 11, y1: 119 },
        Op::Fill { material: 1, x0: 116, y0: 60, x1: 117, y1: 119 },
        Op::Fill { material: 3, x0: 12, y0: 80, x1: 115, y1: 119 },
        Op::SpawnBody { material: WOOD, x: 48, y: 30, w: 32, h: 6, angle_deg: 35 },
    ]);
    s.step(&[]);
    let c0 = body_cells(&s);
    let h0 = c0.iter().map(|c| c.1).max().unwrap() - c0.iter().map(|c| c.1).min().unwrap() + 1;
    assert!(h0 >= 18, "初始应是斜放的（bbox 高 {h0}）");
    for _ in 0..1200 {
        s.step(&[]);
    }
    let c1 = body_cells(&s);
    // 旋转体的实心光栅化 = 面积 ± 边缘格（逆映射按格心判定），不要求恰等
    assert!((c1.len() as i32 - 32 * 6).abs() <= 12, "木条完整（{} 格）", c1.len());
    let (y0, y1) = (c1.iter().map(|c| c.1).min().unwrap(), c1.iter().map(|c| c.1).max().unwrap());
    assert!(y1 - y0 < 9, "稳定后应近乎水平漂浮（bbox 高 {}）", y1 - y0 + 1);
    assert!(y0 < 84 && y1 > 76, "应漂在水面 80 附近：{y0}..{y1}");
}

/// 回归（2026-09-03 目检：水里刚体越转越快）：crate_yard 的深水塘里 32×6 木条 35° 入水，
/// 角速度不得爬升——全程 |ω| 有界，稳定后角速度趋零且最终入睡。
///
/// 根因：刚体自己推起来的水堆（盖章排开的水以刚体线速度弹出、立刻落回迎水面，随刚体
/// 一起被抬着走）被 `surface_line` 采成"水面"，`h` 比真实水面高十几格 ⇒ 判成全淹没 ⇒
/// 以 g/3 向上猛推 ⇒ 弹出水面横拍回来 ⇒ 单端入水的大力臂把这份假势能全转成自旋；
/// 而阻力只阻线速度，自旋无处耗散。
#[test]
fn plank_in_deep_pool_does_not_spin_up() {
    let t = body_table();
    let r = ReactionTable::empty(&t);
    let mut s = sim_with_reactions(4, 3, 20260902, 1, ScanMode::LiveRect, t, r);
    s.apply_setup(&[
        Op::Fill { material: 1, x0: 0, y0: 180, x1: 255, y1: 191 },
        Op::Fill { material: 1, x0: 170, y0: 120, x1: 173, y1: 179 },
        Op::Fill { material: 1, x0: 246, y0: 120, x1: 249, y1: 179 },
        Op::Fill { material: 3, x0: 174, y0: 120, x1: 245, y1: 179 },
        Op::SpawnBody { material: WOOD, x: 190, y: 10, w: 32, h: 6, angle_deg: 35 },
    ]);
    let mut max_av = 0f32;
    let mut late_max_av = 0f32;
    for tick in 0..3000 {
        s.step(&[]);
        let (_, (_, av), _) = s.body_state(0).expect("木条应一直存在");
        max_av = max_av.max(av.abs());
        if tick >= 1500 {
            late_max_av = late_max_av.max(av.abs());
        }
    }
    assert!(max_av < 6.0, "全程角速度必须有界：max |ω| = {max_av:.2} rad/s");
    assert!(late_max_av < 0.5, "1500 tick 后应基本不转：max |ω| = {late_max_av:.2} rad/s");
    let ((_, y, _), _, sleeping) = s.body_state(0).unwrap();
    assert!(sleeping, "木条最终应入睡");
    assert!((110.0..=124.0).contains(&y), "木条应漂在水面附近：y = {y:.1}");
}

// ==================== 方案 1：接触门控水面线（2026-09-03，spec 决策记录第 14 条）====================

/// 水面线只能来自与刚体边界相接触的液体：地上的箱子旁边隔着玻璃壁的水槽不得给它浮力。
#[test]
fn crate_beside_glass_tank_stays_on_ground() {
    let mut s = body_sim(53, 1);
    s.apply_setup(&[
        floor(128, 128),
        Op::Fill { material: 1, x0: 58, y0: 60, x1: 59, y1: 123 }, // 玻璃壁，与箱子隔 2 格空气
        Op::Fill { material: 1, x0: 90, y0: 60, x1: 91, y1: 123 },
        Op::Fill { material: 3, x0: 60, y0: 80, x1: 89, y1: 123 }, // 水面 y=80
        Op::SpawnBody { material: WOOD, x: 40, y: 112, w: 16, h: 12, angle_deg: 0 },
    ]);
    for _ in 0..300 {
        s.step(&[]);
    }
    let ((_, y, _), _, sleeping) = s.body_state(0).unwrap();
    assert!((117.0..=119.0).contains(&y), "箱子应留在地上（箱心 y = {y:.1}，地上静止 = 118）");
    assert!(sleeping, "地上的箱子应入睡");
}

/// 架高水槽的底板与箱子等高、水在箱子旁边但隔着底板与壁：同样不得给浮力。
#[test]
fn crate_beside_shelf_tank_stays_on_ground() {
    let mut s = body_sim(59, 1);
    s.apply_setup(&[
        floor(128, 128),
        Op::Fill { material: 1, x0: 58, y0: 118, x1: 91, y1: 119 }, // 底板
        Op::Fill { material: 1, x0: 58, y0: 90, x1: 59, y1: 117 },
        Op::Fill { material: 1, x0: 90, y0: 90, x1: 91, y1: 117 },
        Op::Fill { material: 3, x0: 60, y0: 100, x1: 89, y1: 117 },
        Op::SpawnBody { material: WOOD, x: 40, y: 112, w: 16, h: 12, angle_deg: 0 },
    ]);
    for _ in 0..300 {
        s.step(&[]);
    }
    let ((_, y, _), _, sleeping) = s.body_state(0).unwrap();
    assert!((117.0..=119.0).contains(&y), "箱子应留在地上（箱心 y = {y:.1}）");
    assert!(sleeping, "地上的箱子应入睡");
}

/// 睡着的浮体在水退掉后必须醒来跟着降（2026-09-03 目检：炸穿池壁后木箱挂在半空）：
/// 木箱浮稳入睡 → 抽掉它脚下的水（水面从 80 降到 ≈105）→ 木箱应落到新水面并再次入睡。
#[test]
fn sleeping_floater_wakes_when_water_drains() {
    let mut s = body_sim(61, 1);
    s.apply_setup(&[
        Op::Fill { material: 1, x0: 0, y0: 120, x1: 127, y1: 123 },
        Op::Fill { material: 1, x0: 10, y0: 60, x1: 11, y1: 119 },
        Op::Fill { material: 1, x0: 116, y0: 60, x1: 117, y1: 119 },
        Op::Fill { material: 3, x0: 12, y0: 80, x1: 115, y1: 119 },
        Op::SpawnBody { material: WOOD, x: 30, y: 40, w: 16, h: 12, angle_deg: 0 },
    ]);
    for _ in 0..900 {
        s.step(&[]);
    }
    let ((_, y0, _), _, sleeping) = s.body_state(0).unwrap();
    assert!(sleeping, "浮稳后应入睡");
    assert!((78.0..=90.0).contains(&y0), "应浮在水面 80 附近：y = {y0:.1}");
    // 抽水：把箱子下方 y=95..119 的水换成空气（上面的水会塌下去，水面降到 ≈105）
    s.step(&[Op::Fill { material: 0, x0: 12, y0: 95, x1: 115, y1: 119 }]);
    for _ in 0..1500 {
        s.step(&[]);
    }
    let ((_, y1, _), _, sleeping) = s.body_state(0).unwrap();
    assert!(y1 > y0 + 10.0, "水退后箱子必须跟着降：{y0:.1} → {y1:.1}");
    assert!(y1 < 118.0, "不该沉到池底：y = {y1:.1}");
    assert!(sleeping, "落到新水面后应再次入睡");
}

// ==================== 浮力尾巴（2026-09-03，spec 决策记录第 16 条）====================

/// 一股水从旁边落下擦过地上的箱子：下落中的水（`vel` ≥ 2 格/tick）不算接触，箱子不得被抬。
#[test]
fn water_stream_beside_crate_gives_no_lift() {
    let mut s = body_sim(67, 1);
    s.apply_setup(&[
        floor(128, 128),
        Op::Fill { material: 3, x0: 58, y0: 20, x1: 58, y1: 100 },
        Op::SpawnBody { material: WOOD, x: 40, y: 112, w: 16, h: 12, angle_deg: 0 },
    ]);
    let mut min_y = f32::MAX;
    for _ in 0..300 {
        s.step(&[]);
        let ((_, y, _), _, _) = s.body_state(0).unwrap();
        min_y = min_y.min(y);
    }
    assert!(min_y > 117.4, "落水流不得抬起箱子：最高到 y = {min_y:.1}（地上静止 = 118）");
}

/// 卡在同宽槽里的木箱压着一段封闭水柱：不可压缩的水托住它（密封支撑），不沉到槽底。
#[test]
fn plug_in_sealed_channel_rests_on_trapped_water() {
    let mut s = body_sim(71, 1);
    s.apply_setup(&[
        floor(128, 128),
        Op::Fill { material: 1, x0: 38, y0: 60, x1: 39, y1: 123 },
        Op::Fill { material: 1, x0: 56, y0: 60, x1: 57, y1: 123 },
        Op::Fill { material: 3, x0: 40, y0: 100, x1: 55, y1: 123 },
        Op::SpawnBody { material: WOOD, x: 40, y: 60, w: 16, h: 12, angle_deg: 0 },
    ]);
    for _ in 0..900 {
        s.step(&[]);
    }
    let ((_, y, _), _, sleeping) = s.body_state(0).unwrap();
    assert!((90.0..=100.0).contains(&y), "塞子应停在水柱顶（水面 100，箱心 ≈ 94）：y = {y:.1}");
    assert!(sleeping, "停稳后应入睡");
}

/// 木条架在浮着的木箱上：木条的侧面碰不到水，不得被旁边的池水"抬"起来。
#[test]
fn plank_resting_on_floating_crate_is_not_lifted() {
    let t = body_table();
    let r = ReactionTable::empty(&t);
    let mut s = sim_with_reactions(4, 3, 73, 1, ScanMode::LiveRect, t, r);
    s.apply_setup(&[
        Op::Fill { material: 1, x0: 0, y0: 180, x1: 255, y1: 191 },
        Op::Fill { material: 1, x0: 170, y0: 120, x1: 173, y1: 179 },
        Op::Fill { material: 1, x0: 246, y0: 120, x1: 249, y1: 179 },
        Op::Fill { material: 3, x0: 174, y0: 120, x1: 245, y1: 179 },
        Op::SpawnBody { material: WOOD, x: 185, y: 40, w: 20, h: 14, angle_deg: 0 },
    ]);
    for _ in 0..600 {
        s.step(&[]);
    }
    let ((cx, cy, _), _, _) = s.body_state(0).unwrap();
    // 木条水平地落在木箱正上方（两端各悬出 6 格）
    s.step(&[Op::SpawnBody { material: WOOD, x: cx as i32 - 16, y: cy as i32 - 30, w: 32, h: 6, angle_deg: 0 }]);
    for _ in 0..900 {
        s.step(&[]);
    }
    let ((_, cy1, _), _, _) = s.body_state(0).unwrap();
    let ((_, py, _), _, _) = s.body_state(1).unwrap();
    assert!(py >= cy1 - 12.0, "木条不得浮到木箱之上：木条 y = {py:.1}，木箱 y = {cy1:.1}");
    assert!(py <= cy1 + 8.0, "木条应仍在木箱附近（架着或滑到旁边）：木条 y = {py:.1}，木箱 y = {cy1:.1}");
}

/// 顶面载荷：往浮稳的木箱顶上倒 4 行水，箱子被压下去；水滑掉后回到原来的吃水并入睡。
#[test]
fn heap_on_floating_crate_pushes_it_down() {
    let mut s = body_sim(79, 1);
    s.apply_setup(&[
        Op::Fill { material: 1, x0: 0, y0: 120, x1: 127, y1: 123 },
        Op::Fill { material: 1, x0: 10, y0: 60, x1: 11, y1: 119 },
        Op::Fill { material: 1, x0: 116, y0: 60, x1: 117, y1: 119 },
        Op::Fill { material: 3, x0: 12, y0: 80, x1: 115, y1: 119 },
        Op::SpawnBody { material: WOOD, x: 30, y: 40, w: 16, h: 12, angle_deg: 0 },
    ]);
    for _ in 0..900 {
        s.step(&[]);
    }
    let ((_, y0, _), _, _) = s.body_state(0).unwrap();
    let cells = body_cells(&s);
    let (x0, x1) = (cells.iter().map(|c| c.0).min().unwrap(), cells.iter().map(|c| c.0).max().unwrap());
    let top = cells.iter().map(|c| c.1).min().unwrap();
    s.step(&[Op::Fill { material: 3, x0, y0: top - 4, x1, y1: top - 1 }]);
    let mut max_y = f32::MIN;
    for _ in 0..120 {
        s.step(&[]);
        let ((_, y, _), _, _) = s.body_state(0).unwrap();
        max_y = max_y.max(y);
    }
    assert!(max_y > y0 + 1.0, "顶上 4 行水应把箱子压下去：{y0:.1} → 最深 {max_y:.1}");
    for _ in 0..900 {
        s.step(&[]);
    }
    let ((_, y1, _), _, sleeping) = s.body_state(0).unwrap();
    assert!((y1 - y0).abs() < 2.5, "水滑掉后应回到原吃水附近：{y0:.1} → {y1:.1}");
    assert!(sleeping, "应再次入睡");
}
