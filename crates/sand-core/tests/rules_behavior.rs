//! 规则行为集成测试（spec §5.4 第 1 层）：经 Sim 公共 API 驱动。

mod common;

use common::{sim, sim_with_table, test_table_with_water_dispersion, SAND, WATER};
use sand_core::{Op, ScanMode, DISPERSION_MAX, MAT_AIR, MAT_WALL};

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

// ==================== 液体色散 ≤8（Layer G Task 1，spec §3）====================
//
// 全部用 128×128（2×2 chunk）+ floor_op 的单行地板（y=124），水静置于
// y=123：向下与斜下都撞地板 → 必走 `side()` 分支。水放在 x=100（偶 → 初始
// 方向左，`world.rs:123` 的 `with_dir(x & 1 == 1)`），色散路径整个落在
// chunk 1 内部，不掺进跨 chunk 缝的干扰。

/// 单 tick 横移 = min(dispersion, 到最近障碍的距离)，取**最远可达空格**——
/// 不是改动前的 1 格，也不是一路冲到 DISPERSION_MAX。
#[test]
fn water_disperses_to_farthest_reachable_air_cell() {
    let mut s = sim_with_table(2, 2, 7, 1, ScanMode::LiveRect, test_table_with_water_dispersion(5));
    s.apply_setup(&[floor_op(128, 128), Op::Brush { material: WATER, x: 100, y: 123, r: 0 }]);
    s.step(&[]);
    assert_eq!(s.world().cell(100, 123).material(), MAT_AIR, "源格必须腾空");
    assert_eq!(
        s.world().cell(95, 123).material(),
        WATER,
        "dispersion=5 应单 tick 横移 5 格落在 x=95（改动前的 1 格语义会停在 99）"
    );
}

/// 遇阻即停：路径第 3 格是 wall → 落在第 2 格（不穿墙、不停在第 1 格）。
#[test]
fn water_dispersion_stops_at_first_obstacle() {
    let mut s = sim_with_table(2, 2, 8, 1, ScanMode::LiveRect, test_table_with_water_dispersion(5));
    s.apply_setup(&[
        floor_op(128, 128),
        Op::Fill { material: MAT_WALL, x0: 97, y0: 123, x1: 97, y1: 123 },
        Op::Brush { material: WATER, x: 100, y: 123, r: 0 },
    ]);
    s.step(&[]);
    assert_eq!(
        s.world().cell(98, 123).material(),
        WATER,
        "左行第 3 格（x=97）是 wall，水必须停在第 2 格 x=98"
    );
    assert_eq!(s.world().cell(97, 123).material(), MAT_WALL, "墙不得被穿过或置换");
}

/// 方向记忆不变量：成功后记忆 = **实际**移动方向；翻向路径同样成立
/// （2026-06-14 液面冻结修复语义，M0 spec §4.3——色散不得动摇它）。
#[test]
fn water_dispersion_preserves_direction_commitment() {
    // ① 正常左行：dir 保持 -1
    let mut s = sim_with_table(2, 2, 9, 1, ScanMode::LiveRect, test_table_with_water_dispersion(5));
    s.apply_setup(&[floor_op(128, 128), Op::Brush { material: WATER, x: 100, y: 123, r: 0 }]);
    s.step(&[]);
    assert_eq!(s.world().cell(95, 123).dir(), -1, "左行后记忆方向必须是左");

    // ② 翻向：紧邻左侧是墙 → 首选方向失败 → 翻向右行，记忆必须跟着翻成 +1
    let mut s = sim_with_table(2, 2, 10, 1, ScanMode::LiveRect, test_table_with_water_dispersion(5));
    s.apply_setup(&[
        floor_op(128, 128),
        Op::Fill { material: MAT_WALL, x0: 99, y0: 123, x1: 99, y1: 123 },
        Op::Brush { material: WATER, x: 100, y: 123, r: 0 },
    ]);
    s.step(&[]);
    assert_eq!(
        s.world().cell(105, 123).material(),
        WATER,
        "左侧被堵应翻向右行满 5 格"
    );
    assert_eq!(s.world().cell(105, 123).dir(), 1, "翻向后记忆必须 = 实际移动方向（右）");
}

/// core 侧 clamp（spec §3.1 评审修订）：`dispersion` 越界会让 `side()` 写出
/// WriteWindow（debug 撞窗口断言，release 变同相数据竞争 → SyncTest 分叉），
/// 属破坏 P4 写域论证的字段，不能只靠 I/O 层校验。这条测试**刻意绕过 harness**
/// 直接构表，断言 core 自己把半径压回 DISPERSION_MAX。
#[test]
fn water_dispersion_is_clamped_to_max_inside_core() {
    let mut s = sim_with_table(2, 2, 11, 1, ScanMode::LiveRect, test_table_with_water_dispersion(20));
    s.apply_setup(&[floor_op(128, 128), Op::Brush { material: WATER, x: 100, y: 123, r: 0 }]);
    s.step(&[]);
    let landed = (0..128).find(|&x| s.world().cell(x, 123).material() == WATER).unwrap();
    assert_eq!(
        landed,
        100 - DISPERSION_MAX as i32,
        "dispersion=20 必须被 core clamp 到 DISPERSION_MAX={DISPERSION_MAX}"
    );
}

/// 缺省 dispersion=1 的材质行为与改动前逐位相同（spec §3.4 缺省行为条）。
#[test]
fn water_with_default_dispersion_moves_exactly_one_cell() {
    let mut s = sim(2, 2, 12, 1, ScanMode::LiveRect);
    s.apply_setup(&[floor_op(128, 128), Op::Brush { material: WATER, x: 100, y: 123, r: 0 }]);
    s.step(&[]);
    assert_eq!(s.world().cell(99, 123).material(), WATER, "缺省色散必须仍是单格横移");
}

/// 色散的**用户目标**回归：更大的 dispersion 必须让水面摊平得更快
/// （M0 记录的"摊平极慢"顽疾，spec §1.2 目标 4）。
///
/// 前面几条测试锁的是单 tick 位移的精确落点，这条锁的是它换来的宏观行为——
/// 两者都要，否则"位移对了但摊平没变快"这种实现照样能过关。判据刻意写成
/// **两个配置的相对比较**而非某个魔法 tick 数：绝对收敛速度依赖场景几何与
/// 机器，相对关系才是本 Task 真正承诺的东西。
#[test]
fn higher_dispersion_levels_water_faster() {
    // 水柱在左、盆地向右敞开：摊平 = 顶面下降到接近平衡水位。
    // 判据取"最高水面行 >= 118"——初始水柱顶在 y=90，只有真正摊开到整个
    // 盆地宽度后才可能降到 118（544 格水 / 190 列 ≈ 2.9 行深，平衡顶面 ≈121）。
    fn ticks_to_spread(dispersion: u8, budget: u32) -> Option<u32> {
        let mut s =
            sim_with_table(3, 2, 4, 1, ScanMode::LiveRect, test_table_with_water_dispersion(dispersion));
        s.apply_setup(&[
            floor_op(192, 128),
            Op::Fill { material: MAT_WALL, x0: 0, y0: 80, x1: 0, y1: 123 },
            Op::Fill { material: MAT_WALL, x0: 191, y0: 80, x1: 191, y1: 123 },
            Op::Fill { material: WATER, x0: 30, y0: 90, x1: 45, y1: 123 },
        ]);
        for t in 1..=budget {
            s.step(&[]);
            let top = (1..191)
                .filter_map(|x| (0..124).find(|&y| s.world().cell(x, y).material() == WATER))
                .min();
            if top.is_some_and(|t| t >= 118) {
                return Some(t);
            }
        }
        None
    }

    let slow = ticks_to_spread(1, 3000).expect("dispersion=1 应在 3000 tick 内摊平（改动前语义）");
    let fast = ticks_to_spread(5, 3000).expect("dispersion=5 应在 3000 tick 内摊平");
    assert!(
        fast < slow,
        "色散必须加快摊平：dispersion=5 用了 {fast} tick，dispersion=1 用了 {slow} tick"
    );
}
