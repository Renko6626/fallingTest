//! 世界 = W×H chunk 的 flat Vec + 行主序静态页表（charter §9 缝 1）。
//! 越界读返回 WALL 哨兵；brush/fill 与场景 setup 共用同一写入路径（spec §5.2）。

use crate::cell::Cell;
use crate::chunk::{Chunk, CHUNK, DirtyRect};
use crate::dda::CellWalk;
use crate::fixed::{isqrt, Fx};
use crate::material::{MaterialTable, Category, MAT_AIR, MAT_WALL};
use crate::particle::{clamp_speed, MAX_SPEED};
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
    /// 爆炸（spec §6，Noita 射线模型）：以 `(x, y)` 为圆心、半径 `r` 格的
    /// Bresenham 圆周每格发一条 DDA 射线，射线初始能量 `power`，逐格消耗
    /// `MaterialTable::blast_cost`，能量 ≥ 格消耗即摧毁该格（置 air + 溅射
    /// 粒子，或按 `MaterialTable::vaporize_threshold` 判定汽化——删除、不
    /// 溅射，用户裁决 2026-08-30，见 `fire_ray` 文档），能量耗尽或撞
    /// `BLAST_COST_INFINITE` 材料（M1 里即 wall）断线。
    /// 整数签名——圆心/半径是格坐标，不经过 `Fx` 量化（与 `Op::Emit` 的
    /// 连续坐标不同，爆炸是格对齐的离散几何）。
    Explode { x: i32, y: i32, r: i32, power: u32 },
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

// ==================== Op::Explode（spec §6，M1 Task 6）====================

/// `Op::Explode` 里 vx 抖动用的骰子标号，语义与 `EMIT_ROLL_VX` 相同——两者
/// 数值巧合相等（都是 0/1）不代表可以共用常量：`Op::Emit`/`Op::Explode` 是
/// 两个不同的调用点，各自的 `attempt` 编码独立演化，未来任一方改动骰子数量
/// 不该牵连另一方。
const EXPLODE_ROLL_VX: u32 = 0;
/// `Op::Explode` 里 vy 抖动用的骰子标号（同上，见 [`EXPLODE_ROLL_VX`]）。
const EXPLODE_ROLL_VY: u32 = 1;

/// 溅射速度抖动幅度（spec §6 point 3"调参项"）：`Fx::from_ratio(1, 2)`
/// 的位模式（写成字面量是因为 `from_ratio` 非 `const fn`，理由同
/// `particle.rs::GRAVITY`；`explode_jitter_matches_from_ratio_one_half`
/// 单测钉死等价性）。复用 [`emit_jitter`] 做区间映射——同一套抖动数学，
/// 只是幅度常量与调用点（stream/salt/attempt）不同。
const EXPLODE_JITTER: Fx = Fx(0x0000_8000);

/// 半格偏移（Q16.16 的 0.5）：格坐标 → 格心连续坐标，供爆炸射线的起点/
/// 落点定位（cell_walk 的 DDA 几何要求连续坐标，格心比格角更安全——见
/// `dda.rs` 顶部注释关于恰好贴边界时 `rem=0` 的讨论）。
const HALF_CELL: Fx = Fx(0x0000_8000);

/// Bresenham 圆周（半径 `r` 格，圆心偏移量）：返回圆心到每个周长格的整数
/// 偏移 `(dx, dy)`，**定序、无重复**（spec §6 point 1 + §10 单测
/// `explode_circle_offsets_is_stable_and_has_no_duplicates`）。
///
/// 算法：经典 Bresenham/midpoint 圆算法（决策变量 `d = 3 - 2r`，八分圆
/// `0 <= x <= y` 递推），每步产出的八分圆点 `(a, b)` 按固定顺序展开成全圆的
/// 8 个镜像点：
/// `[(a,b), (b,a), (-b,a), (-a,b), (-a,-b), (-b,-a), (b,-a), (a,-b)]`。
///
/// **确定性论证**：递推变量 `x/y/d` 全整数、无浮点、无随机源，同一 `r` 每次
/// 调用产出完全相同的序列（纯算术，不依赖遍历顺序以外的任何状态）。
///
/// **无重复论证**：八分圆递推里 `y` 严格单调递减、`x` 单调不减（标准
/// Bresenham 圆性质），故每步的 `(a, b)` 互不相同；对不同的 `(a,b)`，其 8
/// 个镜像点集合两两不相交——除非 `(a1,b1)` 与 `(a2,b2)` 互为交换
/// （`a1=b2 且 b1=a2`），但循环全程维持 `a<=b`，只有 `a=b`（对角线）才可能
/// 自交换，那正是同一步内部的退化情形。步内去重（[`push_octant_mirror`]）
/// 用长度 ≤8 的线性 `contains` 扫描——退化情形（`a=0` 轴上或 `a=b` 对角线）
/// 产出的重复索引在候选数组里不保证相邻（`a=0` 时重复出现在 idx0/idx3 而非
/// 相邻位置，手工验证过），故不能只比较"相邻 + 首尾折返"，必须是全量成员
/// 检测；`Vec` 而非 `HashSet` 承载（候选数不超过 8，线性扫描足够快，也不
/// 触碰"禁 std HashSet 默认 hasher"红线）。
///
/// `r <= 0` 特例：圆退化为圆心一点，返回 `vec![(0, 0)]`（半径为 0 的"爆炸"
/// 只处理爆心格自身，见 [`fire_ray`] 起点格计费口径）。
pub(crate) fn circle_offsets(r: i32) -> Vec<(i32, i32)> {
    if r <= 0 {
        return vec![(0, 0)];
    }
    let mut offsets = Vec::new();
    let mut x: i32 = 0;
    let mut y: i32 = r;
    let mut d: i32 = 3 - 2 * r;
    while y >= x {
        push_octant_mirror(&mut offsets, x, y);
        x += 1;
        if d > 0 {
            y -= 1;
            d += 4 * (x - y) + 10;
        } else {
            d += 4 * x + 6;
        }
    }
    offsets
}

/// 单个八分圆点 `(a, b)` 展开为全圆最多 8 个镜像点，去重（退化情形：
/// `a == 0` 或 `a == b` 时实际只有 4 个不同点，且重复项在候选数组里未必
/// 相邻——`a == 0` 时是 idx0 与 idx3 重复，见 [`circle_offsets`] 文档的
/// 手工验证，故用全量 `contains` 而非相邻比较）。展开顺序固定（先出现者
/// 保留），是圆周格的最终遍历序。
fn push_octant_mirror(offsets: &mut Vec<(i32, i32)>, a: i32, b: i32) {
    let candidates: [(i32, i32); 8] =
        [(a, b), (b, a), (-b, a), (-a, b), (-a, -b), (-b, -a), (b, -a), (a, -b)];
    let mut group: Vec<(i32, i32)> = Vec::with_capacity(8);
    for &p in &candidates {
        if !group.contains(&p) {
            group.push(p);
        }
    }
    offsets.extend(group);
}

/// 单条爆炸射线（spec §6 point 2/3）：从圆心 `(cx, cy)` 出发，沿方向
/// `(dx, dy)` 逐格消耗能量，能量 ≥ 格消耗即摧毁（置 air + 溅射粒子，或按
/// `MaterialTable::vaporize_threshold` 判定汽化——置 air、不溅射，见下方
/// "近心汽化"节），能量不足或撞 `BLAST_COST_INFINITE` 材料即断线。
///
/// **爆心格自身的口径**（任务书 + spec §6 point 1"起点格按第一格计费处理"）：
/// `(cx, cy)` 本身作为该射线的第一格纳入能量结算——与 [`CellWalk`]"不含
/// 起点格"的粒子 DDA 语义相反，这里手动 `std::iter::once((cx, cy))` 前置，
/// 再接 `CellWalk` 产出的后续格。`r>=1` 时每条射线都独立从爆心起算，第一条
/// 处理到的射线摧毁爆心（若能量足够）；后续射线到达时爆心已是 air，
/// `blast_cost(air)=0` 恒满足、且已 air 不重复摧毁/不重复溅射（spec §6
/// point 4），对能量预算而言等同免费经过。
///
/// **能量衰减用于溅射速度**：摧毁某格后取*该格消耗完成后的剩余能量*
/// （`remaining = energy_before - cost`）参与速度合成——"剩余能量/power"
/// 随射线深入单调递减，天然实现"爆心附近溅得快、边缘溅得慢"的线性衰减
/// （spec §6 point 3）。
///
/// **近心汽化**（`vaporize_threshold`，spec §6 汽化小节，用户裁决
/// 2026-08-30）：同一个 `remaining`（即上一段的剩余能量，两处共享同一变量、
/// 不做区分）若使比例 `remaining/power` 严格超过材质阈值，该格直接删除、
/// **不**产出 `SpawnRequest`——做出"近心没了、外圈飞溅"的观感，质量在此
/// 确定性蒸发（`World::vaporized_total` 计数，不入哈希）。
///
/// `salt = op_idx`（充分性论证见 `rng.rs::STREAM_EXPLODE` 文档：坐标本身
/// `(gx, gy)` 已经是"一次 Explode 应用内至多摧毁一次"的天然唯一键，
/// `op_idx` 只需区分同 tick 内的不同 `Op::Explode`）。
#[allow(clippy::too_many_arguments)]
fn fire_ray(
    world: &mut World,
    table: &MaterialTable,
    cx: i32,
    cy: i32,
    dx: i32,
    dy: i32,
    power: u32,
    stamp: u8,
    fseed: u32,
    op_idx: usize,
    spawns: &mut Vec<SpawnRequest>,
) {
    debug_assert!(power != 0, "fire_ray 要求 power != 0（调用方 apply_op 已在 Explode 分支判零）");
    let mag_sq = (dx as i64) * (dx as i64) + (dy as i64) * (dy as i64);
    let mag = isqrt(mag_sq as u64) as i32;
    let (unit_dx, unit_dy) =
        if mag == 0 { (Fx::ZERO, Fx::ZERO) } else { (Fx::from_ratio(dx, mag), Fx::from_ratio(dy, mag)) };

    let center = (Fx::from_int(cx) + HALF_CELL, Fx::from_int(cy) + HALF_CELL);
    let ray_cells = std::iter::once((cx, cy))
        .chain(CellWalk::new(center, (Fx::from_int(dx), Fx::from_int(dy))));

    let mut energy = power;
    let salt = op_idx as u32;
    for (gx, gy) in ray_cells {
        if !world.in_bounds(gx, gy) {
            break; // 出界断线，同"撞不可摧毁材料"一样直接停止该射线。
        }
        let material = world.cell(gx, gy).material();
        let cost = table.blast_cost(material);
        if energy < cost {
            break; // 能量不足以摧毁这一格（含 BLAST_COST_INFINITE 撞线）。
        }
        energy -= cost;
        if material == MAT_AIR {
            continue; // 已是 air（原生或已被前序射线炸掉）：计零费、不重复溅射。
        }

        // 近心汽化（vaporize_threshold，spec §6 汽化小节，用户裁决
        // 2026-08-30）：`energy` 此刻已完成 `cost` 扣减——这正是下面
        // `speed_ratio` 用的同一个"剩余能量"值，口径钉死不做区分（不存在
        // "扣费前"的候选口径：`fire_ray_vaporize_*` 单测锁定这一点）。比例
        // `energy/power` 一旦**严格超过**材质阈值 `threshold/255`，格子直接
        // 删除、不产出 `SpawnRequest`（质量确定性蒸发，`vaporized_total`
        // 计数，不入哈希）——纯整数比较避免除法：
        // `energy/power > threshold/255` 等价于 `energy*255 > power*threshold`
        // （`power != 0` 已由函数入口 `debug_assert` 保证，两侧同乘不改变
        // 不等号方向；两边最大约 `u32::MAX * 255 ≈ 1.1e12`，`i64` 内不会溢出）。
        // 严格大于是关键：`threshold=255`（RON 缺省 1.0）时条件退化为
        // `energy > power`，而 `energy <= power` 恒成立（`cost` 是无符号扣减），
        // 故缺省材质永不汽化，即便 `energy == power`（`cost == 0`）也不触发。
        let threshold = table.vaporize_threshold(material);
        if (energy as i64) * 255 > (power as i64) * (threshold as i64) {
            world.set_cell_stamped(table, gx, gy, MAT_AIR, stamp);
            world.vaporized_total += 1;
            continue; // 汽化：不生成粒子，跳过下面的速度合成与 spawn。
        }

        let speed_ratio = Fx::from_ratio(energy as i32, power as i32);
        let speed_mag = MAX_SPEED.mul(speed_ratio);
        let rx = rng::rng_u32(fseed, rng::STREAM_EXPLODE, gx, gy, salt, emit_attempt(stamp, EXPLODE_ROLL_VX));
        let ry = rng::rng_u32(fseed, rng::STREAM_EXPLODE, gx, gy, salt, emit_attempt(stamp, EXPLODE_ROLL_VY));
        let vx = clamp_speed(unit_dx.mul(speed_mag) + emit_jitter(rx, EXPLODE_JITTER));
        let vy = clamp_speed(unit_dy.mul(speed_mag) + emit_jitter(ry, EXPLODE_JITTER));

        world.set_cell_stamped(table, gx, gy, MAT_AIR, stamp);
        spawns.push(SpawnRequest {
            material,
            x: Fx::from_int(gx) + HALF_CELL,
            y: Fx::from_int(gy) + HALF_CELL,
            vx,
            vy,
        });
    }
}

pub struct World {
    pub width_chunks: usize,
    pub height_chunks: usize,
    pub chunks: Vec<Chunk>,
    pub tick: u64,
    pub seed: u64,
    /// 爆炸近心汽化诊断计数（vaporize_threshold，用户裁决 2026-08-30）：
    /// `fire_ray` 判定某格汽化（删除、不入生成队列）时 +1。纯诊断计数器，
    /// 仿 `Particles::rejected_total`/`buried_total` 先例——**不参与**
    /// `hash::state_hash`（该函数只读 `tick` + `chunks`，见 hash.rs:23-33），
    /// 不影响 SyncTest 哈希比对，但本身仍是（状态,输入）的确定性函数，可供
    /// 测试直接断言。
    vaporized_total: u64,
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
    /// 操作（Brush/Fill/Explode 摧毁）盖戳，`Op::Emit`/`Op::Explode` 里
    /// 另外折进抖动 `attempt`（见 [`emit_attempt`]）；`fseed` 供两者的抖动
    /// 掷骰（`rng::frame_seed(world.seed, tick)`，调用方与网格四相同一
    /// 口径）；`op_idx` 是本 `op` 在调用方本 tick `ops` 切片里的下标
    /// （`enumerate()` 天然定序），折进抖动 `salt`（`Op::Emit` 见
    /// [`emit_salt`]，`Op::Explode` 见 [`fire_ray`] 文档），区分同 tick 内
    /// 多个同类型 `Op`；`spawns` 是本 tick 粒子生成队列的输出参数——
    /// `Op::Emit`/`Op::Explode` 把产出的 [`SpawnRequest`] 追加进去，调用方
    /// （`Sim::step`/`Sim::apply_setup`）负责随后把它们并入 `Particles` 的
    /// 入队序（world.rs 本身不直接碰 `Particles`，保持不知道粒子池存在——
    /// 白名单通信介质走队列，而非类型耦合）。
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
            Op::Explode { x, y, r, power } => {
                // power == 0：没有能量可摧毁任何格（哪怕 blast_cost=0 的
                // air，"摧毁"逻辑本身不会为 air 触发），且 `fire_ray` 的
                // `Fx::from_ratio(energy, power)` 除数不能为零——提前判零，
                // 语义上等价于"零能量爆炸 = 无操作"，不依赖 fire_ray 内部
                // 的分支顺序侥幸绕开除零。
                if power != 0 {
                    // 圆周格定序遍历（see circle_offsets 文档的确定性/无
                    // 重复论证），每格一条独立射线，salt = op_idx（见
                    // fire_ray 文档：坐标本身已是天然唯一键，op_idx 只需
                    // 区分同 tick 内不同 Op::Explode，charter §11 翻案 4 +
                    // Task 5 I1 同款纪律）。
                    //
                    // 质量守恒缺口（终审观察，非 bug）：`fire_ray` 对每个命中
                    // 格先 `set_cell_stamped(.., MAT_AIR, ..)` 清格，再把同一
                    // 份质量以 `SpawnRequest` 追加进 `spawns`；`spawns` 之后
                    // 由调用方（`Sim::step`/`apply_setup`）drain 进
                    // `Particles::spawn`。若彼时粒子池已在 `MAX_PARTICLES`
                    // 上限，`spawn` 会确定性拒绝——格子已经变 air，粒子却没能
                    // 生成，这份质量永久丢失（不返还、不回滚已清的格）。两端
                    // 状态一致（drain 序定序、拒绝条件是纯函数），不破坏
                    // 确定性，但需知悉：拒绝事件计入
                    // `Particles::rejected_total()`，可观测、可断言。
                    for (dx, dy) in circle_offsets(r) {
                        fire_ray(self, table, x, y, dx, dy, power, stamp, fseed, op_idx, spawns);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::{Category, MaterialDef};

    fn test_table() -> MaterialTable {
        // blast_cost 取 spec §6 的口径值（air 0 / water 1 / sand 2 / wall
        // 免疫），供本文件的 Op::Explode 测试直接复用；Emit 测试不关心该
        // 字段取值。
        use crate::material::BLAST_COST_INFINITE;
        let def = |id: u8, name: &str, category: Category, density: u16, blast_cost: u32| MaterialDef {
            id,
            name: name.into(),
            category,
            density,
            color: (0, 0, 0),
            blast_cost,
            // 255 = 永不汽化：本表供 blast_cost/断线/守恒等既有行为测试复用，
            // 不应引入意料之外的汽化分支——专门测汽化差异的用例另建材料表
            // （见下方"vaporize_threshold"分节）。
            vaporize_threshold: 255,
        };
        MaterialTable::new(vec![
            def(0, "air", Category::Static, 0, 0),
            def(1, "wall", Category::Static, 100, BLAST_COST_INFINITE),
            def(2, "water", Category::Liquid, 16, 1),
            def(3, "sand", Category::Powder, 40, 2),
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

    // ==================== circle_offsets：单测（M1 Task 6，spec §10）====================

    #[test]
    fn circle_offsets_r0_is_center_only() {
        assert_eq!(circle_offsets(0), vec![(0, 0)]);
        assert_eq!(circle_offsets(-3), vec![(0, 0)], "负半径按 0 处理");
    }

    #[test]
    fn circle_offsets_is_deterministic_across_repeated_calls() {
        for r in [1, 2, 3, 5, 8, 13, 20, 50] {
            assert_eq!(circle_offsets(r), circle_offsets(r), "r={r} 重复调用必须给出完全相同序列");
        }
    }

    #[test]
    fn circle_offsets_has_no_duplicate_cells() {
        for r in 1..=40 {
            let offs = circle_offsets(r);
            let mut sorted = offs.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), offs.len(), "r={r} 圆周格出现重复：{offs:?}");
        }
    }

    #[test]
    fn circle_offsets_every_point_within_one_cell_of_radius() {
        // Bresenham 圆是整数近似，允许 sqrt(dx²+dy²) 与 r 有 <1 格的栅格化
        // 误差，但不应离谱偏离（回归防线：算法写错通常表现为半径整体跑偏）。
        for r in [1, 5, 12, 30] {
            for (dx, dy) in circle_offsets(r) {
                let dist_sq = (dx as i64) * (dx as i64) + (dy as i64) * (dy as i64);
                let dist = isqrt(dist_sq as u64) as i64;
                assert!((dist - r as i64).abs() <= 1, "r={r} 的点 ({dx},{dy}) 距圆心 {dist}，偏离过大");
            }
        }
    }

    #[test]
    fn circle_offsets_r1_matches_four_axis_neighbors() {
        let mut offs = circle_offsets(1);
        offs.sort();
        let mut want = vec![(1, 0), (-1, 0), (0, 1), (0, -1)];
        want.sort();
        assert_eq!(offs, want);
    }

    // ==================== fire_ray：白盒（能量衰减 + 断线语义）====================

    #[test]
    fn fire_ray_speed_decays_monotonically_outward_along_axis() {
        // power=16、sand cost=2：每摧毁一格衰减 MAX_SPEED*2/16=2.0 格/tick
        // 的"真值"速度，远大于 EXPLODE_JITTER 的最大抖动摆幅（两次独立抖动
        // 最坏情况相差 1.0 格/tick），保证断言不受随机抖动噪声干扰。
        let t = test_table();
        let mut w = World::new(1, 1, 0xABC);
        let (cx, cy) = (10, 10);
        for dx in 1..=6 {
            w.set_cell_stamped(&t, cx + dx, cy, 3, 0); // sand
        }
        let fseed = rng::frame_seed(w.seed, w.tick);
        let mut spawns = Vec::new();
        fire_ray(&mut w, &t, cx, cy, 8, 0, 16, 0, fseed, 0, &mut spawns);

        assert_eq!(spawns.len(), 6, "6 个沙格应全部被摧毁：{spawns:?}");
        for i in 0..6i32 {
            assert_eq!(w.cell(cx + 1 + i, cy).material(), MAT_AIR, "沙格应变 air");
        }
        for pair in spawns.windows(2) {
            assert!(
                pair[0].vx.0 > pair[1].vx.0,
                "沿射线向外速度应单调衰减：{:?}",
                spawns.iter().map(|s| s.vx.0).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn fire_ray_stops_at_wall_shields_cells_behind_and_does_not_destroy_wall() {
        let t = test_table();
        let mut w = World::new(1, 1, 0);
        let (cx, cy) = (10, 10);
        w.set_cell_stamped(&t, cx + 3, cy, 1, 0); // wall
        w.set_cell_stamped(&t, cx + 4, cy, 3, 0); // sand behind wall
        let fseed = rng::frame_seed(w.seed, w.tick);
        let mut spawns = Vec::new();
        fire_ray(&mut w, &t, cx, cy, 8, 0, 1000, 0, fseed, 0, &mut spawns);

        assert_eq!(w.cell(cx + 3, cy).material(), 1, "wall 本身不可摧毁");
        assert_eq!(w.cell(cx + 4, cy).material(), 3, "wall 后方沙格应逐格完好（遮挡）");
        assert!(spawns.is_empty(), "撞墙前只有 air，撞墙即断线，不应产出任何溅射");
    }

    #[test]
    fn fire_ray_already_air_cells_cost_zero_and_do_not_respawn() {
        // 模拟"已被前序射线炸掉"：整条路径预先就是 air，射线应无阻碍走完
        // 全程且不产出任何 spawn（air 的 blast_cost=0，从不触发摧毁分支）。
        let t = test_table();
        let mut w = World::new(1, 1, 0);
        let fseed = rng::frame_seed(w.seed, w.tick);
        let mut spawns = Vec::new();
        fire_ray(&mut w, &t, 10, 10, 8, 0, 5, 0, fseed, 0, &mut spawns);
        assert!(spawns.is_empty());
    }

    // ==================== fire_ray：近心汽化（vaporize_threshold，用户裁决 2026-08-30）====================

    /// 两个材质专为边界测试构造：`blast_cost` 分别精确算到 power=255 时
    /// `remaining` 落在阈值 128 的正上方一格（129）与恰好持平（128）——
    /// 隔离出"严格大于"判定的两侧，不受材质其他属性干扰。
    fn vaporize_boundary_table() -> MaterialTable {
        use crate::material::BLAST_COST_INFINITE;
        let def = |id: u8, name: &str, category: Category, blast_cost: u32, vaporize_threshold: u8| MaterialDef {
            id,
            name: name.into(),
            category,
            density: 40,
            color: (0, 0, 0),
            blast_cost,
            vaporize_threshold,
        };
        MaterialTable::new(vec![
            def(0, "air", Category::Static, 0, 255),
            def(1, "wall", Category::Static, BLAST_COST_INFINITE, 255),
            // power=255 时 remaining=255-127=128，恰好等于阈值 128。
            def(2, "target_at_threshold", Category::Powder, 127, 128),
            // power=255 时 remaining=255-126=129，比阈值多 1。
            def(3, "target_above_threshold", Category::Powder, 126, 128),
        ])
        .unwrap()
    }

    #[test]
    fn fire_ray_vaporize_boundary_remaining_at_threshold_does_not_vaporize() {
        // remaining(128)*255 == power(255)*threshold(128)：判定是"严格大于"，
        // 持平不算，仍走正常摧毁 + 溅射路径。
        let t = vaporize_boundary_table();
        let mut w = World::new(1, 1, 0);
        let (cx, cy) = (10, 10);
        w.set_cell_stamped(&t, cx, cy, 2, 0);
        let fseed = rng::frame_seed(w.seed, w.tick);
        let mut spawns = Vec::new();
        fire_ray(&mut w, &t, cx, cy, 1, 0, 255, 0, fseed, 0, &mut spawns);
        assert_eq!(spawns.len(), 1, "阈值恰好持平，不应汽化");
        assert_eq!(w.vaporized_total(), 0);
        assert_eq!(w.cell(cx, cy).material(), MAT_AIR, "仍应正常摧毁为 air");
    }

    #[test]
    fn fire_ray_vaporize_boundary_remaining_just_above_threshold_vaporizes() {
        // remaining(129)*255 > power(255)*threshold(128)：越过阈值一格即汽化。
        let t = vaporize_boundary_table();
        let mut w = World::new(1, 1, 0);
        let (cx, cy) = (10, 10);
        w.set_cell_stamped(&t, cx, cy, 3, 0);
        let fseed = rng::frame_seed(w.seed, w.tick);
        let mut spawns = Vec::new();
        fire_ray(&mut w, &t, cx, cy, 1, 0, 255, 0, fseed, 0, &mut spawns);
        assert!(spawns.is_empty(), "越过阈值应汽化，不生成粒子");
        assert_eq!(w.vaporized_total(), 1);
        assert_eq!(w.cell(cx, cy).material(), MAT_AIR, "汽化仍需清空格子（质量蒸发）");
    }

    #[test]
    fn fire_ray_default_threshold_255_never_vaporizes_even_at_remaining_equals_power() {
        // RON 缺省 1.0 → 量化 255。blast_cost=0（非 air/wall 材质里的边界配置，
        // 纯为逼出 remaining==power 这个比例=1.0 的极端输入，现实材质不会
        // 这样配）依然不触发汽化——"严格大于"是关键：threshold=255 时条件
        // 退化为 `remaining > power`，而 `remaining <= power` 恒成立（cost
        // 是无符号扣减），故缺省材质在任何输入下都不汽化。
        use crate::material::BLAST_COST_INFINITE;
        let def = |id: u8, name: &str, category: Category, blast_cost: u32| MaterialDef {
            id,
            name: name.into(),
            category,
            density: 40,
            color: (0, 0, 0),
            blast_cost,
            vaporize_threshold: 255,
        };
        let t = MaterialTable::new(vec![
            def(0, "air", Category::Static, 0),
            def(1, "wall", Category::Static, BLAST_COST_INFINITE),
            def(2, "target_zero_cost", Category::Powder, 0),
        ])
        .unwrap();
        let mut w = World::new(1, 1, 0);
        let (cx, cy) = (10, 10);
        w.set_cell_stamped(&t, cx, cy, 2, 0);
        let fseed = rng::frame_seed(w.seed, w.tick);
        let mut spawns = Vec::new();
        fire_ray(&mut w, &t, cx, cy, 1, 0, 1000, 0, fseed, 0, &mut spawns);
        assert_eq!(spawns.len(), 1, "缺省阈值下 remaining==power 也不应汽化");
        assert_eq!(w.vaporized_total(), 0);
    }

    // ==================== Op::Explode：apply_op 行为测试（spec §10）====================

    fn explode_spawn_positions(spawns: &[SpawnRequest]) -> Vec<(i32, i32)> {
        let mut v: Vec<(i32, i32)> = spawns.iter().map(|s| (s.x.to_cell(), s.y.to_cell())).collect();
        v.sort();
        v
    }

    #[test]
    fn explode_center_cell_is_destroyed_when_r_at_least_one() {
        // 爆心口径钉死（任务书要求）：r>=1 时爆心格自身按第一格计费——只要
        // 爆心原本非 air 且能量足够，必被摧毁 + 溅射，不因"起点格豁免"跳过
        // （那是 DDA 粒子飞行语义，爆炸射线的口径明确相反，见 fire_ray 文档）。
        let t = test_table();
        let mut w = World::new(1, 1, 0);
        let (cx, cy) = (30, 30);
        w.set_cell_stamped(&t, cx, cy, 3, 0); // 爆心本身预置为沙
        let fseed = rng::frame_seed(w.seed, w.tick);
        let op = Op::Explode { x: cx, y: cy, r: 3, power: 50 };
        let mut spawns = Vec::new();
        w.apply_op(&t, &op, 0, fseed, 0, &mut spawns);

        assert_eq!(w.cell(cx, cy).material(), MAT_AIR, "爆心格必须被摧毁");
        assert!(
            explode_spawn_positions(&spawns).contains(&(cx, cy)),
            "爆心格必须产出溅射粒子：{:?}",
            explode_spawn_positions(&spawns)
        );
    }

    #[test]
    fn explode_thin_wall_shields_sand_behind_it() {
        // 行为测试（任务书）：薄墙遮挡——1 格 wall 后方的沙逐格完好。
        let t = test_table();
        let mut w = World::new(2, 2, 0);
        let (cx, cy) = (60, 60);
        w.set_cell_stamped(&t, cx + 4, cy, 1, 0); // 薄墙（1 格）
        for dx in 5..=9 {
            w.set_cell_stamped(&t, cx + dx, cy, 3, 0); // 墙后一整排沙
        }
        let fseed = rng::frame_seed(w.seed, w.tick);
        let op = Op::Explode { x: cx, y: cy, r: 12, power: 10_000 }; // 能量充裕，仍不能穿墙
        let mut spawns = Vec::new();
        w.apply_op(&t, &op, 0, fseed, 0, &mut spawns);

        assert_eq!(w.cell(cx + 4, cy).material(), 1, "wall 不可摧毁");
        for dx in 5..=9 {
            assert_eq!(w.cell(cx + dx, cy).material(), 3, "墙后第 {dx} 格沙应完好");
        }
    }

    #[test]
    fn explode_pit_conservation_destroyed_cells_equal_spawn_count() {
        // 挖坑守恒（任务书；口径随 vaporize_threshold 更新，用户裁决
        // 2026-08-30）：炸掉的格数 == 生成的溅射请求数 + 汽化计数
        // （汽化格既不生成粒子也不算"未处理"，质量确定性蒸发）。`test_table()`
        // 全部材质 `vaporize_threshold=255`（永不汽化），故本例
        // `vaporized_total` 恒为 0——断言写成通用形式，覆盖两条路径。
        let t = test_table();
        let mut w = World::new(2, 2, 0);
        let (cx, cy) = (60, 60);
        let r = 6;
        // 圆心为中心的一个方块实心沙，半径足够覆盖整个爆炸圆盘。
        for dy in -(r + 1)..=(r + 1) {
            for dx in -(r + 1)..=(r + 1) {
                w.set_cell_stamped(&t, cx + dx, cy + dy, 3, 0);
            }
        }
        let before = w.count_material(3);
        let fseed = rng::frame_seed(w.seed, w.tick);
        let op = Op::Explode { x: cx, y: cy, r, power: 1000 };
        let mut spawns = Vec::new();
        w.apply_op(&t, &op, 0, fseed, 0, &mut spawns);
        let after = w.count_material(3);

        let destroyed = before - after;
        assert!(destroyed > 0, "应至少摧毁一些沙格");
        assert_eq!(
            destroyed,
            spawns.len() + w.vaporized_total() as usize,
            "摧毁格数必须等于（生成的溅射请求数 + 汽化计数）"
        );
        // 每个 spawn 必须落在一个"曾是沙、现在是 air"的坐标上，且坐标互不重复
        // （每格至多被摧毁一次，spec §6 point 4）。
        let positions = explode_spawn_positions(&spawns);
        let mut dedup = positions.clone();
        dedup.dedup();
        assert_eq!(dedup.len(), positions.len(), "spawn 坐标不应重复：{positions:?}");
        for &(x, y) in &positions {
            assert_eq!(w.cell(x, y).material(), MAT_AIR, "spawn 坐标处网格应已是 air");
        }
    }

    // ==================== Op::Explode：材质差异化汽化（用户裁决 2026-08-30）====================

    /// 与 `data/materials.ron` 初值同口径：water 阈值 0.4（量化 102），
    /// sand 阈值 0.7（量化 179）。
    fn mixed_vaporize_table() -> MaterialTable {
        use crate::material::BLAST_COST_INFINITE;
        let def = |id: u8, name: &str, category: Category, density: u16, blast_cost: u32, vaporize_threshold: u8| {
            MaterialDef { id, name: name.into(), category, density, color: (0, 0, 0), blast_cost, vaporize_threshold }
        };
        MaterialTable::new(vec![
            def(0, "air", Category::Static, 0, 0, 255),
            def(1, "wall", Category::Static, 100, BLAST_COST_INFINITE, 255),
            def(2, "water", Category::Liquid, 16, 1, 102),
            def(3, "sand", Category::Powder, 40, 2, 179),
        ])
        .unwrap()
    }

    #[test]
    fn explode_mixed_water_sand_target_vaporizes_more_water_than_sand() {
        // 沙水混合目标（任务书要求）：同一次 Op::Explode 命中两种材质，water
        // 阈值（0.4）比 sand（0.7）低，预期 water 的汽化比例更高。几何：圆心
        // 左侧（dx<=0）填水、右侧（dx>0）填沙——每条射线沿固定方向走，dx 的
        // 符号沿途不变，故每条射线全程只穿一种材质，互不干扰。
        //
        // 参数口径（power=100）：water cost=1，remaining=100-d，
        // ratio>0.4 等价 d<60——目标区域内所有射线 d 远小于 60，water 应
        // 全汽化；sand cost=2，remaining=100-2d，ratio>0.702 等价 d<14.9，
        // 目标区域 d 覆盖到 ~26+，故 sand 应"近心汽化、远心正常溅射"两段
        // 都有——用整数交叉相乘比较汽化比例，不引入浮点。
        let t = mixed_vaporize_table();
        let mut w = World::new(2, 2, 0);
        let (cx, cy) = (60, 60);
        let r = 24;
        for dy in -(r + 2)..=(r + 2) {
            for dx in -(r + 2)..=(r + 2) {
                let mat = if dx <= 0 { 2 } else { 3 };
                w.set_cell_stamped(&t, cx + dx, cy + dy, mat, 0);
            }
        }
        let before_water = w.count_material(2);
        let before_sand = w.count_material(3);
        let fseed = rng::frame_seed(w.seed, w.tick);
        let op = Op::Explode { x: cx, y: cy, r, power: 100 };
        let mut spawns = Vec::new();
        w.apply_op(&t, &op, 0, fseed, 0, &mut spawns);
        let after_water = w.count_material(2);
        let after_sand = w.count_material(3);

        let destroyed_water = before_water - after_water;
        let destroyed_sand = before_sand - after_sand;
        let spawn_water = spawns.iter().filter(|s| s.material == 2).count();
        let spawn_sand = spawns.iter().filter(|s| s.material == 3).count();
        let vaporized_water = destroyed_water - spawn_water;
        let vaporized_sand = destroyed_sand - spawn_sand;

        assert!(destroyed_water > 0 && destroyed_sand > 0, "两种材质都应被摧毁一些");
        assert!(vaporized_water > 0, "water 阈值更低，近心应有汽化");
        assert!(spawn_sand > 0, "sand 阈值更高，远心应有未汽化、正常溅射的部分");
        // vaporized_water/destroyed_water > vaporized_sand/destroyed_sand
        // <=> 交叉相乘（两边都是非负整数，不改变不等号方向）。
        assert!(
            vaporized_water * destroyed_sand > vaporized_sand * destroyed_water,
            "water 汽化比例应高于 sand：water {vaporized_water}/{destroyed_water}，\
             sand {vaporized_sand}/{destroyed_sand}"
        );
    }

    #[test]
    fn explode_zero_power_is_a_no_op() {
        let t = test_table();
        let mut w = World::new(1, 1, 0);
        let (cx, cy) = (10, 10);
        w.set_cell_stamped(&t, cx, cy, 3, 0);
        let fseed = rng::frame_seed(w.seed, w.tick);
        let op = Op::Explode { x: cx, y: cy, r: 5, power: 0 };
        let mut spawns = Vec::new();
        w.apply_op(&t, &op, 0, fseed, 0, &mut spawns);
        assert_eq!(w.cell(cx, cy).material(), 3, "power=0 不应摧毁任何格子");
        assert!(spawns.is_empty());
    }

    #[test]
    fn explode_repeated_application_is_deterministic() {
        // 同 Op 重跑一致（任务书）：两个独立构造、内容完全相同的世界，喂同一
        // 个 Op::Explode（同 fseed/op_idx），必须产出逐位相同的溅射序列。
        let t = test_table();
        let (cx, cy) = (60, 60);
        let build = || {
            let mut w = World::new(2, 2, 0x77);
            for dy in -4..=4 {
                for dx in -4..=4 {
                    w.set_cell_stamped(&t, cx + dx, cy + dy, 3, 0);
                }
            }
            w
        };
        let mut wa = build();
        let mut wb = build();
        let fseed = rng::frame_seed(wa.seed, wa.tick);
        let op = Op::Explode { x: cx, y: cy, r: 4, power: 40 };

        let mut a = Vec::new();
        wa.apply_op(&t, &op, 0, fseed, 0, &mut a);
        let mut b = Vec::new();
        wb.apply_op(&t, &op, 0, fseed, 0, &mut b);

        let av: Vec<(i32, i32, i32, i32)> = a.iter().map(|s| (s.x.0, s.y.0, s.vx.0, s.vy.0)).collect();
        let bv: Vec<(i32, i32, i32, i32)> = b.iter().map(|s| (s.x.0, s.y.0, s.vx.0, s.vy.0)).collect();
        assert_eq!(av, bv, "同一 Op::Explode 重复应用必须给出逐位相同的溅射序列");
    }

    #[test]
    fn explode_same_tick_two_explodes_have_different_jitter_sequences() {
        // I1 同款测试（任务书）：同 tick 内两个参数完全相同、圆心重合的
        // Op::Explode（op_idx 不同），抖动序列必须不同——即便坐标键相同，
        // salt=op_idx 这一维必须生效（rng.rs::STREAM_EXPLODE 文档）。
        let t = test_table();
        let (cx, cy) = (60, 60);
        let build = || {
            let mut w = World::new(2, 2, 0xC0FFEE);
            for dy in -3..=3 {
                for dx in -3..=3 {
                    w.set_cell_stamped(&t, cx + dx, cy + dy, 3, 0);
                }
            }
            w
        };
        let mut wa = build();
        let mut wb = build();
        let fseed = rng::frame_seed(wa.seed, wa.tick);
        let op = Op::Explode { x: cx, y: cy, r: 3, power: 30 };

        let mut a = Vec::new();
        wa.apply_op(&t, &op, 0, fseed, 0, &mut a);
        let mut b = Vec::new();
        wb.apply_op(&t, &op, 0, fseed, 1, &mut b);

        let av: Vec<(i32, i32)> = a.iter().map(|s| (s.vx.0, s.vy.0)).collect();
        let bv: Vec<(i32, i32)> = b.iter().map(|s| (s.vx.0, s.vy.0)).collect();
        assert_ne!(av, bv, "op_idx 不同时，同参数同圆心的两个 Explode 抖动序列必须不同（I1 回归）");
    }

    #[test]
    fn explode_jitter_matches_from_ratio_one_half() {
        assert_eq!(EXPLODE_JITTER, Fx::from_ratio(1, 2));
    }
}
