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
}
