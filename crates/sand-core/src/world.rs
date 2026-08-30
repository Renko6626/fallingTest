//! 世界 = W×H chunk 的 flat Vec + 行主序静态页表（charter §9 缝 1）。
//! 越界读返回 WALL 哨兵；brush/fill 与场景 setup 共用同一写入路径（spec §5.2）。

use crate::cell::Cell;
use crate::chunk::{Chunk, CHUNK, DirtyRect};
use crate::material::{MaterialTable, Category, MAT_WALL};

pub const WALL_SENTINEL: Cell = Cell(MAT_WALL as u32);

/// 确定性输入操作（M0：脚本化 brush；InputFrame 正式编码留 M4）。
#[derive(Clone, Debug)]
pub enum Op {
    Brush { material: u8, x: i32, y: i32, r: i32 },
    Fill { material: u8, x0: i32, y0: i32, x1: i32, y1: i32 },
}

pub struct World {
    pub width_chunks: usize,
    pub height_chunks: usize,
    pub chunks: Vec<Chunk>,
    pub tick: u64,
    pub seed: u64,
}

impl World {
    pub fn new(width_chunks: usize, height_chunks: usize, seed: u64) -> World {
        assert!(width_chunks >= 1 && height_chunks >= 1, "世界至少 1×1 chunk");
        let mut chunks = Vec::with_capacity(width_chunks * height_chunks);
        for _ in 0..width_chunks * height_chunks {
            let mut c = Chunk::new();
            c.dirty = DirtyRect::FULL; // 启动 tick 全扫（spec §1.4）
            chunks.push(c);
        }
        World { width_chunks, height_chunks, chunks, tick: 0, seed }
    }

    pub fn width(&self) -> i32 {
        (self.width_chunks * CHUNK) as i32
    }

    pub fn height(&self) -> i32 {
        (self.height_chunks * CHUNK) as i32
    }

    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.width() && y < self.height()
    }

    pub fn chunk_index(&self, cx: usize, cy: usize) -> usize {
        cy * self.width_chunks + cx
    }

    pub fn cell(&self, x: i32, y: i32) -> Cell {
        if !self.in_bounds(x, y) {
            return WALL_SENTINEL;
        }
        let (ci, li) = self.locate(x, y);
        self.chunks[ci].cells[li]
    }

    fn locate(&self, x: i32, y: i32) -> (usize, usize) {
        let (x, y) = (x as usize, y as usize);
        let ci = self.chunk_index(x / CHUNK, y / CHUNK);
        (ci, (y % CHUNK) * CHUNK + (x % CHUNK))
    }

    /// brush/setup 共用写入路径：写 cell + 盖戳 + 脏标记 ±1。
    /// 液体方向记忆按 x 奇偶初始化（确定性且无整体偏置）。
    ///
    /// `pub(crate)`（M1 Task 4 起）：粒子落格提交复用同一写入路径，保证脏矩形
    /// 合并 + chunk 唤醒对粒子写入与 brush/setup 写入一视同仁（spec §5 明文）。
    pub(crate) fn set_cell_stamped(&mut self, table: &MaterialTable, x: i32, y: i32, material: u8, stamp: u8) {
        if !self.in_bounds(x, y) {
            return;
        }
        let mut cell = Cell::pack(material, stamp);
        if table.category(material) == Category::Liquid {
            cell = cell.with_dir(x & 1 == 1);
        }
        let (ci, li) = self.locate(x, y);
        self.chunks[ci].cells[li] = cell;
        self.mark_dirty_around(x, y);
    }

    /// (x,y)±1 邻域并入所辖 chunk 的 next_dirty（跨界唤醒，spec §1.4）。
    pub fn mark_dirty_around(&self, x: i32, y: i32) {
        let x0 = (x - 1).max(0);
        let y0 = (y - 1).max(0);
        let x1 = (x + 1).min(self.width() - 1);
        let y1 = (y + 1).min(self.height() - 1);
        let (cx0, cy0) = (x0 as usize / CHUNK, y0 as usize / CHUNK);
        let (cx1, cy1) = (x1 as usize / CHUNK, y1 as usize / CHUNK);
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                let bx0 = (cx * CHUNK) as i32;
                let by0 = (cy * CHUNK) as i32;
                self.chunks[self.chunk_index(cx, cy)].next_dirty.merge_rect(
                    (x0.max(bx0) - bx0) as u8,
                    (y0.max(by0) - by0) as u8,
                    (x1.min(bx0 + CHUNK as i32 - 1) - bx0) as u8,
                    (y1.min(by0 + CHUNK as i32 - 1) - by0) as u8,
                );
            }
        }
    }

    pub fn apply_op(&mut self, table: &MaterialTable, op: &Op, stamp: u8) {
        match *op {
            Op::Brush { material, x, y, r } => {
                for dy in -r..=r {
                    for dx in -r..=r {
                        if dx * dx + dy * dy <= r * r {
                            self.set_cell_stamped(table, x + dx, y + dy, material, stamp);
                        }
                    }
                }
            }
            Op::Fill { material, x0, y0, x1, y1 } => {
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        self.set_cell_stamped(table, x, y, material, stamp);
                    }
                }
            }
        }
    }

    /// 按材料统计 cell 数（守恒测试用）。
    pub fn count_material(&self, material: u8) -> usize {
        self.chunks
            .iter()
            .flat_map(|c| c.cells.iter())
            .filter(|c| c.material() == material)
            .count()
    }
}
