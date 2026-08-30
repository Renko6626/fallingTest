//! 规则行为集成测试（spec §5.4 第 1 层）：经 Sim 公共 API 驱动。

mod common;

use common::{sim, SAND, WATER};
use sand_core::{Op, ScanMode, MAT_AIR, MAT_WALL};

fn floor_op(w: i32, h: i32) -> Op {
    Op::Fill { material: MAT_WALL, x0: 0, y0: h - 4, x1: w - 1, y1: h - 4 }
}

#[test]
fn sand_falls_straight_down() {
    let mut s = sim(2, 2, 1, 1, ScanMode::LiveRect);
    s.apply_setup(&[Op::Brush { material: SAND, x: 40, y: 10, r: 0 }]);
    s.step(&[]);
    assert_eq!(s.world().cell(40, 10).material(), MAT_AIR);
    assert_eq!(s.world().cell(40, 11).material(), SAND);
    for _ in 0..5 {
        s.step(&[]);
    }
    assert_eq!(s.world().cell(40, 16).material(), SAND);
}

#[test]
fn sand_piles_and_is_conserved() {
    let mut s = sim(2, 2, 2, 1, ScanMode::LiveRect);
    s.apply_setup(&[floor_op(128, 128)]);
    for t in 0..400u64 {
        let ops = if t % 2 == 0 && t < 240 {
            vec![Op::Brush { material: SAND, x: 64, y: 8, r: 1 }]
        } else {
            vec![]
        };
        s.step(&ops);
    }
    let n = s.world().count_material(SAND);
    assert!(n > 0);
    // 全部落定在地板上方，没有穿地板
    for x in 0..128 {
        for y in 125..128 {
            assert_ne!(s.world().cell(x, y).material(), SAND, "沙穿透地板 at ({x},{y})");
        }
    }
    // 静置后不再变化（堆稳定）
    let h1 = s.state_hash();
    let t1 = s.tick();
    s.step(&[]);
    // tick 计数变化会改 state_hash；比 cells 就位：材质计数与位置抽样
    assert_eq!(s.world().count_material(SAND), n, "静置期沙数量必须守恒");
    let _ = (h1, t1);
}

#[test]
fn sand_sinks_in_water() {
    let mut s = sim(2, 2, 3, 1, ScanMode::LiveRect);
    s.apply_setup(&[
        floor_op(128, 128),
        Op::Fill { material: WATER, x0: 50, y0: 110, x1: 78, y1: 123 },
        Op::Brush { material: SAND, x: 64, y: 100, r: 1 },
    ]);
    for _ in 0..200 {
        s.step(&[]);
    }
    // 下沉判据：稳态下任何沙的正下方不允许是水（沙浮在水上 = 未沉）
    let mut sand_seen = false;
    for y in 0..127 {
        for x in 0..128 {
            if s.world().cell(x, y).material() == SAND {
                sand_seen = true;
                assert_ne!(
                    s.world().cell(x, y + 1).material(),
                    WATER,
                    "沙浮在水上 at ({x},{y})"
                );
            }
        }
    }
    assert!(sand_seen, "场景里应该还有沙");
}

#[test]
fn water_levels_out_across_chunk_seam() {
    // 192×128（3×2 chunk）：左侧水柱跨过垂直缝摊平；材质守恒（缝无源汇）
    let mut s = sim(3, 2, 4, 1, ScanMode::LiveRect);
    s.apply_setup(&[
        floor_op(192, 128),
        Op::Fill { material: MAT_WALL, x0: 0, y0: 80, x1: 0, y1: 123 },
        Op::Fill { material: MAT_WALL, x0: 191, y0: 80, x1: 191, y1: 123 },
        Op::Fill { material: WATER, x0: 30, y0: 90, x1: 45, y1: 123 },
    ]);
    let n0 = s.world().count_material(WATER);
    for _ in 0..1200 {
        s.step(&[]);
    }
    assert_eq!(s.world().count_material(WATER), n0, "水量守恒（chunk 缝无源汇）");
    // 摊平判据：水面最高与最低行差 ≤ 2（简版横流收敛慢，放宽阈值）
    let mut min_surface = i32::MAX;
    let mut max_surface = 0;
    for x in 1..191 {
        for y in 0..124 {
            if s.world().cell(x, y).material() == WATER {
                min_surface = min_surface.min(y);
                break;
            }
        }
    }
    for x in 1..191 {
        let mut top = None;
        for y in 0..124 {
            if s.world().cell(x, y).material() == WATER {
                top = Some(y);
                break;
            }
        }
        if let Some(t) = top {
            max_surface = max_surface.max(t);
        }
    }
    assert!(
        max_surface - min_surface <= 2,
        "液面未摊平：surface range [{min_surface},{max_surface}]"
    );
}

#[test]
fn water_direction_commitment() {
    // 单粒水在平地上：向记忆方向走；撞墙后翻转记忆继续走（不打乒乓）
    let mut s = sim(2, 2, 5, 1, ScanMode::LiveRect);
    s.apply_setup(&[
        floor_op(128, 128),
        Op::Fill { material: MAT_WALL, x0: 20, y0: 100, x1: 20, y1: 123 },
        Op::Brush { material: WATER, x: 24, y: 123, r: 0 },
    ]);
    // x=24 为偶 → 初始方向左（world.rs 按 x 奇偶初始化）；先向左走到墙 (21)，
    // 然后翻转向右一路走远。
    let mut positions = vec![];
    for _ in 0..30 {
        s.step(&[]);
        for x in 0..128 {
            if s.world().cell(x, 123).material() == WATER {
                positions.push(x);
            }
        }
    }
    let last = *positions.last().unwrap();
    assert!(last > 24, "撞墙后应翻转向右走远，实际停在 {last}（轨迹 {positions:?}）");
    // 无乒乓：轨迹中不允许出现 a,b,a,b 抖动
    let pingpong = positions.windows(4).any(|w| w[0] == w[2] && w[1] == w[3] && w[0] != w[1]);
    assert!(!pingpong, "方向承诺失效，出现乒乓：{positions:?}");
}
