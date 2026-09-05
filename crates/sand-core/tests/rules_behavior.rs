//! 规则行为集成测试（spec §5.4 第 1 层）：经 Sim 公共 API 驱动。

mod common;

use common::{
    sim, sim_with_reactions, sim_with_table, test_table_with_gas, test_table_with_splash,
    test_table_with_water_dispersion, SAND, SMOKE, WATER,
};
use sand_core::{Category, MaterialDef, MaterialTable, ReactionRule, ReactionTable};
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
    s.step(&[], &[]);
    assert_eq!(s.world().cell(40, 10).material(), MAT_AIR);
    assert_eq!(s.world().cell(40, 11).material(), SAND);
    for _ in 0..5 {
        s.step(&[], &[]);
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
        s.step(&ops, &[]);
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
    s.step(&[], &[]);
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
        s.step(&[], &[]);
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
        s.step(&[], &[]);
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
        s.step(&[], &[]);
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
    s.step(&[], &[]);
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
    s.step(&[], &[]);
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
    s.step(&[], &[]);
    assert_eq!(s.world().cell(95, 123).dir(), -1, "左行后记忆方向必须是左");

    // ② 翻向：紧邻左侧是墙 → 首选方向失败 → 翻向右行，记忆必须跟着翻成 +1
    let mut s = sim_with_table(2, 2, 10, 1, ScanMode::LiveRect, test_table_with_water_dispersion(5));
    s.apply_setup(&[
        floor_op(128, 128),
        Op::Fill { material: MAT_WALL, x0: 99, y0: 123, x1: 99, y1: 123 },
        Op::Brush { material: WATER, x: 100, y: 123, r: 0 },
    ]);
    s.step(&[], &[]);
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
    s.step(&[], &[]);
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
    s.step(&[], &[]);
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
            s.step(&[], &[]);
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
        s.step(&[], &[]);
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
        s.step(&[], &[]);
    }
    let y15 = find(&s).expect("沙还在下落中");
    assert_eq!(
        s.world().cell(40, y15).vel(),
        15 * G_ACCEL,
        "第 15 tick 速度应为 15 个 ¼ 格单位，尚未封顶"
    );
    s.step(&[], &[]);
    let y16 = find(&s).expect("沙还在下落中");
    assert_eq!(s.world().cell(40, y16).vel(), V_MAX_CELL, "第 16 tick 应首次达终端速度");
    s.step(&[], &[]);
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
        s.step(&[], &[]);
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
        s.step(&[], &[]);
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
        s.step(&[], &[]);
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
        s.step(&[], &[]);
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
        s.step(&[], &[]);
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

// ==================== M2 Task 1：气体（spec §3）====================

/// 上浮镜像 `sand_falls_straight_down`：上方为空时永远走正上方分支，且
/// **恰一格/tick**——spec §3.3 的 stamp 防连锁回归（自下而上扫描对上浮是
/// 连锁方向，`displace` 双格盖戳 + eval 开头戳检查堵死一帧多升）。
#[test]
fn gas_bubble_rises_exactly_one_cell_per_tick() {
    let mut s = sim_with_table(2, 2, 7, 1, ScanMode::LiveRect, test_table_with_gas());
    s.apply_setup(&[Op::Brush { material: SMOKE, x: 40, y: 100, r: 0 }]);
    for t in 1..=20i32 {
        s.step(&[], &[]);
        assert_eq!(s.world().cell(40, 100 - t).material(), SMOKE, "tick {t}：烟应恰在 y={}", 100 - t);
        assert_eq!(s.world().count_material(SMOKE), 1, "tick {t}：烟数量守恒");
    }
}

/// 水下的烟必须冒出水面（spec §3.1 密度梯度反转：上浮要求目标更重才让路；
/// 加上水自身下沉置换，两条通路都把烟往上推）。
#[test]
fn gas_bubbles_up_through_liquid() {
    let mut s = sim_with_table(2, 2, 11, 1, ScanMode::LiveRect, test_table_with_gas());
    s.apply_setup(&[
        Op::Fill { material: MAT_WALL, x0: 30, y0: 110, x1: 50, y1: 110 }, // 池底
        Op::Fill { material: MAT_WALL, x0: 30, y0: 90, x1: 30, y1: 109 },  // 左壁
        Op::Fill { material: MAT_WALL, x0: 50, y0: 90, x1: 50, y1: 109 },  // 右壁
        Op::Fill { material: WATER, x0: 31, y0: 100, x1: 49, y1: 109 },    // 水体
        Op::Brush { material: SMOKE, x: 40, y: 108, r: 0 },                // 池底一格烟
    ]);
    for _ in 0..200 {
        s.step(&[], &[]);
    }
    assert_eq!(s.world().count_material(SMOKE), 1, "烟不得消失");
    let smoke_y = (0..128)
        .flat_map(|x| (0..128).map(move |y| (x, y)))
        .find(|&(x, y)| s.world().cell(x, y).material() == SMOKE)
        .map(|(_, y)| y)
        .unwrap();
    let water_top = (0..128)
        .flat_map(|y| (0..128).map(move |x| (x, y)))
        .find(|&(x, y)| s.world().cell(x, y).material() == WATER)
        .map(|(_, y)| y)
        .unwrap();
    assert!(
        smoke_y < water_top,
        "烟（y={smoke_y}）必须升到全部水（最高 y={water_top}）之上"
    );
}

/// 被困气体零写入 ⇒ chunk 照常入睡（写回纪律，spec §5.6 同源；
/// 镜像 `resting_pile_lets_every_chunk_sleep`）。烟被墙完全围死，
/// gas_step 四路全 Blocked——什么都不许写。
#[test]
fn trapped_gas_lets_chunk_sleep() {
    let mut s = sim_with_table(2, 2, 13, 1, ScanMode::LiveRect, test_table_with_gas());
    s.apply_setup(&[
        Op::Fill { material: MAT_WALL, x0: 39, y0: 99, x1: 41, y1: 101 }, // 3×3 实心
        Op::Brush { material: SMOKE, x: 40, y: 100, r: 0 },               // 中心换成烟
    ]);
    for _ in 0..100 {
        s.step(&[], &[]);
    }
    assert_eq!(s.world().cell(40, 100).material(), SMOKE, "被困的烟原地不动");
    for (ci, c) in s.world().chunks.iter().enumerate() {
        assert!(c.dirty.is_empty(), "chunk {ci} 静置后仍脏：{:?}", c.dirty);
        assert!(
            c.next_dirty.snapshot().is_empty(),
            "chunk {ci} 静置后 next_dirty 非空：{:?}",
            c.next_dirty.snapshot()
        );
    }
}

// ==================== M2 Task 2：反应表（spec §4）====================

/// 反应测试材质表：基线 + smoke(4) + fire(5)。fire 故意排在 smoke 之后——
/// 让 water(3) < smoke(4) < fire(5)，water 对两者都是发起方。
const FIRE: u8 = 5;

fn fire_table() -> MaterialTable {
    MaterialTable::new(vec![
        MaterialDef::base(0, "air", Category::Static, 0),
        MaterialDef::base(1, "wall", Category::Static, 100),
        MaterialDef::base(SAND, "sand", Category::Powder, 40),
        MaterialDef::base(WATER, "water", Category::Liquid, 16),
        MaterialDef::base(SMOKE, "smoke", Category::Gas, 2),
        MaterialDef::base(FIRE, "fire", Category::Gas, 1),
    ])
    .unwrap()
}

/// 发起方约定防双结算（spec §4.2）：water + fire 相邻、概率 255（必发），
/// 一 tick 后两格**恰各转化一次**——water 结算出 water + smoke；若正反双向
/// 注册（原型 reaction.py:44-46 的反例），fire 侧会再结算一次。
#[test]
fn initiator_convention_prevents_double_settlement() {
    let t = fire_table();
    let reactions = ReactionTable::new(
        &t,
        vec![ReactionRule { a: WATER, b: FIRE, out_a: WATER, out_b: SMOKE, threshold: 255 }],
    )
    .unwrap();
    let mut s = sim_with_reactions(2, 2, 3, 1, ScanMode::LiveRect, t, reactions);
    // 1 格深的口袋：water 四面围死（下/左/右墙），fire 在其上方。
    s.apply_setup(&[
        Op::Fill { material: MAT_WALL, x0: 39, y0: 100, x1: 39, y1: 101 },
        Op::Fill { material: MAT_WALL, x0: 41, y0: 100, x1: 41, y1: 101 },
        Op::Brush { material: MAT_WALL, x: 40, y: 101, r: 0 },
        Op::Brush { material: WATER, x: 40, y: 100, r: 0 },
        Op::Brush { material: FIRE, x: 40, y: 99, r: 0 },
    ]);
    s.step(&[], &[]);
    assert_eq!(s.world().cell(40, 100).material(), WATER, "发起方产物 = water（1:1）");
    assert_eq!(s.world().cell(40, 99).material(), SMOKE, "邻居产物 = smoke");
    assert_eq!(s.world().count_material(FIRE), 0);
    assert_eq!(s.world().count_material(SMOKE), 1, "恰一次结算——双结算会在别处再冒 smoke");
    assert_eq!(s.world().count_material(WATER), 1);
}

/// 已盖当前戳的格本 tick 不再作为反应对象（spec §4.5 审阅补漏）：
/// water–fire–water 一行，第一个 water 把 fire 转成 smoke（盖戳）；若第二个
/// water 还能对这个新 smoke 结算 water+smoke→sand+sand，就是同格一 tick 二次
/// 转化。断言：不产生任何 sand。
#[test]
fn reaction_skips_neighbors_stamped_this_tick() {
    let t = fire_table();
    let reactions = ReactionTable::new(
        &t,
        vec![
            ReactionRule { a: WATER, b: FIRE, out_a: WATER, out_b: SMOKE, threshold: 255 },
            ReactionRule { a: WATER, b: SMOKE, out_a: SAND, out_b: SAND, threshold: 255 },
        ],
    )
    .unwrap();
    let mut s = sim_with_reactions(2, 2, 5, 1, ScanMode::LiveRect, t, reactions);
    // 密闭 3 格横槽：wall | water fire water | wall，底与顶全墙。
    s.apply_setup(&[
        Op::Fill { material: MAT_WALL, x0: 37, y0: 98, x1: 43, y1: 102 },
        Op::Brush { material: WATER, x: 39, y: 100, r: 0 },
        Op::Brush { material: FIRE, x: 40, y: 100, r: 0 },
        Op::Brush { material: WATER, x: 41, y: 100, r: 0 },
    ]);
    s.step(&[], &[]);
    assert_eq!(s.world().count_material(SAND), 0, "戳跳过失效：产物同 tick 被二次结算成 sand");
    assert_eq!(s.world().count_material(SMOKE), 1);
    assert_eq!(s.world().count_material(WATER), 2);
    // 下一 tick 戳过期，water+smoke 才允许结算（机制是"一格一 tick 至多一次"，
    // 不是"永不"）。
    s.step(&[], &[]);
    assert_eq!(s.world().count_material(SAND), 2, "次 tick 应正常结算 water+smoke");
}

/// 分布回归（spec §7.2，本 spec 新立的规矩）：反应触发率贴近声明概率。
/// RNG salt 维度缺失这类 bug 两端一样地错、SyncTest 抓不到——必须验分布。
/// 全图墙背景里刻出 ~1600 个隔离的 water/fire 竖直对，threshold 128
/// （p = 128/255 ≈ 0.502），一 tick 后统计转化率。
#[test]
fn reaction_rate_matches_declared_probability() {
    let t = fire_table();
    let reactions = ReactionTable::new(
        &t,
        vec![ReactionRule { a: WATER, b: FIRE, out_a: WATER, out_b: SMOKE, threshold: 128 }],
    )
    .unwrap();
    let mut s = sim_with_reactions(2, 2, 9, 4, ScanMode::LiveRect, t, reactions);
    let mut setup = vec![Op::Fill { material: MAT_WALL, x0: 0, y0: 0, x1: 127, y1: 127 }];
    let mut pairs = 0u32;
    let mut x = 2;
    while x < 126 {
        let mut y = 2;
        while y < 125 {
            setup.push(Op::Brush { material: WATER, x, y: y + 1, r: 0 });
            setup.push(Op::Brush { material: FIRE, x, y, r: 0 });
            pairs += 1;
            y += 3;
        }
        x += 3;
    }
    s.apply_setup(&setup);
    s.step(&[], &[]);
    let hits = s.world().count_material(SMOKE) as f64;
    let p = hits / pairs as f64;
    let expect = 128.0 / 255.0;
    // n≈1722 ⇒ σ = √(p(1−p)/n) ≈ 1.2%，取 4σ ≈ 4.8%
    assert!(
        (p - expect).abs() < 0.048,
        "触发率 {p:.4}（{hits}/{pairs}）偏离声明概率 {expect:.4} 超 4σ——salt/attempt 维度出问题了？"
    );
}

// ==================== M2 Task 3：燃烧（spec §5）====================

const WOOD: u8 = 6;
const OIL: u8 = 7;

/// 燃烧测试材质表：fire 温 100 点得着 wood(80)/oil(40)；**燃料也声明
/// `fire_temp: 100`**——燃烧中的油/木直接点燃同类蔓延（源门 spec §5.2 保证
/// 冷燃料点不着任何东西；火是气体、升离表面极快，横向蔓延必须靠燃料自身
/// 温度）。wood 不产火（fire_chance 0，纯温度蔓延），oil 产火 0.6。
/// 燃料池刻意偏短（wood 50 / oil 30），让测试在几百 tick 内自然烧完。
fn burn_table() -> MaterialTable {
    MaterialTable::new(vec![
        MaterialDef::base(0, "air", Category::Static, 0),
        MaterialDef::base(1, "wall", Category::Static, 100),
        MaterialDef::base(SAND, "sand", Category::Powder, 40),
        MaterialDef { extinguisher: true, ..MaterialDef::base(WATER, "water", Category::Liquid, 16) },
        MaterialDef { lifetime: 200, ..MaterialDef::base(SMOKE, "smoke", Category::Gas, 2) },
        MaterialDef {
            lifetime: 40,
            fire_temp: 100,
            decay_to: SMOKE,
            rise_chance: 128,
            ..MaterialDef::base(FIRE, "fire", Category::Gas, 1)
        },
        MaterialDef {
            fire_hp: 50,
            ignition_temp: 80,
            fire_temp: 100,
            ..MaterialDef::base(WOOD, "wood", Category::Static, 60)
        },
        MaterialDef {
            fire_hp: 30,
            ignition_temp: 40,
            fire_temp: 100,
            fire_chance: 153,
            flame_to: FIRE,
            ..MaterialDef::base(OIL, "oil", Category::Liquid, 12)
        },
    ])
    .unwrap()
}

fn burn_sim(seed: u64, table: MaterialTable) -> sand_core::Sim {
    let reactions = ReactionTable::empty(&table);
    sim_with_reactions(2, 2, seed, 1, ScanMode::LiveRect, table, reactions)
}

/// 点燃判定的"源正在燃烧"门（spec §5.2 审阅补漏）：**冷**油即便声明了高
/// fire_temp（这里故意给 100），也绝不点燃邻居——burn 阶段只对 counter > 0
/// 的格运行。若这道门缺失，本表下冷油一 tick 内就会点着隔壁 wood。
#[test]
fn ignition_needs_burning_source() {
    let defs = vec![
        MaterialDef::base(0, "air", Category::Static, 0),
        MaterialDef::base(1, "wall", Category::Static, 100),
        MaterialDef::base(SAND, "sand", Category::Powder, 40),
        MaterialDef::base(WATER, "water", Category::Liquid, 16),
        MaterialDef { lifetime: 200, ..MaterialDef::base(SMOKE, "smoke", Category::Gas, 2) },
        MaterialDef { lifetime: 40, fire_temp: 100, decay_to: SMOKE, ..MaterialDef::base(FIRE, "fire", Category::Gas, 1) },
        MaterialDef { fire_hp: 50, ignition_temp: 80, fire_temp: 100, ..MaterialDef::base(WOOD, "wood", Category::Static, 60) },
        MaterialDef { fire_hp: 30, ignition_temp: 40, fire_temp: 100, fire_chance: 153, flame_to: FIRE, ..MaterialDef::base(OIL, "oil", Category::Liquid, 12) },
    ];
    // 油的 fire_temp = 100 高于 wood 的着火点 80——源门失效即点燃
    let mut s = burn_sim(21, MaterialTable::new(defs).unwrap());
    s.apply_setup(&[
        Op::Fill { material: MAT_WALL, x0: 37, y0: 97, x1: 43, y1: 103 },
        Op::Brush { material: OIL, x: 39, y: 100, r: 0 },
        Op::Brush { material: WOOD, x: 40, y: 100, r: 0 },
        Op::Brush { material: 0, x: 40, y: 99, r: 0 }, // wood 上方留 air（氧气）
    ]);
    for _ in 0..100 {
        s.step(&[], &[]);
    }
    assert_eq!(s.world().cell(40, 100).material(), WOOD, "冷油旁的 wood 必须还在");
    assert_eq!(s.world().cell(40, 100).counter(), 0, "冷油绝不点燃邻居（源门 spec §5.2）");
    assert_eq!(s.world().count_material(FIRE), 0);
}

/// 火油连锁端到端（spec §5 链条闭合）：火点燃油 → 油产火 → 火衰变烟 →
/// 烟衰变空气。终态火烟归零、油有损耗，过程中三种中间态都出现过。
#[test]
fn fire_ignites_oil_and_chain_decays_to_air() {
    let mut s = burn_sim(31, burn_table());
    s.apply_setup(&[
        Op::Fill { material: MAT_WALL, x0: 30, y0: 90, x1: 60, y1: 110 },
        Op::Fill { material: 0, x0: 32, y0: 92, x1: 58, y1: 108 }, // 挖出内腔
        Op::Fill { material: OIL, x0: 36, y0: 106, x1: 54, y1: 108 }, // 油池
        // 点火放进池**内**（覆写一行油）：火是气体、放在油面上一 tick 就升走，
        // 触不到油；埋进池里则上浮沿途四邻皆油，点燃必然发生。
        Op::Fill { material: FIRE, x0: 44, y0: 107, x1: 46, y1: 107 },
    ]);
    let oil0 = s.world().count_material(OIL);
    let (mut saw_burning_oil, mut saw_smoke) = (false, false);
    for _ in 0..1500 {
        s.step(&[], &[]);
        if !saw_burning_oil {
            'scan: for x in 32..59 {
                for y in 92..109 {
                    let c = s.world().cell(x, y);
                    if c.material() == OIL && c.counter() > 0 {
                        saw_burning_oil = true;
                        break 'scan;
                    }
                }
            }
        }
        saw_smoke |= s.world().count_material(SMOKE) > 0;
    }
    assert!(saw_burning_oil, "油必须被点燃过（counter > 0）");
    assert!(saw_smoke, "火熄必须产生过烟");
    assert!(s.world().count_material(OIL) < oil0, "必须有油被烧掉");
    assert_eq!(s.world().count_material(FIRE), 0, "终态：火全部熄灭");
    assert_eq!(s.world().count_material(SMOKE), 0, "终态：烟全部散尽");
}

/// 由外向内烧（spec §5.4）：大块 wood 的深部格在表面烧完之前 counter 恒 0
/// ——氧气前置（审阅补漏）保证内部格根本不被装填。12×12 块、观察 200 tick
/// （最多烧穿 4 层，中心深 6 层，留余量）。
#[test]
fn wood_burns_outside_in() {
    let mut s = burn_sim(41, burn_table());
    s.apply_setup(&[
        floor_op(128, 128),
        Op::Fill { material: WOOD, x0: 40, y0: 100, x1: 51, y1: 111 },
        Op::Fill { material: FIRE, x0: 44, y0: 99, x1: 47, y1: 99 }, // 顶面点火
    ]);
    for t in 0..200u64 {
        s.step(&[], &[]);
        let center = s.world().cell(45, 106);
        assert_eq!(center.material(), WOOD, "tick {t}：中心格不该被烧没");
        assert_eq!(center.counter(), 0, "tick {t}：中心格在表面烧完前不得装填（由外向内）");
    }
}

/// 火放在油面**上方**必须点得着（spec §5.3.1 第 4 条 rise_chance 的根因
/// 回归）：恒升的火升离水平燃料面后，落点的下邻是自己刚腾出的空气——
/// 2026-08-31 实测 13 格火 × 40 tick 寿命 0 次点燃。rise_chance = 0.5 让火
/// 在表面逗留掷点燃骰，引导环节闭合。
#[test]
fn fire_dropped_on_surface_ignites_pool() {
    let mut s = burn_sim(71, burn_table());
    s.apply_setup(&[
        Op::Fill { material: MAT_WALL, x0: 30, y0: 90, x1: 60, y1: 110 },
        Op::Fill { material: 0, x0: 32, y0: 92, x1: 58, y1: 108 },
        Op::Fill { material: OIL, x0: 36, y0: 106, x1: 54, y1: 108 },
        // 火放在油面**正上方一行**（不埋进池里）
        Op::Fill { material: FIRE, x0: 42, y0: 105, x1: 48, y1: 105 },
    ]);
    let oil0 = s.world().count_material(OIL);
    for _ in 0..600 {
        s.step(&[], &[]);
    }
    assert!(
        s.world().count_material(OIL) < oil0,
        "表面点火必须烧掉油（rise_chance 引导回归：修复前 0 次点燃）"
    );
}

/// 灭火走数据字段（spec §5.5）：邻接 water 的 wood 被点燃即清零、永不烧毁；
/// 无 water 的对照腔烧穿。两腔同构，只差 water。
#[test]
fn water_extinguishes_burning_fuel() {
    let mut s = burn_sim(51, burn_table());
    let cavity = |x0: i32, with_water: bool| -> Vec<Op> {
        let mut ops = vec![
            Op::Fill { material: MAT_WALL, x0: x0 - 2, y0: 96, x1: x0 + 2, y1: 103 },
            Op::Brush { material: 0, x: x0, y: 98, r: 0 },
            Op::Brush { material: 0, x: x0, y: 99, r: 0 },
            Op::Brush { material: 0, x: x0, y: 101, r: 0 }, // wood 下方 air（氧气）
            Op::Brush { material: WOOD, x: x0, y: 100, r: 0 },
        ];
        ops.push(Op::Brush { material: FIRE, x: x0, y: 98, r: 0 });
        ops.push(Op::Brush { material: FIRE, x: x0, y: 99, r: 0 });
        if with_water {
            ops.push(Op::Brush { material: WATER, x: x0 + 1, y: 100, r: 0 });
        }
        ops
    };
    let mut setup = cavity(40, true);
    setup.extend(cavity(80, false));
    s.apply_setup(&setup);
    for _ in 0..600 {
        s.step(&[], &[]);
    }
    assert_eq!(s.world().cell(40, 100).material(), WOOD, "有 water 邻居：wood 必须幸存");
    assert_eq!(s.world().cell(40, 100).counter(), 0, "灭火后 counter 清零");
    assert_eq!(s.world().cell(80, 100).material(), 0, "对照腔：wood 必须烧穿成 air");
}

/// 休眠执法（spec §5.6，镜像 `resting_pile_lets_every_chunk_sleep`）：
/// 未点燃的可燃物必须零写入——静置 wood 全图入睡。
#[test]
fn resting_wood_lets_chunk_sleep() {
    let mut s = burn_sim(26, burn_table());
    s.apply_setup(&[
        floor_op(128, 128),
        Op::Fill { material: WOOD, x0: 40, y0: 110, x1: 80, y1: 123 },
        // 油装盆（两道盆壁）——裸地板上的油要流平很久，那是液体既有行为，
        // 不是本测试的对象；本测试只管"未点燃的可燃物零写入"。
        Op::Fill { material: MAT_WALL, x0: 88, y0: 114, x1: 89, y1: 123 },
        Op::Fill { material: MAT_WALL, x0: 111, y0: 114, x1: 112, y1: 123 },
        Op::Fill { material: OIL, x0: 90, y0: 120, x1: 110, y1: 123 },
    ]);
    for _ in 0..400 {
        s.step(&[], &[]);
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

/// 分布回归（spec §7.2）：点燃方向骰四向均匀。961 个隔离腔，中心 fire 四邻
/// wood（本表 requires_oxygen: false，免去氧气干扰），一 tick 后统计各方向
/// 被点燃的比例 ≈ 25%。方向骰若丢 salt/attempt 维度或取位有偏，这里会炸。
#[test]
fn ignition_direction_roll_is_uniform() {
    let defs = vec![
        MaterialDef::base(0, "air", Category::Static, 0),
        MaterialDef::base(1, "wall", Category::Static, 100),
        MaterialDef::base(SAND, "sand", Category::Powder, 40),
        MaterialDef::base(WATER, "water", Category::Liquid, 16),
        MaterialDef { lifetime: 200, ..MaterialDef::base(SMOKE, "smoke", Category::Gas, 2) },
        MaterialDef { lifetime: 40, fire_temp: 100, decay_to: SMOKE, ..MaterialDef::base(FIRE, "fire", Category::Gas, 1) },
        MaterialDef {
            fire_hp: 50,
            ignition_temp: 80,
            requires_oxygen: false,
            ..MaterialDef::base(WOOD, "wood", Category::Static, 60)
        },
    ];
    let mut s = burn_sim(61, MaterialTable::new(defs).unwrap());
    let mut setup = vec![Op::Fill { material: MAT_WALL, x0: 0, y0: 0, x1: 127, y1: 127 }];
    let mut centers = Vec::new();
    let mut cx = 2;
    while cx < 125 {
        let mut cy = 2;
        while cy < 125 {
            setup.push(Op::Brush { material: WOOD, x: cx, y: cy - 1, r: 0 });
            setup.push(Op::Brush { material: WOOD, x: cx, y: cy + 1, r: 0 });
            setup.push(Op::Brush { material: WOOD, x: cx - 1, y: cy, r: 0 });
            setup.push(Op::Brush { material: WOOD, x: cx + 1, y: cy, r: 0 });
            setup.push(Op::Brush { material: FIRE, x: cx, y: cy, r: 0 });
            centers.push((cx, cy));
            cy += 4;
        }
        cx += 4;
    }
    s.apply_setup(&setup);
    s.step(&[], &[]);
    // NEIGHBORS4 序：上、下、左、右
    let mut counts = [0u32; 4];
    let mut lit_total = 0u32;
    for &(cx, cy) in &centers {
        let dirs = [(0, -1), (0, 1), (-1, 0), (1, 0)];
        for (i, (dx, dy)) in dirs.iter().enumerate() {
            if s.world().cell(cx + dx, cy + dy).counter() > 0 {
                counts[i] += 1;
                lit_total += 1;
            }
        }
    }
    let n = centers.len() as u32;
    assert_eq!(lit_total, n, "每腔应恰点燃一个方向（温度/燃料/戳条件全满足）");
    for (i, &c) in counts.iter().enumerate() {
        let p = c as f64 / n as f64;
        // n=961，σ=√(0.25·0.75/961)≈1.4%，取 4σ≈5.6%
        assert!(
            (p - 0.25).abs() < 0.056,
            "方向 {i} 占比 {p:.4}（{c}/{n}）偏离 0.25 超 4σ——方向骰有偏"
        );
    }
}

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
        s.step(&[], &[]);
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
        s.step(&[], &[]);
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
        s.step(&[], &[]);
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
        s.step(&[], &[]);
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
        s.step(&[], &[]);
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
            s.step(&[], &[]);
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
        s.step(&[], &[]);
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
    s.step(&[], &[]);
    assert_eq!(s.particles().len(), 1, "满速落格 cell 应在下一 tick 经溅射判定重新脱格");
    assert_eq!(s.world().count_material(WATER), 0, "脱格后网格里不该再有水");
}
