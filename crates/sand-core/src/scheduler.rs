//! 四相棋盘调度器（spec §3）。相 = (cx&1, cy&1)，相序按 tick%4 轮换，
//! 相内 chunk 经 rayon 任意调度——线程数只影响快慢不影响结果（SyncTest 执法）。

use rayon::prelude::*;

use crate::chunk::DirtyRect;
use crate::material::MaterialTable;
use crate::reaction::ReactionTable;
use crate::rng;
use crate::rules;
use crate::window::{ChunksPtr, WriteWindow};
use crate::world::{SpawnRequest, World};
use crate::ScanMode;

const PHASES: [(usize, usize); 4] = [(0, 0), (1, 0), (1, 1), (0, 1)];

fn phase_order(tick: u64) -> [(usize, usize); 4] {
    let r = (tick % 4) as usize;
    std::array::from_fn(|i| PHASES[(i + r) % 4])
}

/// 规范 tick 管线的网格部分（architecture §4，顺序即协议）：
/// 2. 网格四相 pass
/// 3'. 封帧：脏矩形交换、tick 递增
///
/// **输入应用（第 1 步）自 M3 起由 `Sim::step` 执行**（纯搬移，2026-09-02）：
/// 刚体相（第 3 步）必须插在输入之后、四相之前，而 `World` 不持有刚体；把 ops
/// 循环提到 `Sim::step` 里是唯一不引入反向依赖的接法。外部可观测顺序
/// （ops → 刚体 → 四相 → 粒子 → 封帧）与架构 §4 一致。
///
/// `pub(crate)`：`spawns: &mut Vec<SpawnRequest>` 的 `SpawnRequest` 是
/// `pub(crate)` 类型，本函数保持 `pub` 只会造成"签名公开但外部拿不到实参
/// 类型"的私有类型泄漏警告，收紧到 `pub(crate)` 与实际可达性一致
/// （唯一调用方是同 crate 的 `Sim::step`）。
pub(crate) fn step(
    world: &mut World,
    table: &MaterialTable,
    reactions: &ReactionTable,
    pool: &rayon::ThreadPool,
    scan: ScanMode,
    spawns: &mut Vec<SpawnRequest>,
) {
    let tick = world.tick;
    let fseed = rng::frame_seed(world.seed, tick);

    let (wc, hc) = (world.width_chunks, world.height_chunks);
    let ptr = ChunksPtr(world.chunks.as_mut_ptr());

    for (px, py) in phase_order(tick) {
        // 休眠为 chunk 粒度，且在每个相位边界重查唤醒：上 tick 脏（dirty）∪
        // 本 tick 更早相位积累的标记（next_dirty）。屏障后原子合并结果与
        // 调度无关 ⇒ 该判定确定（tick 583 分叉的修复，见 M0 spec §1.4 修订）。
        // 起始扫描矩形按模式（O1 spec §2.1）：Full/ChunkSleep 全量，LiveRect =
        // dirty ∪ next_dirty 快照——唤醒判定与起始矩形取同一快照，防漏合。
        let ids: Vec<(usize, DirtyRect)> = (0..hc)
            .flat_map(|cy| (0..wc).map(move |cx| (cx, cy)))
            .filter(|&(cx, cy)| cx & 1 == px && cy & 1 == py)
            .filter_map(|(cx, cy)| {
                let ci = cy * wc + cx;
                if scan == ScanMode::Full {
                    return Some((ci, DirtyRect::FULL));
                }
                // SAFETY: 相位边界单线程语境；并行段尚未开始。
                let c = unsafe { &*ptr.0.add(ci) };
                let awake = c.dirty.union(c.next_dirty.snapshot());
                if awake.is_empty() {
                    return None;
                }
                match scan {
                    ScanMode::LiveRect => Some((ci, awake)),
                    _ => Some((ci, DirtyRect::FULL)),
                }
            })
            .collect();
        pool.install(|| {
            ids.par_iter().for_each(|&(ci, start)| {
                let (cx, cy) = (ci % wc, ci / wc);
                let win = WriteWindow::new(ptr, wc, hc, cx, cy);
                rules::update_chunk(
                    &win,
                    table,
                    reactions,
                    tick,
                    fseed,
                    start,
                    (cx * 64) as i32,
                    (cy * 64) as i32,
                );
            });
        });
        // pool.install + par_iter 完成即相位屏障。
        // 屏障之后立刻按 **chunk index 升序** drain 本相位各 chunk 的溅射生成
        // 请求（Layer G Task 3，spec §6.4）：`ids` 由 cy 外层 / cx 内层构造，
        // ci = cy*wc+cx 天然升序。最终 id 序 = (相位序, chunk index, chunk 内
        // 扫描序)，三者都是状态的纯函数 ⇒ 与线程数、与 rayon 的完成顺序无关。
        // 休眠 chunk 不在 ids 里，它们也不可能产出请求（根本没被扫描）。
        for &(ci, _) in &ids {
            // SAFETY: 相位屏障之后是单线程语境，并行段已结束。
            let buf = unsafe { &mut (*ptr.0.add(ci)).spawn_buf };
            spawns.append(buf);
        }
    }

    for c in world.chunks.iter_mut() {
        c.dirty = c.next_dirty.take();
    }
    world.tick += 1;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::CHUNK;
    use crate::window::HALO;

    /// spec §5.4：同相任意两 chunk 的（未夹断）窗口不相交；四相并集覆盖全部 chunk。
    #[test]
    fn phase_windows_disjoint_and_phases_cover_all() {
        let (wc, hc) = (6, 5);
        let mut covered = vec![false; wc * hc];
        for (px, py) in PHASES {
            let members: Vec<(i32, i32)> = (0..hc as i32)
                .flat_map(|cy| (0..wc as i32).map(move |cx| (cx, cy)))
                .filter(|&(cx, cy)| cx as usize & 1 == px && cy as usize & 1 == py)
                .collect();
            for &(cx, cy) in &members {
                covered[cy as usize * wc + cx as usize] = true;
            }
            let win = |c: i32| (c * CHUNK as i32 - HALO, c * CHUNK as i32 + CHUNK as i32 + HALO - 1);
            for (i, &(ax, ay)) in members.iter().enumerate() {
                for &(bx, by) in &members[i + 1..] {
                    let (ax0, ax1) = win(ax);
                    let (bx0, bx1) = win(bx);
                    let (ay0, ay1) = win(ay);
                    let (by0, by1) = win(by);
                    let overlap = ax0 <= bx1 && bx0 <= ax1 && ay0 <= by1 && by0 <= ay1;
                    assert!(!overlap, "同相窗口相交：({ax},{ay}) vs ({bx},{by})");
                }
            }
        }
        assert!(covered.iter().all(|&c| c), "四相未覆盖全部 chunk");
    }

    #[test]
    fn phase_order_rotates_and_is_a_permutation() {
        for t in 0..8u64 {
            let mut o = phase_order(t).to_vec();
            o.sort();
            let mut p = PHASES.to_vec();
            p.sort();
            assert_eq!(o, p);
        }
        assert_ne!(phase_order(0), phase_order(1));
        assert_eq!(phase_order(0), phase_order(4));
    }
}
