//! WriteWindow：相内并行任务的读写窗口 = 本 chunk + 16px halo（spec §3.2–§3.3）。
//!
//! # 安全论证（P4 实现面）
//! 同相 chunk 中心间距 ≥128px，窗口宽 96px（64+2×16），两两必然不相交
//! （`scheduler::tests::phase_windows_disjoint` 穷举验证）。因此同相任务经
//! 裸指针写入的 cell 区域互斥，无数据竞争；唯一共享写是邻 chunk 的
//! `next_dirty` 原子矩形合并（可交换可结合）。跨相位由 rayon join 屏障定序。
//! debug 构建下每次 cell 读写断言坐标落在窗口内——越界即 panic（写域执法）。

use crate::cell::Cell;
use crate::chunk::{Chunk, CHUNK};
use crate::world::WALL_SENTINEL;

/// 影响半径上限（charter §4 r≤16 契约）。M0 实际移动半径 = 1。
/// 新增任何移动/探测规则必须自证半径 ≤ HALO 并复审脏矩形扩张常数（spec §3.3）。
pub const HALO: i32 = 16;

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
        }
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
                let bx0 = (cx * CHUNK) as i32;
                let by0 = (cy * CHUNK) as i32;
                // SAFETY: next_dirty 是原子字段，共享引用跨线程合并安全（可交换）。
                let nd = unsafe { &(*self.chunks.0.add(cy * self.width_chunks + cx)).next_dirty };
                nd.merge_rect(
                    (x0.max(bx0) - bx0) as u8,
                    (y0.max(by0) - by0) as u8,
                    (x1.min(bx0 + CHUNK as i32 - 1) - bx0) as u8,
                    (y1.min(by0 + CHUNK as i32 - 1) - by0) as u8,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn oob_read_is_wall() {
        let mut w = World::new(1, 1, 0);
        let ptr = ChunksPtr(w.chunks.as_mut_ptr());
        let win = WriteWindow::new(ptr, 1, 1, 0, 0);
        assert_eq!(win.get(-1, 0), WALL_SENTINEL);
        assert_eq!(win.get(0, 64), WALL_SENTINEL);
    }
}
