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

/// `Op::Emit` 里 vx 抖动用的骰子标号（并入 [`emit_attempt`] 的低位）。
const EMIT_ROLL_VX: u32 = 0;
/// `Op::Emit` 里 vy 抖动用的骰子标号。若 vx/vy 复用同一个随机数，两轴抖动
/// 会完全相关（同号同幅度），破坏抖动的各向同性，故必须与 `EMIT_ROLL_VX`
/// 用不同值区分。
const EMIT_ROLL_VY: u32 = 1;

/// `Op::Emit` 抖动 `salt`：折进"本 tick 内 op 序号"（`op_idx`，调用方对本
/// tick `ops` 切片 `enumerate()` 得到的下标，天然定序）与"粒子序号"（`i`，
/// 同一个 `Op::Emit` 内部第几个粒子），各占 32 位里的高/低 16 位。
///
/// **为什么必须有 `op_idx` 这一维**（Task 5 修复轮 1 I1）：抖动流的坐标锚点
/// 是发射点本身的格 `(gx, gy)`，不逐粒子变化。若同一 tick 里有两个
/// `Op::Emit` 命中同一个 `(gx, gy)`（同一发射点重复配置，或不同发射点量化
/// 后恰好落在同一格），仅用 `salt = i` 会让两个 Op 的第 i 个粒子拿到位级
/// 相同的抖动序列——直接违反 charter §11 翻案 4"同帧同格多次掷骰必须彼此
/// 不同"。`op_idx` 折进 salt 后，不同 Op 即便发射点重合，salt 也不同。
///
/// `i` 的上界是 `count: u16`（最大 65535，恰好 16 位）；`op_idx` 按同样的
/// 16 位截断——当前场景规模下一个 tick 的 `ops` 远不到 65536 条，
/// `debug_assert` 兜底防线，真触发即视为场景异常（不是安全问题：截断后仍
/// 是确定性的，只是可能与另一个 `op_idx` 折叠出同一个 salt，回退到 I1 修
/// 复前的风险，故用断言而非静默接受）。
fn emit_salt(op_idx: usize, i: u32) -> u32 {
    debug_assert!(
        op_idx <= u16::MAX as usize,
        "Op::Emit op_idx（{op_idx}）超出 emit_salt 的 16 位折叠范围，需要扩位"
    );
    ((op_idx as u32) << 16) | (i & 0xFFFF)
}

/// `Op::Emit` 抖动 `attempt`：折进"调用相位"（`stamp`）与"骰子标号"
/// （[`EMIT_ROLL_VX`]/[`EMIT_ROLL_VY`]），高 8 位给 `stamp`、最低 1 位给
/// 骰子标号。
///
/// **为什么需要 `stamp` 这一维**（Task 5 修复轮 1 I1 括注场景）：
/// `Sim::apply_setup` 与随后紧接的 tick 0 首个 `step()` 共享同一个
/// `fseed`——两者都是 `rng::frame_seed(seed, 0)`（setup 期 `world.tick`
/// 恒为 0，与真正 tick 0 的 fseed 计算式完全相同）。若 setup 里的
/// `Op::Emit` 与 tick-0 `script` 里的 `Op::Emit` 各自的 `op_idx` 都从 0
/// 起算（两者是完全独立的 `ops` 切片，互不知情），仍可能在同一发射点撞出
/// 相同 `salt`。`stamp` 是这两条路径里唯一保证不同的信号
/// （`SETUP_STAMP = 255` vs. 真实 tick 的 `tick % 256`，tick 0 时为 0），
/// 折进 `attempt` 后天然区分。跨 tick 不需要它兜底：不同 tick 的 `fseed`
/// 本就不同，`stamp` 循环撞车（如 tick 0 与 tick 256 同为 0）不构成风险
/// ——两次调用的 `fseed` 已经不同，`rng_u32` 整体输入早已分叉。
fn emit_attempt(stamp: u8, roll: u32) -> u32 {
    ((stamp as u32) << 1) | roll
}

/// `emit_jitter` 允许的最大 `jitter.0`（Q16.16 raw）。取值推导：设
/// `width = 2*jitter.0 + 1`，`emit_jitter` 内部把 `[0, width)` 的缩放结果
/// 转回 `i32`（最大值 `width - 1 = 2*jitter.0`）；要保证这一步不越过
/// `i32::MAX` 发生静默 wrapping，需要 `2*jitter.0 <= i32::MAX`，即
/// `jitter.0 <= (i32::MAX - 1) / 2`。`(1 << 30) - 1` 恰好满足（代入得
/// `width = i32::MAX`，缩放结果最大 `i32::MAX - 1`，安全落在正 `i32` 内）
/// ——约合 16384 格，远超任何合理法术/发射器配置，纯属越界防线
/// （Task 5 修复轮 1 Minor 1）。`sand-harness::scenario::resolve_op` 复用
/// 同一常量做加载期校验，避免两处各自定义、日后各改各的。
pub const MAX_EMIT_JITTER_RAW: i32 = (1 << 30) - 1;

/// 把 32-bit 随机数映射到 `[-jitter, +jitter]`（Fx raw 域闭区间），纯整数
/// 运算、无浮点、无运行时除法（唯一除法点在 `fixed.rs::from_ratio`，此处
/// 只用移位）；全部算术走 `wrapping_*`，与核心其余定点运算（`fixed.rs`）
/// 的约定一致——即便 [`MAX_EMIT_JITTER_RAW`] 的 `debug_assert` 已经在
/// debug/test 构建里挡住越界输入，release 构建仍要求这里的算术本身不会
/// 因溢出检查开关差异而分叉。
///
/// 映射算法：设 `width = 2*jitter.0 + 1`（Fx raw 单位，含两端点共 `width`
/// 个整数值）。`r` 均匀分布在 `[0, u32::MAX]`，右移 32 位重缩放
/// `(r as u64).wrapping_mul(width) 右移 32 位`把它等比例映射到整数区间
/// `[0, width)`（等价于除以 `2^32 / width` 但不做运行时除法）；再减去
/// `jitter.0` 平移居中，落入 `[-jitter.0, jitter.0]`：`r = 0` 时缩放结果为
/// 0，最终为 `-jitter.0`（下界可达）；`r = u32::MAX` 时缩放结果逼近但小于
/// `width`，即最大为 `2*jitter.0`，最终为 `+jitter.0`（上界可达）。
/// **残余非均匀性**：`2^32` 一般不能被 `width` 整除，`[0, width)` 里排在
/// 前面（`2^32 mod width`个）的整数值命中概率比其余的高出至多 1/2^32——
/// 量级完全淹没在抖动的游戏感官分辨率之下（远小于 1 个 raw 单位的期望
/// 偏差），不做额外校正。
/// `jitter.0 <= 0` 视为"无抖动"直接返回零（调用方按 harness 量化约定
/// 保证非负，这里兜底避免符号翻转出现反向区间）。
fn emit_jitter(r: u32, jitter: Fx) -> Fx {
    if jitter.0 <= 0 {
        return Fx::ZERO;
    }
    debug_assert!(
        jitter.0 <= MAX_EMIT_JITTER_RAW,
        "Op::Emit jitter（raw={}）超出 MAX_EMIT_JITTER_RAW（{MAX_EMIT_JITTER_RAW}），\
         emit_jitter 的定点重缩放会溢出 i32 静默 wrapping",
        jitter.0
    );
    let width = (jitter.0 as i64).wrapping_mul(2).wrapping_add(1);
    let scaled = (r as u64).wrapping_mul(width as u64) >> 32;
    Fx((scaled as i32).wrapping_sub(jitter.0))
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
    /// 操作（Brush/Fill）盖戳，`Op::Emit` 里另外折进抖动 `attempt`
    /// （见 [`emit_attempt`]）；`fseed` 供 `Op::Emit` 的抖动掷骰（`rng::
    /// frame_seed(world.seed, tick)`，调用方与网格四相同一口径）；`op_idx`
    /// 是本 `op` 在调用方本 tick `ops` 切片里的下标（`enumerate()` 天然
    /// 定序），折进抖动 `salt`（见 [`emit_salt`]），区分同 tick 内多个
    /// `Op::Emit`；`spawns` 是本 tick 粒子生成队列的输出参数——`Op::Emit`
    /// 把产出的 [`SpawnRequest`] 追加进去，调用方（`Sim::step`/
    /// `Sim::apply_setup`）负责随后把它们并入 `Particles` 的入队序（Emit
    /// 本身不直接碰 `Particles`，保持 world.rs 不知道粒子池存在——白名单
    /// 通信介质走队列，而非类型耦合）。
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
                // 抖动流的格坐标锚点用发射点本身的格（不逐粒子变化——同一次
                // Emit 的全部粒子共享 (x,y)）。salt 折进 (op_idx, i) 两维：
                // op_idx 区分"本 tick 内哪个 Op::Emit"（I1 修复：仅用
                // salt = i 会让同 tick 命中同一发射格的两个 Emit 撞出位级
                // 相同的抖动序列），i 区分"同一个 Emit 内第几个粒子"。
                // attempt 折进 (stamp, roll) 两维：roll 区分 vx/vy 两骰，
                // stamp 额外区分 setup 阶段与 tick 0 首个 step()（两者共享
                // 同一 fseed，见 emit_attempt 文档）。
                let gx = x.to_cell();
                let gy = y.to_cell();
                for i in 0..count as u32 {
                    let salt = emit_salt(op_idx, i);
                    let rx = rng::rng_u32(fseed, rng::STREAM_EMIT, gx, gy, salt, emit_attempt(stamp, EMIT_ROLL_VX));
                    let ry = rng::rng_u32(fseed, rng::STREAM_EMIT, gx, gy, salt, emit_attempt(stamp, EMIT_ROLL_VY));
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
        w.apply_op(&t, &op, 0, fseed, 0, &mut spawns);
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
        w.apply_op(&t, &op, 0, fseed, 0, &mut spawns);
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
        w.apply_op(&t, &op, 0, fseed, 0, &mut a);
        w.apply_op(&t, &op, 0, fseed, 0, &mut b);
        let av: Vec<(i32, i32)> = a.iter().map(|s| (s.vx.0, s.vy.0)).collect();
        let bv: Vec<(i32, i32)> = b.iter().map(|s| (s.vx.0, s.vy.0)).collect();
        assert_eq!(av, bv, "同一 fseed 重复应用 Emit 必须给出逐粒子完全相同的抖动序列");
    }

    // ==================== 修复轮 1 I1：同帧同格多 Emit 撞 key ====================

    /// 同一 tick 内两个 `Op::Emit` 命中同一发射格：op_idx 不同（0 vs 1），
    /// 抖动序列必须整体不同——修复前（salt 只含粒子序号 i）这里会逐位相同。
    #[test]
    fn emit_op_idx_differentiates_same_tick_same_cell_emits() {
        let t = test_table();
        let mut w = World::new(2, 2, 0xC0FFEE);
        let fseed = rng::frame_seed(w.seed, w.tick);
        let op = Op::Emit {
            material: 2,
            x: Fx::from_int(20),
            y: Fx::from_int(20),
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            count: 6,
            jitter: Fx::from_int(3),
        };
        let mut a = Vec::new();
        let mut b = Vec::new();
        // 同一 op 值、同一 fseed，唯一差异是 op_idx（模拟同 tick ops 切片
        // 里的第 0 与第 1 条）。
        w.apply_op(&t, &op, 0, fseed, 0, &mut a);
        w.apply_op(&t, &op, 0, fseed, 1, &mut b);
        let av: Vec<(i32, i32)> = a.iter().map(|s| (s.vx.0, s.vy.0)).collect();
        let bv: Vec<(i32, i32)> = b.iter().map(|s| (s.vx.0, s.vy.0)).collect();
        assert_ne!(
            av, bv,
            "op_idx 不同时，同发射格的两个 Emit 抖动序列必须不同（I1 回归）"
        );
    }

    /// `emit_salt` 白盒：不同 `op_idx` 折出不同 salt（即便 `i` 相同）。
    #[test]
    fn emit_salt_differs_across_op_idx_for_same_particle_index() {
        assert_ne!(emit_salt(0, 3), emit_salt(1, 3));
        assert_ne!(emit_salt(0, 0), emit_salt(1, 0));
    }

    /// I1 括注场景：`Sim::apply_setup`（`stamp = SETUP_STAMP = 255`）与
    /// tick 0 首个 `step()`（`stamp = 0`）共享同一 fseed；`emit_attempt`
    /// 把 `stamp` 折进去后，即便 op_idx/salt 完全相同，两个相位的抖动
    /// 序列也必须不同。
    #[test]
    fn emit_attempt_differentiates_setup_phase_from_tick_zero_step() {
        let t = test_table();
        let mut w = World::new(1, 1, 7);
        let fseed = rng::frame_seed(w.seed, w.tick); // world.tick == 0，setup 期同款计算
        let op = Op::Emit {
            material: 2,
            x: Fx::from_int(8),
            y: Fx::from_int(8),
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            count: 3,
            jitter: Fx::from_int(2),
        };
        const SETUP_STAMP: u8 = 255;
        let mut setup_spawns = Vec::new();
        let mut tick0_spawns = Vec::new();
        w.apply_op(&t, &op, SETUP_STAMP, fseed, 0, &mut setup_spawns);
        w.apply_op(&t, &op, 0, fseed, 0, &mut tick0_spawns);
        let sv: Vec<(i32, i32)> = setup_spawns.iter().map(|s| (s.vx.0, s.vy.0)).collect();
        let tv: Vec<(i32, i32)> = tick0_spawns.iter().map(|s| (s.vx.0, s.vy.0)).collect();
        assert_ne!(
            sv, tv,
            "setup 阶段与 tick 0 首个 step() 共享 fseed，stamp 必须区分出不同抖动序列"
        );
    }

    // ==================== 修复轮 1 Minor 1：jitter 上界防护 ====================

    #[test]
    fn emit_jitter_at_max_bound_does_not_panic() {
        // 恰在边界：width = i32::MAX，缩放结果最大 i32::MAX - 1，安全。
        let jitter = Fx(MAX_EMIT_JITTER_RAW);
        let lo = emit_jitter(0, jitter);
        let hi = emit_jitter(u32::MAX, jitter);
        assert_eq!(lo, -jitter);
        assert_eq!(hi, jitter);
    }

    #[test]
    #[should_panic(expected = "超出 MAX_EMIT_JITTER_RAW")]
    fn emit_jitter_above_max_bound_panics_in_debug() {
        let _ = emit_jitter(0, Fx(MAX_EMIT_JITTER_RAW + 1));
    }
}
