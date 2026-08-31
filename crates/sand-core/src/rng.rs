//! Counter-based RNG（charter §4 随机性法典、spec §4.1）。
//! SquirrelNoise5 一比一移植自 `archive/prototype-python/core/rng.py`，
//! 金值由 Python 版交叉锚定（见测试）。
//!
//! key = (world_seed, tick) → frame_seed（每 tick 预计算），
//! 其余分量 (stream, x, y, salt, attempt) 素数折叠后单次 squirrel5。
//! **stream 显式编码调用点**（charter §11 翻案 4：同帧同格多次掷骰必须不同流），
//! 新调用点在下方常量区追加，禁止复用。

const N1: u32 = 0xD2A8_0A3F;
const N2: u32 = 0xA884_F197;
const N3: u32 = 0x6C73_6F4B;
const N4: u32 = 0xB79F_3ABB;
const N5: u32 = 0x1B56_C4F5;

const P_X: u32 = 0x9E37_79B1;
const P_Y: u32 = 0x85EB_CA77;
const P_STREAM: u32 = 0xC2B2_AE3D;
const P_SALT: u32 = 0x27D4_EB2F;
const P_ATTEMPT: u32 = 0x1656_67B1;

/// 调用点流注册表（对应 Python 版 pass_id 维度；spec §4.1）。
pub const STREAM_DIAG: u32 = 0;

/// `Op::Emit` 发射器速度抖动流（spec §7/§8，M1 Task 5）。调用点：
/// `world.rs::World::apply_op` 的 `Op::Emit` 分支——`salt` = 粒子序号 i，
/// `attempt` 挪用为"vx/vy 骰子标号"（0=vx、1=vy，见该分支文档注释），
/// 与其原始"重试次数"语义不同但同属"同 salt 下的独立第 N 骰"，满足
/// charter §11 翻案 4"同帧同格多骰不同参数"的纪律。
pub const STREAM_EMIT: u32 = 1;

/// `Op::Explode` 溅射速度抖动流（spec §6/§8，M1 Task 6）。调用点：
/// `world.rs::World::apply_op` 的 `Op::Explode` 分支——`(x, y)` 直接用
/// **被摧毁格自身的绝对坐标**（不像 `Op::Emit` 那样共享单一发射点，故不需要
/// 逐粒子的 `salt = i` 维度：一次 Explode 应用内，每个格子至多被摧毁一次，
/// 坐标天然唯一），`salt = op_idx`（区分同 tick 内多个 `Op::Explode`，charter
/// §11 翻案 4 + Task 5 评审 I1 同款教训——即便两个爆炸参数完全相同、圆心重合，
/// 也不能让同一格子在假设性的"重算"路径上撞出相同抖动，op_idx 维度是免费的
/// 防御），`attempt` 复用 `emit_attempt(stamp, roll)` 的编码（`stamp` 区分
/// setup 与 tick 0 首个 step 共享 fseed 的相位撞车，`roll` 区分 vx/vy 两骰，
/// 与 `Op::Emit` 同一套 `EXPLODE_ROLL_VX`/`EXPLODE_ROLL_VY` 常量）。
pub const STREAM_EXPLODE: u32 = 2;

/// 重力速度积分的子像素概率取整流（Layer G Task 2，spec §4.2③）。调用点：
/// `rules.rs::substeps` —— 每 tick 每 cell 掷一次，决定 `v` 的小数部分是否
/// 兑换成额外一个子步（`n = v/VEL_ONE + [roll % VEL_ONE < v % VEL_ONE]`）。
///
/// **key 取该 cell 本 tick 的起始坐标**，不掺 salt/attempt：扫描开始时每个
/// 网格位置至多一个 cell，故起始坐标在同一 tick 内天然唯一，不存在 charter
/// §11 翻案 4 点名的"同帧同格多次掷骰返回同值"隐患。（终点坐标则**不**唯一
/// ——高速 cell 腾空原格后上方 cell 可能同 tick 落入同格，Task 3 溅射骰的
/// key 选择记的是同一条教训。）
///
/// 与 [`STREAM_SCANDIR`] 不同，本流的 key 含 `x`/`y`，与 `STREAM_DIAG` 天然
/// 不同流；两者取的都是低位，但流号不同即 `pos` 不同，无需错开取位。
pub const STREAM_FALLSTEP: u32 = 3;

/// 行扫描方向流（charter §11 实施期决策第 3 条，2026-08-31）。调用点：
/// `rules.rs::update_chunk` 每 tick 掷一次，决定本 tick 的行方向全局奇偶相位。
///
/// **为什么不能沿用 `(y + tick) & 1`**：那个式子对**运动中**的粒子会自我抵消
/// 失效——自由下落者 `y+1`/`tick+1` 使 `(y+tick)` 奇偶恒定，整个下落被锁死在
/// 同一扫描方向，交替机制只对静止粒子生效。更一般地，**任何周期为 2 的定向
/// 方案都会与周期为 2 的动力学共振**（`tick & 1` 实测更差）。实测粉末堆积因此
/// 产生系统性右偏，详见 `docs/proposals/2026-08-31-powder-scan-direction-bias.md`。
///
/// **key 只取 `tick`（经 `fseed`），`x`/`y` 恒传 0** —— 每 tick 全局掷一次，
/// 行方向 = `(y ^ flip) & 1`。这样同时拿到三件事：① 无周期，粒子每 tick 跨
/// 任意 n 行都不会被锁死（Layer G Task 2 的速度积分会让 n ∈ 1..4，逐行哈希与
/// `y & 1` 在 n 的某种奇偶下都会重新锁死，本方案不会）；② **保留"同一 tick 内
/// 相邻行必然反向"**，这是一个免费的对偶变量方差缩减，也避免同向行成串带来的
/// 剪切带伪影；③ 同一行在所有 chunk 同向——方向若随 chunk 变，物理行为就会
/// 依赖 chunk 划分，在竖缝上引入新的各向异性。
///
/// **取 bit16 而非 bit0**：`rng_u32` 的 `pos` 是各分量的线性折叠、不是单射，
/// 存在唯一一列 `x*` 使 `pos(SCANDIR, ...) == pos(DIAG, x*, ...)`（`P_X` 为奇数
/// 可逆 ⇒ 解唯一）。当前 `x*` 远在任何世界宽度之外，但那是**凑巧安全而非构造
/// 安全**——与 `diag_side` 的 bit0 错开一位代价为零。执法测试见本文件
/// `tests::scandir_bit_independent_of_diag_bit`。
///
/// **编号 4 而非 3**：3 归 Layer G Task 2 的 [`STREAM_FALLSTEP`]（2026-08-31 已落实）
/// （`docs/superpowers/specs/2026-08-31-layer-g-velocity-design.md` §4.2③），
/// 5 预留给 Task 3 的 `STREAM_SPLASH`（同 spec §6.1）。此处一次性排好，避免
/// 后续撞号被迫改常量、再作废一次 golden。
pub const STREAM_SCANDIR: u32 = 4;

/// 撞击溅射流（Layer G Task 3，spec §6.1③/§6.3）。调用点：`rules.rs::try_splash`
/// —— 三颗骰共用本流，靠 `attempt` 区分（`SPLASH_ROLL_TRIGGER`/`_VX`/`_VY`），
/// 体例同 `explode.rs` 的 `EXPLODE_ROLL_*`。
///
/// **key 用该 cell 本 tick 的起始坐标 `(sx, sy)`，不是撞停坐标**——这是本流
/// 唯一需要小心的地方，也是 2026-08-31 评审专门修订过的一条。撞停坐标在同一
/// tick 内**不唯一**：cell A 撞停脱格后原格变 AIR（盖戳不阻止它被 `displace`
/// 当作目标），上方 cell B 同 tick 落入同格再撞停；若 key 取撞停坐标，A 与 B
/// 掷出同一个值 —— 同材质则"A 溅则 B 必溅"，整列连锁全脱或全停，正是 charter
/// §11 翻案 4 点名的"同帧同格多骰同值"偏置。起始坐标每 tick 每 cell 唯一
/// （与 [`STREAM_FALLSTEP`] 同一条论证），撞停位置只决定粒子出生点、不进 key。
///
/// 不需要 `salt`：每 cell 每 tick 至多触发一次溅射，起始坐标已经唯一。
pub const STREAM_SPLASH: u32 = 5;

/// 本 tick 的行方向全局相位（0 或 1）；行方向 = `(y ^ scan_flip(fseed)) & 1 == 0`
/// 为左→右。见 [`STREAM_SCANDIR`] 的完整论证。
///
/// **红线（LiveRect 逐位等价的承重条件）**：行方向必须是 `(tick, y)` 的纯函数。
/// **禁止**让它读 `WriteWindow::live_rect`、扫描起始矩形、`own_ci`/chunk 坐标、
/// `dirty`/`next_dirty` 或任何线程上下文——O1 活矩形的等价性论证
/// （`docs/superpowers/specs/2026-08-30-o1-live-rect-design.md` §1）要求全扫访问序 V
/// 在三种 ScanMode 下完全一致，而 V 的行内定向正由此给出。违反即三模式分叉。
pub fn scan_flip(fseed: u32) -> u64 {
    ((rng_u32(fseed, STREAM_SCANDIR, 0, 0, 0, 0) >> 16) & 1) as u64
}

pub fn squirrel5(pos: u32, seed: u32) -> u32 {
    let mut m = pos.wrapping_mul(N1);
    m = m.wrapping_add(seed);
    m ^= m >> 9;
    m = m.wrapping_add(N2);
    m ^= m >> 11;
    m = m.wrapping_mul(N3);
    m ^= m >> 13;
    m = m.wrapping_add(N4);
    m ^= m >> 15;
    m = m.wrapping_mul(N5);
    m ^= m >> 17;
    m
}

pub fn frame_seed(world_seed: u64, tick: u64) -> u32 {
    squirrel5(tick as u32, world_seed as u32)
}

pub fn rng_u32(fseed: u32, stream: u32, x: i32, y: i32, salt: u32, attempt: u32) -> u32 {
    let pos = (x as u32)
        .wrapping_mul(P_X)
        .wrapping_add((y as u32).wrapping_mul(P_Y))
        .wrapping_add(stream.wrapping_mul(P_STREAM))
        .wrapping_add(salt.wrapping_mul(P_SALT))
        .wrapping_add(attempt.wrapping_mul(P_ATTEMPT));
    squirrel5(pos, fseed)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 金值来自 archive/prototype-python/core/rng.py 同参实跑（交叉锚定，见 spec §4.1）。
    #[test]
    fn golden_cross_anchored_with_python() {
        assert_eq!(squirrel5(0, 0), 0x1679_1E00);
        assert_eq!(squirrel5(12345, 67890), 0x3D9B_AAAB);
        assert_eq!(frame_seed(0xDEAD_BEEF_CAFE, 424_242), 0xBE6B_3BE3);
        assert_eq!(rng_u32(0x1234_5678, 3, -7, 63, 4, 2), 0xE9A4_F78B);
        assert_eq!(rng_u32(1, 0, 0, 0, 0, 0), 0x23F6_C851);
    }

    #[test]
    fn streams_and_attempts_are_independent() {
        let f = frame_seed(42, 100);
        let a = rng_u32(f, 0, 5, 5, 0, 0);
        assert_ne!(a, rng_u32(f, 1, 5, 5, 0, 0), "不同 stream 必须不同值");
        assert_ne!(a, rng_u32(f, 0, 5, 5, 0, 1), "不同 attempt 必须不同值");
        assert_ne!(a, rng_u32(f, 0, 5, 5, 1, 0), "不同 salt 必须不同值");
    }

    #[test]
    fn pure_function_repeats() {
        let f = frame_seed(7, 9);
        assert_eq!(rng_u32(f, 0, 1, 2, 3, 4), rng_u32(f, 0, 1, 2, 3, 4));
    }

    /// [`STREAM_SCANDIR`] 文档里那条"取 bit16 而非 bit0"的执法测试。
    ///
    /// **风险模型**：`rng_u32` 的 `pos` 是各分量的线性折叠、不是单射。存在一条
    /// 由 `x·P_X + y·P_Y ≡ STREAM_SCANDIR·P_STREAM` 定义的格子集合，其
    /// `rng_u32(DIAG, x, y)` 与 `rng_u32(SCANDIR, 0, 0)` **返回完全相同的 u32**。
    /// 若行方向与斜向偏好共用同一比特，这些格子的"往哪边滑"就与"本 tick 行方向"
    /// 永久完全相关 —— 系统性伪影。错开取位后，即便 `pos` 撞上，用的也是同一个
    /// u32 的不同比特，只要 squirrel5 的比特间无相关即安全。
    ///
    /// 故本测试检验的正是那个承重前提：**squirrel5 输出的 bit0 与 bit16 不相关**。
    ///
    /// （注意撞车线上所有 `y` 共享同一个 `pos`、因而共享同一个输出值，所以不能
    /// 按"每格一个独立样本"计数——那样会把有效样本数高估几十倍。这里直接对
    /// 大量互不相同的输出取样。）
    #[test]
    fn scandir_bit_independent_of_diag_bit() {
        let (mut same, mut n) = (0u32, 0u32);
        for tick in 0..4096u64 {
            for seed_i in 0..16u32 {
                let v = squirrel5(tick as u32, 0xC0FF_EE00u32.wrapping_add(seed_i));
                // scan 用 bit16（见 scan_flip），diag 用 bit0（见 rules::diag_side）
                if ((v >> 16) & 1) == (v & 1) {
                    same += 1;
                }
                n += 1;
            }
        }
        let p = same as f64 / n as f64;
        // n = 65536 ⇒ σ = 0.5/√n ≈ 0.195%，取 4σ ≈ 0.78% 作判据（留余量防偶发红）
        assert!(
            (p - 0.5).abs() < 0.0078,
            "squirrel5 的 bit0 与 bit16 符合率 {p:.5} 偏离 0.5 超过 4σ（n={n}）——\
             scan_flip 与 diag_side 的取位错开失去意义，撞车线上会出现系统性伪影"
        );
    }

    /// 撞车线确实存在（上面那条测试的风险模型不是臆想出来的）。
    #[test]
    fn scandir_and_diag_pos_collision_line_exists() {
        let inv_px = {
            let mut inv = P_X;
            for _ in 0..5 {
                inv = inv.wrapping_mul(2u32.wrapping_sub(P_X.wrapping_mul(inv)));
            }
            inv
        };
        assert_eq!(P_X.wrapping_mul(inv_px), 1);
        let fseed = frame_seed(0xC0FF_EE00, 7);
        let y = 13i32;
        let target = STREAM_SCANDIR
            .wrapping_mul(P_STREAM)
            .wrapping_sub((y as u32).wrapping_mul(P_Y));
        let x = target.wrapping_mul(inv_px);
        assert_eq!(
            rng_u32(fseed, STREAM_SCANDIR, 0, 0, 0, 0),
            rng_u32(fseed, STREAM_DIAG, x as i32, y, 0, 0),
            "撞车线应当存在——它是 scandir 取 bit16 的全部理由"
        );
        // 但它落在任何合理世界宽度之外，故只是"凑巧安全"，不能依赖
        assert!(x > 1 << 20, "撞车列 x={x} 竟落在可能的世界宽度内，取位错开成为唯一防线");
    }

}