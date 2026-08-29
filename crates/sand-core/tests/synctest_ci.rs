//! CI 级 SyncTest（spec §5.4 第 2 层）：256×192、6000 tick、四配置
//! （1/4 线程 × 休眠跳过开/关）逐 tick 全局哈希比对。
//! 浇注到 tick 4000，其后为沉降与入睡阶段（休眠唤醒语义在此阶段受测）。

mod common;

use common::{sim, SAND, WATER};
use sand_core::Op;

#[test]
fn four_configs_identical_hash_stream() {
    let configs = [(1usize, true), (4, true), (1, false), (4, false)];
    let mut sims: Vec<_> = configs.iter().map(|&(t, sk)| sim(4, 3, 0xC0FFEE, t, sk)).collect();
    let setup = [
        Op::Fill { material: 1, x0: 0, y0: 188, x1: 255, y1: 191 },
        Op::Fill { material: 1, x0: 100, y0: 120, x1: 160, y1: 124 },
    ];
    for s in &mut sims {
        s.apply_setup(&setup);
    }
    for tick in 0..6_000u64 {
        let ops = pour(tick);
        for s in &mut sims {
            s.step(&ops);
        }
        let h0 = sims[0].state_hash();
        for (i, s) in sims.iter().enumerate().skip(1) {
            assert_eq!(
                s.state_hash(),
                h0,
                "tick {tick}: 配置 {:?} 与 {:?} 分叉",
                configs[i],
                configs[0]
            );
        }
    }
}

fn pour(tick: u64) -> Vec<Op> {
    let mut ops = vec![];
    if tick < 4000 && tick % 3 == 0 {
        ops.push(Op::Brush { material: SAND, x: 130, y: 10, r: 2 });
    }
    if tick < 4000 && tick % 5 == 0 {
        ops.push(Op::Brush { material: WATER, x: 60, y: 10, r: 2 });
        ops.push(Op::Brush { material: WATER, x: 200, y: 10, r: 2 });
    }
    ops
}
