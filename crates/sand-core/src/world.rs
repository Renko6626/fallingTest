//! 世界 = W×H chunk 的 flat Vec + 行主序静态页表（charter §9 缝 1）。
//! 越界读返回 WALL 哨兵；brush/fill 与场景 setup 共用同一写入路径（spec §5.2）。

use crate::cell::Cell;
use crate::chunk::{Chunk, CHUNK, DirtyRect};
use crate::emit;
use crate::explode;
use crate::fixed::Fx;
use crate::material::{MaterialTable, Category, MAT_WALL};

pub const WALL_SENTINEL: Cell = Cell::pack(MAT_WALL, 0);

/// 确定性输入操作（M0：脚本化 brush；`Op::Emit` 为 M1 Task 5 新增发射器；
/// InputFrame 正式编码留 M4）。
#[derive(Clone, Debug)]
pub enum Op {
    Brush { material: u8, x: i32, y: i32, r: i32 },
    Fill { material: u8, x0: i32, y0: i32, x1: i32, y1: i32 },
    /// 发射器（spec §7）：在 `(x, y)` 生成 `count` 个粒子，初速 `(vx, vy)`，
    /// 每个粒子的速度各自独立加 `[-jitter, +jitter]` 抖动（`emit::emit_jitter`）。
    /// harness 场景 RON 里写十进制小数，加载期一次性 round 量化为本处的
    /// `Fx`（`sand-harness::scenario::quantize_fx`）——core 边界只见 `Fx`。
    Emit { material: u8, x: Fx, y: Fx, vx: Fx, vy: Fx, count: u16, jitter: Fx },
    /// 爆炸（spec §6，Noita 射线模型）：以 `(x, y)` 为圆心、半径 `r` 格的
    /// Bresenham 圆周每格发一条 DDA 射线，射线初始能量 `power`，逐格消耗
    /// `MaterialTable::blast_cost`，能量 ≥ 格消耗即摧毁该格（置 air + 溅射
    /// 粒子，或按 `MaterialTable::vaporize_threshold` 判定汽化——删除、不
    /// 溅射，用户裁决 2026-08-30，见 `explode::fire_ray` 文档），能量耗尽或撞
    /// `BLAST_COST_INFINITE` 材料（M1 里即 wall）断线。
    /// 整数签名——圆心/半径是格坐标，不经过 `Fx` 量化（与 `Op::Emit` 的
    /// 连续坐标不同，爆炸是格对齐的离散几何）。
    Explode { x: i32, y: i32, r: i32, power: u32 },
}

/// 生成队列条目（M1 spec §4 第 3 步 a）：由 `Op::Emit`/`Op::Explode`
/// 在 ops 阶段产出，经调用方提供的队列（`Sim::spawn_queue`）
/// 或测试代码经 `Sim::queue_spawn` 压入；本 tick 粒子相开头按入队序 drain
/// （`lib.rs::Sim::step`）。定义在此（而非 `lib.rs`）是因为 `World::apply_op`
/// 需要直接产出它，避免 world.rs → lib.rs 的反向依赖。
#[derive(Clone, Copy, Debug)]
pub(crate) struct SpawnRequest {
    pub(crate) material: u8,
    pub(crate) x: Fx,
    pub(crate) y: Fx,
    pub(crate) vx: Fx,
    pub(crate) vy: Fx,
}

pub struct World {
    pub width_chunks: usize,
    pub height_chunks: usize,
    pub chunks: Vec<Chunk>,
    pub tick: u64,
    pub seed: u64,
    /// 爆炸近心汽化诊断计数（vaporize_threshold，用户裁决 2026-08-30）：
    /// `explode::fire_ray` 判定某格汽化（删除、不入生成队列）时 +1。纯诊断
    /// 计数器，仿 `Particles::rejected_total`/`buried_total` 先例——**不
    /// 参与** `hash::state_hash`（该函数只读 `tick` + `chunks`，见
    /// hash.rs:23-33），不影响 SyncTest 哈希比对，但本身仍是（状态,输入）
    /// 的确定性函数，可供测试直接断言。`pub(crate)`：`explode::fire_ray`
    /// 跨模块直接自增（纯搬移，M1 收口 2026-08-30）。
    pub(crate) vaporized_total: u64,
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
        World { width_chunks, height_chunks, chunks, tick: 0, seed, vaporized_total: 0 }
    }

    /// 爆炸近心汽化诊断计数（见字段文档）。
    pub fn vaporized_total(&self) -> u64 {
        self.vaporized_total
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
        // 液体/气体方向记忆按 x 奇偶初始化（确定性且无整体偏置）；气体与液体
        // 同待遇（M2 Task 1）：水平扩散共用 dir 承诺语义，缺初始化会给烟一个
        // 系统性的首选侧。
        if matches!(table.category(material), Category::Liquid | Category::Gas) {
            cell = cell.with_dir(x & 1 == 1);
        }
        let (ci, li) = self.locate(x, y);
        self.chunks[ci].cells[li] = cell;
        self.mark_dirty_around(x, y);
    }

    /// 给已存在的 cell 写竖直速度位（Layer G Task 3 的 **P→G 通路**）。
    ///
    /// 只在 `particle::commit` 的落格分支调用，紧跟 `set_cell_stamped` 之后
    /// ——那一格刚被写过，脏矩形已标，这里不重复标记。
    pub(crate) fn set_cell_vel(&mut self, x: i32, y: i32, v: u8) {
        if !self.in_bounds(x, y) {
            return;
        }
        let (ci, li) = self.locate(x, y);
        self.chunks[ci].cells[li] = self.chunks[ci].cells[li].with_vel(v);
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

    /// 应用一个 `Op`（ops 阶段，spec §4 第 1 步）。`stamp` 供网格写入类
    /// 操作（Brush/Fill/Explode 摧毁）盖戳，`Op::Emit`/`Op::Explode` 里
    /// 另外折进抖动 `attempt`（见 `emit::emit_attempt`）；`fseed` 供两者的抖动
    /// 掷骰（`rng::frame_seed(world.seed, tick)`，调用方与网格四相同一
    /// 口径）；`op_idx` 是本 `op` 在调用方本 tick `ops` 切片里的下标
    /// （`enumerate()` 天然定序），折进抖动 `salt`（`Op::Emit` 见
    /// `emit::emit_salt`，`Op::Explode` 见 `explode::fire_ray` 文档），区分
    /// 同 tick 内多个同类型 `Op`；`spawns` 是本 tick 粒子生成队列的输出
    /// 参数——`Op::Emit`/`Op::Explode` 把产出的 [`SpawnRequest`] 追加进去，
    /// 调用方（`Sim::step`/`Sim::apply_setup`）负责随后把它们并入
    /// `Particles` 的入队序（world.rs 本身不直接碰 `Particles`，保持不知道
    /// 粒子池存在——白名单通信介质走队列，而非类型耦合）。
    ///
    /// Emit/Explode 分支体分别委派给 [`emit::apply_emit`]/
    /// [`explode::apply_explode`]（M1 收口 2026-08-30，纯搬移拆分 world.rs
    /// 三职责为 world/emit/explode 三模块，一行逻辑未改）。
    ///
    /// `pub(crate)`（Task 5 起，随 `SpawnRequest` 一起收紧）：出参类型
    /// `SpawnRequest` 本就 `pub(crate)`，外部 crate 拿不到能传的实参，保持
    /// `pub` 只会产生"公开但不可调用"的私有类型泄漏警告，无实际开放意义。
    pub(crate) fn apply_op(
        &mut self,
        table: &MaterialTable,
        op: &Op,
        stamp: u8,
        fseed: u32,
        op_idx: usize,
        spawns: &mut Vec<SpawnRequest>,
    ) {
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
            Op::Emit { material, x, y, vx, vy, count, jitter } => {
                emit::apply_emit(material, x, y, vx, vy, count, jitter, stamp, fseed, op_idx, spawns);
            }
            Op::Explode { x, y, r, power } => {
                explode::apply_explode(self, table, x, y, r, power, stamp, fseed, op_idx, spawns);
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
