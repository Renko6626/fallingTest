//! 64×64 chunk 与脏矩形（spec §1.3–§1.4）。
//! `next_dirty` 用原子 min/max 合并——可交换可结合 ⇒ 调度无关（P4）；
//! 这是相内唯一允许的跨任务共享写。

use std::sync::atomic::{AtomicU8, Ordering};

use crate::cell::Cell;

pub const CHUNK: usize = 64;
pub const CELLS_PER_CHUNK: usize = CHUNK * CHUNK;

/// 本地坐标闭区间矩形；`x0 > x1` 表示空。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirtyRect {
    pub x0: u8,
    pub y0: u8,
    pub x1: u8,
    pub y1: u8,
}

impl DirtyRect {
    pub const EMPTY: DirtyRect = DirtyRect { x0: u8::MAX, y0: u8::MAX, x1: 0, y1: 0 };
    pub const FULL: DirtyRect =
        DirtyRect { x0: 0, y0: 0, x1: (CHUNK - 1) as u8, y1: (CHUNK - 1) as u8 };

    pub fn is_empty(&self) -> bool {
        self.x0 > self.x1 || self.y0 > self.y1
    }
}

/// 原子累积版脏矩形。merge 只做 fetch_min/fetch_max（Relaxed 足够：
/// 合并可交换可结合，相位间由 rayon join 屏障定序）。
#[derive(Debug)]
pub struct AtomicDirty {
    x0: AtomicU8,
    y0: AtomicU8,
    x1: AtomicU8,
    y1: AtomicU8,
}

impl AtomicDirty {
    pub fn empty() -> AtomicDirty {
        AtomicDirty {
            x0: AtomicU8::new(u8::MAX),
            y0: AtomicU8::new(u8::MAX),
            x1: AtomicU8::new(0),
            y1: AtomicU8::new(0),
        }
    }

    pub fn merge_rect(&self, x0: u8, y0: u8, x1: u8, y1: u8) {
        self.x0.fetch_min(x0, Ordering::Relaxed);
        self.y0.fetch_min(y0, Ordering::Relaxed);
        self.x1.fetch_max(x1, Ordering::Relaxed);
        self.y1.fetch_max(y1, Ordering::Relaxed);
    }

    /// 是否有任何标记（相位边界唤醒检查；merge 只会让 x0 变小，非空 ⟺ x0 ≠ MAX）。
    pub fn is_marked(&self) -> bool {
        self.x0.load(Ordering::Relaxed) != u8::MAX
    }

    /// 取出并清空（tick 末封帧调用，单线程语境）。
    pub fn take(&self) -> DirtyRect {
        let r = DirtyRect {
            x0: self.x0.swap(u8::MAX, Ordering::Relaxed),
            y0: self.y0.swap(u8::MAX, Ordering::Relaxed),
            x1: self.x1.swap(0, Ordering::Relaxed),
            y1: self.y1.swap(0, Ordering::Relaxed),
        };
        if r.is_empty() { DirtyRect::EMPTY } else { r }
    }
}

pub struct Chunk {
    pub cells: [Cell; CELLS_PER_CHUNK],
    /// 本 tick 扫描范围（tick 起点冻结）。
    pub dirty: DirtyRect,
    /// 本 tick 写入积累，tick 末 take() 换入 dirty。
    pub next_dirty: AtomicDirty,
}

impl Chunk {
    pub fn new() -> Chunk {
        Chunk {
            cells: [Cell::AIR; CELLS_PER_CHUNK],
            dirty: DirtyRect::EMPTY,
            next_dirty: AtomicDirty::empty(),
        }
    }
}

impl Default for Chunk {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_take_roundtrip() {
        let d = AtomicDirty::empty();
        assert!(d.take().is_empty());
        d.merge_rect(5, 6, 5, 6);
        d.merge_rect(2, 8, 3, 9);
        assert_eq!(d.take(), DirtyRect { x0: 2, y0: 6, x1: 5, y1: 9 });
        assert!(d.take().is_empty(), "take 后应清空");
    }

    #[test]
    fn merge_is_commutative() {
        let a = AtomicDirty::empty();
        let b = AtomicDirty::empty();
        a.merge_rect(1, 1, 2, 2);
        a.merge_rect(10, 0, 12, 5);
        b.merge_rect(10, 0, 12, 5);
        b.merge_rect(1, 1, 2, 2);
        assert_eq!(a.take(), b.take());
    }
}
