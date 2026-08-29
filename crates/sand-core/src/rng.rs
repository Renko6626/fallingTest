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
