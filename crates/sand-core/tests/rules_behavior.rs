//! 规则行为集成测试（spec §5.4 第 1 层）：经 Sim 公共 API 驱动。

mod common;

use common::{sim, sim_with_table, test_table_with_splash, test_table_with_water_dispersion, SAND, WATER};
use sand_core::{Fx, Op, ScanMode, DISPERSION_MAX, G_ACCEL, MAT_AIR, MAT_WALL, VEL_ONE, V_MAX_CELL};

fn floor_op(w: i32, h: i32) -> Op {
    Op::Fill { material: MAT_WALL, x0: 0, y0: h - 4, x1: w - 1, y1: h - 4 }
}

/// 自由下落全程不横漂（下方为空时永远走正下方分支）。落点随 Layer G Task 2
/// 的速度积分变化，故精确落点交给 `falling_sand_starts_at_exactly_one_cell_per_tick`
/// 与 `free_fall_reaches_terminal_velocity_at_tick_16`，这里只锁"竖直"这一条。
#[test]
fn sand_falls_straight_down() {
    let mut s = sim(2, 2, 1, 1, ScanMode::LiveRect);
    s.apply_setup(&[Op::Brush { material: SAND, x: 40, y: 10, r: 0 }]);
    s.step(&[]);
    assert_eq!(s.world().cell(40, 10).material(), MAT_AIR);
    assert_eq!(s.world().cell(40, 11).material(), SAND);
    for _ in 0..5 {
        s.step(&[]);
        for x in 0..128 {
            for y in 0..128 {
                if s.world().cell(x, y).material() == SAND {
                    assert_eq!(x, 40, "自由下落不得横漂：沙出现在 x={x}");
                    assert!(y > 10, "沙必须持续下落");
                }
            }
        }
    }
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

// ==================== 重力速度积分（Layer G Task 2，spec §4）====================
//
// 语义：每 tick `v ← min(v + G_ACCEL, V_MAX_CELL)`（Q3.2，单位 ¼ 格/tick），
// 子步数 `n = max(1, v/VEL_ONE + frac_roll)`，撞停（Blocked / MovedSide）清零。
// `v = 0` 时 n = 1 ⇒ 退化为 Task 2 之前的语义（spec §4.2①）。

/// 从静止起的**头 4 tick** 必然是每 tick 恰好 1 格：v1 ≤ VEL_ONE ⇒ n = 1，
/// 与概率取整无关，是可退化性（spec §4.2①）在行为层的直接体现。
#[test]
fn falling_sand_starts_at_exactly_one_cell_per_tick() {
    let mut s = sim(2, 2, 21, 1, ScanMode::LiveRect);
    s.apply_setup(&[Op::Brush { material: SAND, x: 40, y: 10, r: 0 }]);
    for t in 1..=4i32 {
        s.step(&[]);
        assert_eq!(
            s.world().cell(40, 10 + t).material(),
            SAND,
            "第 {t} tick 应恰好落 1 格（v1 = {t}/4 格 < 1 ⇒ n = 1）"
        );
    }
}

/// 自由落体 16 tick 后达终端速度，且**第 16 tick 才首次达到**（G_ACCEL = 1
/// ⇒ V_MAX_CELL / G_ACCEL = 16 tick）。这条同时锁死 `vel()` 位段真的被写回。
#[test]
fn free_fall_reaches_terminal_velocity_at_tick_16() {
    let mut s = sim(2, 2, 22, 1, ScanMode::LiveRect);
    s.apply_setup(&[Op::Brush { material: SAND, x: 40, y: 2, r: 0 }]);
    let find = |s: &sand_core::Sim| (0..128).find(|&y| s.world().cell(40, y).material() == SAND);
    for _ in 0..15 {
        s.step(&[]);
    }
    let y15 = find(&s).expect("沙还在下落中");
    assert_eq!(
        s.world().cell(40, y15).vel(),
        15 * G_ACCEL,
        "第 15 tick 速度应为 15 个 ¼ 格单位，尚未封顶"
    );
    s.step(&[]);
    let y16 = find(&s).expect("沙还在下落中");
    assert_eq!(s.world().cell(40, y16).vel(), V_MAX_CELL, "第 16 tick 应首次达终端速度");
    s.step(&[]);
    let y17 = find(&s).expect("沙还在下落中");
    assert_eq!(s.world().cell(40, y17).vel(), V_MAX_CELL, "终端速度必须被 clamp 住");
    assert_eq!(y17 - y16, (V_MAX_CELL / VEL_ONE) as i32, "终端速度下每 tick 恰好 4 格");
}

/// 加速的宏观后果：同样 20 tick，加速后的落距必须显著超过匀速 1 格/tick。
#[test]
fn gravity_makes_sand_fall_farther_than_one_cell_per_tick() {
    let mut s = sim(2, 2, 23, 1, ScanMode::LiveRect);
    s.apply_setup(&[Op::Brush { material: SAND, x: 40, y: 2, r: 0 }]);
    for _ in 0..20 {
        s.step(&[]);
    }
    let y = (0..128).find(|&y| s.world().cell(40, y).material() == SAND).unwrap();
    assert!(y - 2 > 20, "20 tick 落距 {} 格，加速后必须 > 20 格", y - 2);
}

/// 撞停清零（spec §4.1 `v_final = 0`）：高速下落砸到地板后速度必须归零，
/// 否则休眠不变量与后续 Task 3 的溅射阈值全部失效。
#[test]
fn landing_resets_velocity_to_zero() {
    let mut s = sim(2, 2, 24, 1, ScanMode::LiveRect);
    s.apply_setup(&[floor_op(128, 128), Op::Brush { material: SAND, x: 40, y: 2, r: 0 }]);
    for _ in 0..80 {
        s.step(&[]);
    }
    assert_eq!(s.world().cell(40, 123).material(), SAND, "沙应停在地板上方");
    assert_eq!(s.world().cell(40, 123).vel(), 0, "撞停后速度必须清零");
}

/// 子步循环逐格判定，**不得穿透**单格厚的地板（n 最大 4 ⇒ 若写成"直接跳 n 格"
/// 就会漏检中途格子）。这是速度积分最危险的实现错误。
#[test]
fn fast_fall_does_not_tunnel_through_thin_floor() {
    let mut s = sim(2, 2, 25, 1, ScanMode::LiveRect);
    s.apply_setup(&[
        Op::Fill { material: MAT_WALL, x0: 0, y0: 60, x1: 127, y1: 60 },
        Op::Brush { material: SAND, x: 40, y: 2, r: 0 },
    ]);
    for _ in 0..120 {
        s.step(&[]);
    }
    assert_eq!(s.world().cell(40, 59).material(), SAND, "沙必须停在地板正上方 y=59");
    assert_eq!(s.world().cell(40, 60).material(), MAT_WALL, "地板不得被穿过");
    for y in 61..128 {
        assert_ne!(s.world().cell(40, y).material(), SAND, "沙穿透了单格地板，落到 y={y}");
    }
}

/// 休眠不变量（验收 §0 第 4 项）：静置沙堆跑满 N tick 后，所有 chunk 的
/// `dirty` / `next_dirty` 必须恒空。若照 jason.today 原样"每 tick 无条件
/// `v += accel` 写回"，静止沙的速度会从 0 涨起来 → 每 tick 一次 `set()` →
/// `mark_dirty_around` → 整张图永不入睡，M0 的稀疏性能当场退回全量扫描
/// （spec §4.2②）。
#[test]
fn resting_pile_lets_every_chunk_sleep() {
    let mut s = sim(2, 2, 26, 1, ScanMode::LiveRect);
    s.apply_setup(&[
        floor_op(128, 128),
        Op::Fill { material: SAND, x0: 40, y0: 118, x1: 80, y1: 123 },
    ]);
    for _ in 0..400 {
        s.step(&[]);
    }
    for (ci, c) in s.world().chunks.iter().enumerate() {
        assert!(c.dirty.is_empty(), "chunk {ci} 静置后仍脏：{:?}", c.dirty);
        assert!(
            c.next_dirty.snapshot().is_empty(),
            "chunk {ci} 静置后 next_dirty 非空：{:?}",
            c.next_dirty.snapshot()
        );
    }
}

/// r ≤ 16 写域执法（spec §5）：最坏路径 = (n−1) 次斜下 + 1 次满色散 = 11 格
/// 水平位移。本测试让**高速水**在满色散（DISPERSION_MAX）下跨 chunk 缝乱跑，
/// 靠 `WriteWindow` 的 debug 断言兜底——写出窗口即 panic。
#[test]
fn fast_water_at_max_dispersion_stays_inside_write_window() {
    let mut s = sim_with_table(
        3,
        2,
        27,
        1,
        ScanMode::LiveRect,
        test_table_with_water_dispersion(DISPERSION_MAX),
    );
    s.apply_setup(&[
        floor_op(192, 128),
        Op::Fill { material: MAT_WALL, x0: 0, y0: 60, x1: 0, y1: 123 },
        Op::Fill { material: MAT_WALL, x0: 191, y0: 60, x1: 191, y1: 123 },
        Op::Fill { material: WATER, x0: 60, y0: 2, x1: 100, y1: 40 },
        Op::Fill { material: SAND, x0: 120, y0: 2, x1: 140, y1: 20 },
    ]);
    for _ in 0..300 {
        s.step(&[]);
    }
    assert!(s.world().count_material(WATER) > 0, "水不该凭空消失");
}

// ==================== 撞击溅射脱格（Layer G Task 3，spec §6）====================
//
// 三条触发条件全中才脱格：① 本 tick 撞停（`Blocked` 或 `MovedSide`）；
// ② `v1 >= SPLASH_MIN_SPEED`；③ 概率骰命中 `splash_chance`。
// 下面每条各有一个"不满足即不溅射"的反面测试——只测正面的话，一个恒真的
// 实现照样全绿。
//
// 场景约定：深井（左右墙 + 地板）里丢一格水，落差足够吃满终端速度，
// 三个方向全被挡 ⇒ `Blocked` 撞停。

/// 深井：左右墙 + 地板，井口在 y0，水从井口落到地板。
fn well_ops(w: i32, h: i32, x: i32, y0: i32) -> Vec<Op> {
    vec![
        Op::Fill { material: MAT_WALL, x0: 0, y0: h - 4, x1: w - 1, y1: h - 4 },
        Op::Fill { material: MAT_WALL, x0: x - 1, y0, x1: x - 1, y1: h - 5 },
        Op::Fill { material: MAT_WALL, x0: x + 1, y0, x1: x + 1, y1: h - 5 },
    ]
}

/// 正面：高速水撞底 → 网格格变 air、粒子 +1（质量账对齐），且粒子向上飞。
#[test]
fn fast_impact_ejects_a_splash_particle() {
    let mut s = sim_with_table(2, 2, 41, 1, ScanMode::LiveRect, test_table_with_splash(255, 0));
    let mut ops = well_ops(128, 128, 40, 80);
    ops.push(Op::Brush { material: WATER, x: 40, y: 81, r: 0 });
    s.apply_setup(&ops);
    for _ in 0..60 {
        s.step(&[]);
        if !s.particles().is_empty() {
            break;
        }
    }
    assert_eq!(s.particles().len(), 1, "撞停必溅射（splash_chance 量化 255）");
    assert_eq!(s.world().count_material(WATER), 0, "脱格后网格里不该再有水");
    assert!(s.particles().vy(0).0 < 0, "溅射粒子必须向上飞（Fx 的 y 轴向下为正）");
}

/// 反面①：还在自由下落（`stalled == false`）就不该溅射，哪怕速度已封顶。
#[test]
fn free_falling_cell_never_splashes() {
    let mut s = sim_with_table(2, 2, 42, 1, ScanMode::LiveRect, test_table_with_splash(255, 255));
    s.apply_setup(&[Op::Brush { material: WATER, x: 40, y: 2, r: 0 }]);
    for _ in 0..20 {
        s.step(&[]);
        assert_eq!(s.particles().len(), 0, "下落途中不得溅射");
    }
}

/// 反面②：速度不足 `SPLASH_MIN_SPEED` 的撞停不溅射——水直接放在地板上，
/// 第一 tick 就 `Blocked`，`v1 = G_ACCEL = 1` 远低于阈值。
#[test]
fn slow_impact_does_not_splash() {
    let mut s = sim_with_table(2, 2, 43, 1, ScanMode::LiveRect, test_table_with_splash(255, 0));
    let mut ops = well_ops(128, 128, 40, 100);
    ops.push(Op::Brush { material: WATER, x: 40, y: 123, r: 0 });
    s.apply_setup(&ops);
    for _ in 0..20 {
        s.step(&[]);
    }
    assert_eq!(s.particles().len(), 0, "低速撞停不得溅射");
    assert_eq!(s.world().count_material(WATER), 1, "水必须原地留在网格里");
}

/// 反面③：`splash_chance = 0`（缺省）的材质永不溅射，哪怕高速撞停。
#[test]
fn zero_splash_chance_never_splashes() {
    let mut s = sim_with_table(2, 2, 44, 1, ScanMode::LiveRect, test_table_with_splash(0, 0));
    let mut ops = well_ops(128, 128, 40, 80);
    ops.push(Op::Brush { material: WATER, x: 40, y: 81, r: 0 });
    s.apply_setup(&ops);
    for _ in 0..60 {
        s.step(&[]);
    }
    assert_eq!(s.particles().len(), 0, "splash_chance=0 必须永不溅射");
    assert_eq!(s.world().count_material(WATER), 1, "水必须留在网格里");
}

/// 概率是**逐格独立**的，不是全有或全无：一整行水同时砸地，
/// `splash_chance ≈ 0.5` 时脱格数量必须落在两端之间。
///
/// 这条锁的正是 §6.1③ 的 RNG key 选择——若 key 取撞停坐标而非起始坐标，
/// 同 tick 落进同一格的连锁会掷出同值，整列同进同退。
#[test]
fn splash_probability_is_per_cell_not_all_or_nothing() {
    let mut s = sim_with_table(2, 2, 45, 1, ScanMode::LiveRect, test_table_with_splash(128, 0));
    let mut ops = vec![Op::Fill { material: MAT_WALL, x0: 0, y0: 124, x1: 127, y1: 124 }];
    ops.push(Op::Fill { material: WATER, x0: 10, y0: 80, x1: 109, y1: 80 });
    s.apply_setup(&ops);
    // 数**峰值**而非终值：溅射粒子几 tick 内就落回网格（Layer P 的落格闭环），
    // 跑完再数必然是 0。
    let mut peak = 0usize;
    for _ in 0..80 {
        s.step(&[]);
        peak = peak.max(s.particles().len());
    }
    assert!(peak > 10 && peak < 90, "100 格水在 chance≈0.5 下应部分脱格，峰值实际 {peak}");
}

/// 线程数不变性（验收 §0 第 3 项）：溅射发生在**并行**四相 pass 里，
/// 生成序必须只由 (相位序, chunk index, chunk 内扫描序) 决定，与线程数无关。
#[test]
fn splash_spawn_order_is_thread_count_invariant() {
    let run = |threads: usize| {
        let mut s =
            sim_with_table(4, 3, 46, threads, ScanMode::LiveRect, test_table_with_splash(128, 60));
        s.apply_setup(&[
            Op::Fill { material: MAT_WALL, x0: 0, y0: 188, x1: 255, y1: 188 },
            Op::Fill { material: WATER, x0: 10, y0: 20, x1: 120, y1: 40 },
            Op::Fill { material: SAND, x0: 130, y0: 20, x1: 240, y1: 40 },
        ]);
        let mut hashes = Vec::new();
        let mut peak = 0usize;
        for _ in 0..200 {
            s.step(&[]);
            hashes.push(s.state_hash());
            peak = peak.max(s.particles().len());
        }
        (hashes, peak)
    };
    let (h1, n1) = run(1);
    let (h8, n8) = run(8);
    let (h16, n16) = run(16);
    assert!(n1 > 0, "场景必须真的产出溅射粒子（峰值），否则这条测试是空转");
    assert_eq!(h1, h8, "1 线程与 8 线程的逐 tick 状态哈希必须逐位相同");
    assert_eq!(h1, h16, "1 线程与 16 线程的逐 tick 状态哈希必须逐位相同");
    assert_eq!((n1, n8), (n1, n16));
}

/// **P→G 撞击动量传递**（用户裁决 2026-08-31，并入 Task 3）：粒子落格前
/// 动量整个被丢弃 —— 网格 cell 跑到 4 格/tick 会溅射，粒子跑到 16 格/tick
/// 反而不溅。补法是把撞击速度量化写进 cell 速度位，复用已有的溅射路径。
///
/// 这条测试锁两段：① 落格 cell 真的带上了速度；② 下一 tick 它经既有溅射
/// 判定重新脱格（粒子回来了）。只测 ① 的话，一个写了速度却永远触发不了
/// 溅射的实现照样全绿。
#[test]
fn landing_particle_carries_impact_momentum_into_the_grid() {
    let mut s = sim_with_table(2, 2, 47, 1, ScanMode::LiveRect, test_table_with_splash(255, 0));
    s.apply_setup(&[
        Op::Fill { material: MAT_WALL, x0: 0, y0: 124, x1: 127, y1: 124 },
        Op::Emit {
            material: WATER,
            x: Fx::from_int(64),
            y: Fx::from_int(4),
            vx: Fx::ZERO,
            vy: Fx::from_int(8),
            count: 1,
            jitter: Fx::ZERO,
        },
    ]);
    // 跑到粒子落格：网格里出现水
    let mut landed = None;
    for _ in 0..60 {
        s.step(&[]);
        if let Some(y) = (0..124).find(|&y| s.world().cell(64, y).material() == WATER) {
            landed = Some(y);
            break;
        }
    }
    let y = landed.expect("粒子应在 60 tick 内落格");
    assert_eq!(
        s.world().cell(64, y).vel(),
        V_MAX_CELL,
        "落格 cell 必须带上撞击速度（自由落体 120 格早已封顶）"
    );
    s.step(&[]);
    assert_eq!(s.particles().len(), 1, "满速落格 cell 应在下一 tick 经溅射判定重新脱格");
    assert_eq!(s.world().count_material(WATER), 0, "脱格后网格里不该再有水");
}
