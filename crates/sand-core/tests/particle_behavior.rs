//! 粒子层行为测试 + 并行确定性（Task 4，修复轮 1 补测 I1/I2）：
//! - 自由落体到平地，守恒验收（各占一列 + 同位同速冲突两个变体）；
//! - {1,4} 线程 × {Full, ChunkSleep, LiveRect} 六配置逐 tick 全哈希比对——
//!   现有 CI SyncTest 场景无粒子，这条是粒子相并行正确性的唯一执法，口径
//!   对齐 spec §0（六配置矩阵）。

mod common;

use common::{sim, SAND, WATER};
use sand_core::{Fx, Op, ScanMode, Sim, MAT_WALL};

fn floor_op(w: i32, h: i32) -> Op {
    Op::Fill { material: MAT_WALL, x0: 0, y0: h - 4, x1: w - 1, y1: h - 4 }
}

/// N 个粒子自由落体到平地：跑足够 tick 后全部落格，网格该材质计数 == N（守恒）。
/// 各自独占一列、零横移，守恒断言不掺杂冲突消解的干扰（冲突路径见下一测试
/// 及 particle.rs 的 commit 单测）。
#[test]
fn particles_free_fall_and_land_conserving_count() {
    let mut s = sim(2, 2, 10, 1, ScanMode::LiveRect);
    s.apply_setup(&[floor_op(128, 128)]);

    let n: usize = 40;
    for i in 0..n {
        let x = Fx::from_int(10 + i as i32);
        s.queue_spawn(SAND, x, Fx::from_int(5), Fx::ZERO, Fx::ZERO);
    }
    s.step(&[]); // 本 tick 完成生成 + 首次积分

    // 下落 ~118 格，重力 0.25/tick²，约 31 tick 落地；200 tick 留足余量。
    for _ in 0..200 {
        s.step(&[]);
    }

    assert_eq!(s.particles().len(), 0, "所有粒子应已落格，池中不应再有自由粒子");
    assert_eq!(s.world().count_material(SAND), n, "落格后沙的格数必须等于生成的粒子数（守恒）");
}

/// I2（修复轮 1）：评审复现几何的端到端版本——一大批粒子**同位同速**落到
/// 平地，逐 tick 大量竞争同一候选格与邻格。C1 修复前，这个场景会有相当一部分
/// 粒子被"全占转悬浮"困死、池永不排空；修复后必须全部经落格或兜底向上搜索
/// 排空，网格计数守恒。
#[test]
fn particles_same_position_and_velocity_conflict_still_conserves_and_drains_pool() {
    let mut s = sim(2, 2, 11, 1, ScanMode::LiveRect);
    s.apply_setup(&[floor_op(128, 128)]);

    let n: usize = 40;
    for _ in 0..n {
        // 同一位置、同一速度：DDA 结果逐帧完全一致，几乎必然争抢同一候选格。
        s.queue_spawn(SAND, Fx::from_int(64), Fx::from_int(5), Fx::ZERO, Fx::ZERO);
    }
    s.step(&[]);

    // 冲突消解会把落点摊到候选格 + 邻格 + 向上兜底一整根竖列，比自由落体
    // 单粒子慢不了太多；300 tick 留足余量（含世界顶溢出保护路径的可能性）。
    for _ in 0..300 {
        s.step(&[]);
    }

    assert_eq!(s.particles().len(), 0, "C1 修复后：全部粒子必须落格或经兜底排空，池不得残留");
    assert_eq!(s.world().count_material(SAND), n, "网格新增沙格数必须等于发射粒子数（守恒，零悬浮/零重复）");
}

/// 并行确定性执法（唯一覆盖粒子相的 SyncTest）：{1,4} 线程 × {Full,
/// ChunkSleep, LiveRect} 六配置逐 tick 全哈希一致（spec §0 六配置口径）。
/// 场景在同一窄区间反复密集喷发带初速抖动的一小簇粒子，制造同 tick 多粒子
/// 落同格/邻格竞争，覆盖 commit 冲突消解（含 C1 修复的向上兜底搜索）在并行
/// 调度下的确定性。
#[test]
fn particle_phase_is_thread_count_and_scan_mode_invariant() {
    let configs = [
        (1usize, ScanMode::Full),
        (4, ScanMode::Full),
        (1, ScanMode::ChunkSleep),
        (4, ScanMode::ChunkSleep),
        (1, ScanMode::LiveRect),
        (4, ScanMode::LiveRect),
    ];
    let mut sims: Vec<_> = configs.iter().map(|&(t, sk)| sim(3, 2, 0xBEEF, t, sk)).collect();
    let setup = [floor_op(192, 128)];
    for s in &mut sims {
        s.apply_setup(&setup);
    }

    for tick in 0..600u64 {
        for s in &mut sims {
            spawn_batch(s, tick);
        }
        for s in &mut sims {
            s.step(&[]);
        }
        let h0 = sims[0].state_hash();
        for (i, s) in sims.iter().enumerate().skip(1) {
            assert_eq!(
                s.state_hash(),
                h0,
                "tick {tick}: 配置 {:?} 与 {:?} 粒子相分叉",
                configs[i],
                configs[0]
            );
        }
    }
}

/// 每隔 4 tick 在窄区间喷发一簇 6 个粒子，横向初速做确定性"抖动"（tick/i 的
/// 纯函数，非随机），使它们大概率在下落途中彼此靠近、同 tick 争抢同一格或
/// 邻格——全部六个 Sim（不同线程数/扫描模式）都调用同一份 `spawn_batch`，
/// 输入序列逐 tick 完全一致，唯一变量是并行调度与扫描策略。
fn spawn_batch(s: &mut Sim, tick: u64) {
    if tick < 400 && tick % 4 == 0 {
        for i in 0..6i32 {
            let jitter = Fx::from_ratio(i - 3, 4); // -0.75..0.5 格/tick，逐粒子不同
            s.queue_spawn(
                WATER,
                Fx::from_int(90 + (i % 3)),
                Fx::from_int(5),
                jitter,
                Fx::from_int(2),
            );
        }
    }
}
