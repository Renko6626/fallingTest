//! WriteWindow：相内并行任务的读写窗口 = 本 chunk + 16px halo（spec §3.2–§3.3）。
//!
//! # 安全论证（P4 实现面）
//! 同相 chunk 中心间距 ≥128px，窗口宽 96px（64+2×16），两两必然不相交
//! （`scheduler::tests::phase_windows_disjoint` 穷举验证）。因此同相任务经
//! 裸指针写入的 cell 区域互斥，无数据竞争；唯一共享写是邻 chunk 的
//! `next_dirty` 原子矩形合并（可交换可结合）。跨相位由 rayon join 屏障定序。
//! debug 构建下每次 cell 读写断言坐标落在窗口内——越界即 panic（写域执法）。

use crate::cell::{Cell, VEL_ONE, V_MAX_CELL};
use crate::chunk::{Chunk, DirtyRect, CHUNK};
use crate::material::DISPERSION_MAX;
use crate::world::{SpawnRequest, WALL_SENTINEL};

/// 影响半径上限（charter §4 r≤16 契约）。
///
/// 实际用掉多少（2026-08-31，Layer G Task 2 后）：见 [`MAX_WRITE_RADIUS`]
/// 的逐条推导，**r = 12 ≤ 16，余量 4**。
///
/// 下面的编译期断言把这条不等式从人肉纪律变成契约：谁把 `V_MAX_CELL` 提到 8
/// 格/tick 或把 `DISPERSION_MAX` 提到 12，编译直接不过。
///
/// 断言覆盖不到的部分仍需自证：新增**其他**移动/探测规则时，必须论证自己的
/// 读写半径 ≤ HALO 并复审脏矩形扩张常数（spec §3.3）。
pub const HALO: i32 = 16;

/// 单次 cell 更新实际用掉的最大读写半径（Layer G Task 2 时点，spec §5）。
///
/// 逐条推导：
/// 1. 子步循环最多 `n = V_MAX_CELL / VEL_ONE = 4` 步；世代戳（`rules::eval`）
///    保证每 cell 每 tick 只被 `eval` 一次，不会级联叠加。
/// 2. 色散一旦走到就撞停终止循环（`rules::Step::MovedSide`）⇒ **每 tick 至多
///    一次色散**。故最坏水平路径 = `(n − 1)` 次同向斜下 + 1 次满色散
///    = `3 + 8 = 11`；另一条候选（4 次全斜下 = 4）更小。
/// 3. 最坏竖直位移 = `n` = 4 < 11。
/// 4. `displace` 的探测半径 = 写半径；`side` 的探测路径 ≤ `dispersion` ≤ 写半径。
/// 5. `mark_dirty_around` 再 ±1。
///
/// ⇒ `(V_MAX_CELL/VEL_ONE − 1) + DISPERSION_MAX + 1 = 3 + 8 + 1 = 12`。
///
/// 两项是**串接**（同一 tick 内先斜下、后色散）而非各自独立取最大——正因为
/// 色散会终止子步循环，两者不可能各自吃满。
pub const MAX_WRITE_RADIUS: i32 =
    (V_MAX_CELL / VEL_ONE) as i32 - 1 + DISPERSION_MAX as i32 + 1;

const _: () = assert!(
    MAX_WRITE_RADIUS <= HALO,
    "r<=16 契约破裂：(V_MAX_CELL/VEL_ONE − 1) + DISPERSION_MAX + 1 必须 <= HALO"
);

/// 单 chunk 单 tick 的溅射脱格上限（Layer G Task 3，spec §6.5 本地防线）。
///
/// 本地计数不依赖任何全局状态，故与线程调度无关——这是它能当确定性限流的
/// 全部理由。超限即不脱格（该 cell 照旧停在网格里），**不是**排队等下 tick：
/// 排队需要跨 tick 状态，那会把限流变成状态机而不是纯判定。
///
/// 640×384 图 60 个 chunk ⇒ 最坏 3840 粒子/tick，仍在 `MAX_PARTICLES`（65536）
/// 的第二道防线之内。
pub const MAX_SPLASH_PER_CHUNK: usize = 64;

#[derive(Clone, Copy)]
pub(crate) struct ChunksPtr(pub *mut Chunk);
// SAFETY: 仅在相内并行段使用；写域互斥由上述几何论证保证。
unsafe impl Send for ChunksPtr {}
unsafe impl Sync for ChunksPtr {}

pub(crate) struct WriteWindow {
    chunks: ChunksPtr,
    width_chunks: usize,
    world_w: i32,
    world_h: i32,
    // 窗口闭区间（已与世界求交）
    wx0: i32,
    wy0: i32,
    wx1: i32,
    wy1: i32,
    // 本 chunk 索引（活矩形追踪，O1 spec §2.3）
    own_ci: usize,
    /// 任务本地活矩形（本地坐标）：本任务对自己 chunk 的写入 ±1 邻域实时并入。
    /// 恒常追踪（三模式共用，spec §2.4）；是否消费由扫描起始矩形决定。
    live: std::cell::Cell<DirtyRect>,
}

impl WriteWindow {
    pub(crate) fn new(
        chunks: ChunksPtr,
        width_chunks: usize,
        height_chunks: usize,
        cx: usize,
        cy: usize,
    ) -> WriteWindow {
        let world_w = (width_chunks * CHUNK) as i32;
        let world_h = (height_chunks * CHUNK) as i32;
        let ox = (cx * CHUNK) as i32;
        let oy = (cy * CHUNK) as i32;
        WriteWindow {
            chunks,
            width_chunks,
            world_w,
            world_h,
            wx0: (ox - HALO).max(0),
            wy0: (oy - HALO).max(0),
            wx1: (ox + CHUNK as i32 + HALO - 1).min(world_w - 1),
            wy1: (oy + CHUNK as i32 + HALO - 1).min(world_h - 1),
            own_ci: cy * width_chunks + cx,
            live: std::cell::Cell::new(DirtyRect::EMPTY),
        }
    }

    /// 设置扫描起始矩形（rules 在扫描开始时调用）。
    pub(crate) fn seed_live(&self, start: DirtyRect) {
        self.live.set(start);
    }

    /// 当前活矩形（本地坐标；rules 循环边界每步重读）。
    pub(crate) fn live_rect(&self) -> DirtyRect {
        self.live.get()
    }

    fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.wx0 && x <= self.wx1 && y >= self.wy0 && y <= self.wy1
    }

    /// SAFETY 前提：(x,y) 在世界内。
    unsafe fn cell_ptr(&self, x: i32, y: i32) -> *mut Cell {
        let (xu, yu) = (x as usize, y as usize);
        let ci = (yu / CHUNK) * self.width_chunks + xu / CHUNK;
        let li = (yu % CHUNK) * CHUNK + (xu % CHUNK);
        unsafe {
            let chunk = self.chunks.0.add(ci);
            (&raw mut (*chunk).cells).cast::<Cell>().add(li)
        }
    }

    pub(crate) fn get(&self, x: i32, y: i32) -> Cell {
        if x < 0 || y < 0 || x >= self.world_w || y >= self.world_h {
            return WALL_SENTINEL;
        }
        debug_assert!(self.contains(x, y), "窗口外读：({x},{y}) 窗口 [{},{}]×[{},{}]", self.wx0, self.wx1, self.wy0, self.wy1);
        unsafe { self.cell_ptr(x, y).read() }
    }

    pub(crate) fn set(&self, x: i32, y: i32, cell: Cell) {
        debug_assert!(
            x >= 0 && y >= 0 && x < self.world_w && y < self.world_h,
            "世界外写：({x},{y})"
        );
        debug_assert!(self.contains(x, y), "窗口外写：({x},{y}) 窗口 [{},{}]×[{},{}]", self.wx0, self.wx1, self.wy0, self.wy1);
        unsafe { self.cell_ptr(x, y).write(cell) };
        self.mark_dirty_around(x, y);
    }

    /// 往**本 chunk** 的溅射生成缓冲追加一条请求；达到
    /// [`MAX_SPLASH_PER_CHUNK`] 即拒绝（返回 `false`，调用方照旧把 cell 留在
    /// 网格里）。计数直接取缓冲长度，不另设计数器——缓冲每个相位屏障后被
    /// drain 清空，两者不可能不同步。
    pub(crate) fn push_spawn(&self, req: SpawnRequest) -> bool {
        // SAFETY: 只写 own_ci 这一块的 spawn_buf，写域互斥与 cells 完全同构
        // （同相 chunk 的窗口两两不交，见本文件顶部安全论证）。`&mut (*p).field`
        // 是 place 表达式，**不形成 `&mut Chunk`**——这一点很要紧：邻块此刻
        // 可能正持有本块 `next_dirty` 的 `&AtomicDirty`，整块可变引用会与之
        // 别名（UB）。同 `cell_ptr` 的纪律。
        let buf = unsafe { &mut (*self.chunks.0.add(self.own_ci)).spawn_buf };
        if buf.len() >= MAX_SPLASH_PER_CHUNK {
            return false;
        }
        buf.push(req);
        true
    }

    /// 与 `World::mark_dirty_around` 同语义（并行段专用，原子合并）。
    fn mark_dirty_around(&self, x: i32, y: i32) {
        let x0 = (x - 1).max(0);
        let y0 = (y - 1).max(0);
        let x1 = (x + 1).min(self.world_w - 1);
        let y1 = (y + 1).min(self.world_h - 1);
        let (cx0, cy0) = (x0 as usize / CHUNK, y0 as usize / CHUNK);
        let (cx1, cy1) = (x1 as usize / CHUNK, y1 as usize / CHUNK);
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                let ci = cy * self.width_chunks + cx;
                let bx0 = (cx * CHUNK) as i32;
                let by0 = (cy * CHUNK) as i32;
                let (lx0, ly0, lx1, ly1) = (
                    (x0.max(bx0) - bx0) as u8,
                    (y0.max(by0) - by0) as u8,
                    (x1.min(bx0 + CHUNK as i32 - 1) - bx0) as u8,
                    (y1.min(by0 + CHUNK as i32 - 1) - by0) as u8,
                );
                // SAFETY: next_dirty 是原子字段，共享引用跨线程合并安全（可交换）。
                let nd = unsafe { &(*self.chunks.0.add(ci)).next_dirty };
                nd.merge_rect(lx0, ly0, lx1, ly1);
                // 本 chunk 部分同时并入任务本地活矩形（O1）
                if ci == self.own_ci {
                    let mut live = self.live.get();
                    live.merge_point(lx0, ly0);
                    live.merge_point(lx1, ly1);
                    self.live.set(live);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixed::Fx;
    use crate::world::World;

    #[test]
    #[should_panic(expected = "窗口外写")]
    fn out_of_window_write_panics() {
        // 4×4 chunk 世界，取 (0,0) chunk 的窗口，往 (200,200) 写——必须被写域执法拦下。
        let mut w = World::new(4, 4, 0);
        let ptr = ChunksPtr(w.chunks.as_mut_ptr());
        let win = WriteWindow::new(ptr, 4, 4, 0, 0);
        win.set(200, 200, Cell::AIR);
    }

    /// 本地限流（spec §6.5）：第 65 次 `push_spawn` 起必须被拒，且拒绝是
    /// **纯本地**判定——不看全局粒子数、不看线程。超限的 cell 由调用方留在
    /// 网格里，不排队等下 tick（排队需要跨 tick 状态，那会把限流变成状态机）。
    #[test]
    fn push_spawn_is_capped_per_chunk() {
        let mut w = World::new(2, 2, 0);
        let ptr = ChunksPtr(w.chunks.as_mut_ptr());
        let win = WriteWindow::new(ptr, 2, 2, 0, 0);
        let req = || SpawnRequest {
            material: 3,
            x: Fx::ZERO,
            y: Fx::ZERO,
            vx: Fx::ZERO,
            vy: Fx::ZERO,
        };
        let accepted = (0..100).filter(|_| win.push_spawn(req())).count();
        assert_eq!(accepted, MAX_SPLASH_PER_CHUNK, "本地限流必须恰好放行 MAX_SPLASH_PER_CHUNK 条");
        assert_eq!(w.chunks[0].spawn_buf.len(), MAX_SPLASH_PER_CHUNK);
        assert!(w.chunks[1].spawn_buf.is_empty(), "只许写 own_ci 那一块的缓冲");
    }

    #[test]
    fn oob_read_is_wall() {
        let mut w = World::new(1, 1, 0);
        let ptr = ChunksPtr(w.chunks.as_mut_ptr());
        let win = WriteWindow::new(ptr, 1, 1, 0, 0);
        assert_eq!(win.get(-1, 0), WALL_SENTINEL);
        assert_eq!(win.get(0, 64), WALL_SENTINEL);
    }
}
