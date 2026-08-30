//! 世界 = W×H chunk 的 flat Vec + 行主序静态页表（charter §9 缝 1）。
//! 越界读返回 WALL 哨兵；brush/fill 与场景 setup 共用同一写入路径（spec §5.2）。

use crate::cell::Cell;
use crate::chunk::{Chunk, CHUNK, DirtyRect};
use crate::fixed::Fx;
use crate::material::{MaterialTable, Category, MAT_WALL};
use crate::rng;

pub const WALL_SENTINEL: Cell = Cell(MAT_WALL as u32);

/// 确定性输入操作（M0：脚本化 brush；`Op::Emit` 为 M1 Task 5 新增发射器；
/// InputFrame 正式编码留 M4）。
#[derive(Clone, Debug)]
pub enum Op {
    Brush { material: u8, x: i32, y: i32, r: i32 },
    Fill { material: u8, x0: i32, y0: i32, x1: i32, y1: i32 },
    /// 发射器（spec §7）：在 `(x, y)` 生成 `count` 个粒子，初速 `(vx, vy)`，
    /// 每个粒子的速度各自独立加 `[-jitter, +jitter]` 抖动（`emit_jitter`）。
    /// harness 场景 RON 里写十进制小数，加载期一次性 round 量化为本处的
    /// `Fx`（`sand-harness::scenario::quantize_fx`）——core 边界只见 `Fx`。
    Emit { material: u8, x: Fx, y: Fx, vx: Fx, vy: Fx, count: u16, jitter: Fx },
}

/// 生成队列条目（M1 spec §4 第 3 步 a）：由 `Op::Emit`（本任务）/ 未来
/// `Op::Explode` 在 ops 阶段产出，经调用方提供的队列（`Sim::spawn_queue`）
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

/// `Op::Emit` 里 vx 抖动用的 `rng_u32` `attempt` 标号。
const EMIT_ROLL_VX: u32 = 0;
/// `Op::Emit` 里 vy 抖动用的 `attempt` 标号。**`attempt` 参数在此被挪用为
/// "同一粒子内第几次独立掷骰"，而非其在 DDA/落格语境里的原始"重试次数"
/// 语义**——两者本质都是"同 salt 下需要互相独立的第 N 骰"，charter §11
/// 翻案 4 只要求"同帧同格多骰不同流/参数"，未强制 attempt 只能表示重试。
/// 若 vx/vy 复用同一个随机数，两轴抖动会完全相关（同号同幅度），破坏
/// 抖动的各向同性，故必须用不同 attempt 区分（salt 固定为粒子序号 i 不变）。
const EMIT_ROLL_VY: u32 = 1;

/// 把 32-bit 随机数映射到 `[-jitter, +jitter]`（Fx raw 域闭区间），纯整数
/// 运算、无浮点、无运行时除法（唯一除法点在 `fixed.rs::from_ratio`，此处
/// 只用移位）。
///
/// 映射算法：设 `width = 2*jitter.0 + 1`（Fx raw 单位，含两端点共 `width`
/// 个整数值）。`r` 均匀分布在 `[0, u32::MAX]`，右移 32 位重缩放
/// `(r as u64).wrapping_mul(width) 右移 32 位`把它等比例映射到整数区间
/// `[0, width)`（等价于除以 `2^32 / width` 但不做运行时除法）；再减去
/// `jitter.0` 平移居中，落入 `[-jitter.0, jitter.0]`：`r = 0` 时缩放结果为
/// 0，最终为 `-jitter.0`（下界可达）；`r = u32::MAX` 时缩放结果逼近但小于
/// `width`，即最大为 `2*jitter.0`，最终为 `+jitter.0`（上界可达）。
/// `jitter.0 <= 0` 视为"无抖动"直接返回零（调用方按 harness 量化约定
/// 保证非负，这里兜底避免符号翻转出现反向区间）。
fn emit_jitter(r: u32, jitter: Fx) -> Fx {
    if jitter.0 <= 0 {
        return Fx::ZERO;
    }
    let width = 2i64 * jitter.0 as i64 + 1;
    let scaled = ((r as u64) * (width as u64)) >> 32;
    Fx(scaled as i32 - jitter.0)
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

    /// 应用一个 `Op`（ops 阶段，spec §4 第 1 步）。`stamp` 供网格写入类
    /// 操作（Brush/Fill）盖戳；`fseed` 供 `Op::Emit` 的抖动掷骰（`rng::
    /// frame_seed(world.seed, tick)`，调用方与网格四相同一口径）；`spawns`
    /// 是本 tick 粒子生成队列的输出参数——`Op::Emit` 把产出的
    /// [`SpawnRequest`] 追加进去，调用方（`Sim::step`/`Sim::apply_setup`）
    /// 负责随后把它们并入 `Particles` 的入队序（Emit 本身不直接碰
    /// `Particles`，保持 world.rs 不知道粒子池存在——白名单通信介质走
    /// 队列，而非类型耦合）。
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
                // 抖动流的格坐标锚点用发射点本身的格（不逐粒子变化——同一次
                // Emit 的全部粒子共享 (x,y)，用 salt = 粒子序号 i 区分掷骰，
                // 而非靠坐标区分，否则同点多次 Emit 会撞同一批随机数）。
                let gx = x.to_cell();
                let gy = y.to_cell();
                for i in 0..count as u32 {
                    let rx = rng::rng_u32(fseed, rng::STREAM_EMIT, gx, gy, i, EMIT_ROLL_VX);
                    let ry = rng::rng_u32(fseed, rng::STREAM_EMIT, gx, gy, i, EMIT_ROLL_VY);
                    spawns.push(SpawnRequest {
                        material,
                        x,
                        y,
                        vx: vx + emit_jitter(rx, jitter),
                        vy: vy + emit_jitter(ry, jitter),
                    });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::{Category, MaterialDef};

    fn test_table() -> MaterialTable {
        let def = |id: u8, name: &str, category: Category, density: u16| MaterialDef {
            id,
            name: name.into(),
            category,
            density,
            color: (0, 0, 0),
        };
        MaterialTable::new(vec![
            def(0, "air", Category::Static, 0),
            def(1, "wall", Category::Static, 100),
            def(2, "water", Category::Liquid, 16),
        ])
        .unwrap()
    }

    // ==================== emit_jitter：映射范围与金值 ====================

    #[test]
    fn emit_jitter_zero_jitter_is_always_zero() {
        assert_eq!(emit_jitter(0, Fx::ZERO), Fx::ZERO);
        assert_eq!(emit_jitter(u32::MAX, Fx::ZERO), Fx::ZERO);
        // 负 jitter 视为非法输入，兜底返回零而非翻转符号。
        assert_eq!(emit_jitter(u32::MAX, Fx(-1)), Fx::ZERO);
    }

    #[test]
    fn emit_jitter_reaches_both_closed_bounds() {
        let jitter = Fx::from_int(1); // raw = 0x10000
        assert_eq!(emit_jitter(0, jitter), -jitter, "r=0 应触达下界 -jitter");
        assert_eq!(emit_jitter(u32::MAX, jitter), jitter, "r=u32::MAX 应触达上界 +jitter");
    }

    #[test]
    fn emit_jitter_midpoint_is_near_zero() {
        // r = 2^31（值域中点）应落在 0 附近（±1 raw 单位内，取决于 width 奇偶）。
        let jitter = Fx::from_int(4);
        let mid = emit_jitter(1u32 << 31, jitter);
        assert!(mid.0.abs() <= 1, "值域中点应映射到 0 附近，实际 {}", mid.0);
    }

    // ==================== Op::Emit：salt/attempt 独立性（任务书要求）====================

    #[test]
    fn emit_produces_requested_count_with_unjittered_position() {
        let t = test_table();
        let mut w = World::new(1, 1, 0xABCD);
        let fseed = rng::frame_seed(w.seed, w.tick);
        let (ex, ey) = (Fx::from_int(10), Fx::from_int(10));
        let op = Op::Emit {
            material: 2,
            x: ex,
            y: ey,
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            count: 5,
            jitter: Fx::from_int(1),
        };
        let mut spawns = Vec::new();
        w.apply_op(&t, &op, 0, fseed, &mut spawns);
        assert_eq!(spawns.len(), 5, "count 个粒子必须全部产出（Emit 不吃容量限流，那是 spawn 阶段的事）");
        for s in &spawns {
            assert_eq!(s.material, 2);
            assert_eq!((s.x, s.y), (ex, ey), "位置不抖动，全部粒子共享发射点");
        }
    }

    #[test]
    fn emit_salt_differentiates_particles_and_attempt_differentiates_vx_vy() {
        let t = test_table();
        let mut w = World::new(2, 2, 0xABCD);
        let fseed = rng::frame_seed(w.seed, w.tick);
        let op = Op::Emit {
            material: 2,
            x: Fx::from_int(10),
            y: Fx::from_int(10),
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            count: 8,
            jitter: Fx::from_int(2),
        };
        let mut spawns = Vec::new();
        w.apply_op(&t, &op, 0, fseed, &mut spawns);
        assert_eq!(spawns.len(), 8);

        // salt 独立性：不同粒子序号 i 的 vx 抖动必须有区分度（不能全部相同）。
        let mut vxs: Vec<i32> = spawns.iter().map(|s| s.vx.0).collect();
        vxs.sort();
        vxs.dedup();
        assert!(vxs.len() > 1, "8 个粒子的 vx 抖动不应全部相同：{vxs:?}");

        // attempt 独立性：同一粒子内 vx/vy 两骰不应总是给出相同的抖动值
        // （若 attempt 未生效、vx/vy 复用了同一随机数，这里会全部相等）。
        assert!(
            spawns.iter().any(|s| s.vx.0 != s.vy.0),
            "至少一个粒子的 vx/vy 抖动应不同（各自独立掷骰，而非共享一次掷骰）"
        );
    }

    #[test]
    fn emit_is_deterministic_given_same_fseed() {
        let t = test_table();
        let mut w = World::new(1, 1, 42);
        let fseed = rng::frame_seed(w.seed, w.tick);
        let op = Op::Emit {
            material: 2,
            x: Fx::from_int(5),
            y: Fx::from_int(5),
            vx: Fx::from_ratio(1, 2),
            vy: Fx::from_ratio(1, 4),
            count: 4,
            jitter: Fx::from_int(1),
        };
        let mut a = Vec::new();
        let mut b = Vec::new();
        w.apply_op(&t, &op, 0, fseed, &mut a);
        w.apply_op(&t, &op, 0, fseed, &mut b);
        let av: Vec<(i32, i32)> = a.iter().map(|s| (s.vx.0, s.vy.0)).collect();
        let bv: Vec<(i32, i32)> = b.iter().map(|s| (s.vx.0, s.vy.0)).collect();
        assert_eq!(av, bv, "同一 fseed 重复应用 Emit 必须给出逐粒子完全相同的抖动序列");
    }
}
