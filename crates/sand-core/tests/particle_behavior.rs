//! 粒子层行为测试 + 并行确定性（Task 4）：
//! - 自由落体到平地，守恒验收；
//! - {1, 4} 线程逐 tick 全哈希比对——现有 CI SyncTest 场景无粒子，这条是
//!   粒子相并行正确性的唯一执法（brief 明文，必须有）。

mod common;

use common::{sim, SAND, WATER};
use sand_core::{Fx, Op, ScanMode, Sim, MAT_WALL};

fn floor_op(w: i32, h: i32) -> Op {
    Op::Fill { material: MAT_WALL, x0: 0, y0: h - 4, x1: w - 1, y1: h - 4 }
}

/// N 个粒子自由落体到平地：跑足够 tick 后全部落格，网格该材质计数 == N（守恒）。
#[test]
fn particles_free_fall_and_land_conserving_count() {
    let mut s = sim(2, 2, 10, 1, ScanMode::LiveRect);
    s.apply_setup(&[floor_op(128, 128)]);

    let n: usize = 40;
    for i in 0..n {
        // 分散在地板上方不同 x（各自独占一列，纯自由落体、零横移），
        // 让守恒断言不掺杂冲突消解的干扰（冲突路径由专门单测覆盖）。
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

/// 并行确定性执法（唯一覆盖粒子相的 SyncTest）：{1,4} 线程逐 tick 全哈希一致。
/// 场景在同一窄区间反复密集喷发带初速抖动的一小簇粒子，制造同 tick 多粒子
/// 落同格/邻格竞争，覆盖 commit 冲突消解在并行调度下的确定性。
#[test]
fn particle_phase_is_thread_count_invariant() {
    let mut s1 = sim(3, 2, 0xBEEF, 1, ScanMode::LiveRect);
    let mut s4 = sim(3, 2, 0xBEEF, 4, ScanMode::LiveRect);
    let setup = [floor_op(192, 128)];
    s1.apply_setup(&setup);
    s4.apply_setup(&setup);

    for tick in 0..600u64 {
        spawn_batch(&mut s1, tick);
        spawn_batch(&mut s4, tick);
        s1.step(&[]);
        s4.step(&[]);
        assert_eq!(s1.state_hash(), s4.state_hash(), "tick {tick}: 1 线程与 4 线程粒子相分叉");
    }
}

/// 每隔 4 tick 在窄区间喷发一簇 6 个粒子，横向初速做确定性"抖动"（tick/i 的
/// 纯函数，非随机），使它们大概率在下落途中彼此靠近、同 tick 争抢同一格或
/// 邻格——两个 Sim（不同线程数）都调用同一份 `spawn_batch`，输入序列逐 tick
/// 完全一致，唯一变量是并行调度。
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
