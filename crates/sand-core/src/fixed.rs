//! Q16.16 定点数（spec §2）。范围 ±32768 格，1280×768 世界余量充足。
//! 手写极简 newtype——不上 fixed crate、不搬 STG 代码（M1 运算面太小，自家可审可测；
//! 日后要换，newtype 边界不动）。除法只有 `from_ratio` 一处（常量构造），无运行时
//! 除法链。**无三角函数**——方向量走分量运算 + `isqrt` 归一。
//!
//! 全部算术走 `wrapping_*`：保证同一份代码在 dev/release profile 下位级一致，
//! 不依赖 Rust 溢出检查开关（那是运行时环境差异，不是逻辑本身，但仍会破坏
//! 跨机确定性，因此在核心里一律显式 wrapping）。

use std::ops::{Add, Neg, Sub};

use crate::sin_table::SIN_TABLE;

pub(crate) const FRAC_BITS: u32 = 16;

/// Q16.16 定点：高位整数部分 + 低 16 位小数部分，二补码 `i32`。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Fx(pub i32);

/// 半格偏移（Q16.16 的 0.5）：格坐标 → 格心连续坐标。
///
/// 粒子出生点一律取格中心而非格角——`cell_walk` 的 DDA 几何要求连续坐标，
/// 格心比格角安全（格角会撞上"恰好贴边界时 `rem = 0`"那条讨论，见 `dda.rs`
/// 顶部注释）。两个使用者：`explode` 的爆炸溅射与 `rules` 的撞击溅射
/// （Layer G Task 3 起），故提到本模块共享。
pub(crate) const HALF_CELL: Fx = Fx(0x0000_8000);

impl Fx {
    pub const ZERO: Fx = Fx(0);

    /// 整数 v → Q16.16（左移 16 位）。左移不做溢出检查（Rust 对移位量而非移出
    /// 的高位做 panic 校验，16 恒 < 32，dev/release 行为一致）。
    pub fn from_int(v: i32) -> Fx {
        Fx(v << FRAC_BITS)
    }

    /// num/den → Q16.16。整数除法**向零截断**（唯一除法点，仅用于常量构造，
    /// 如 `GRAVITY = from_ratio(1, 4)`）；调用方保证 `den != 0`。
    /// 注意与 [`Fx::mul`] / [`Fx::to_cell`] 的 floor 语义不同——那两处走算术
    /// 右移，是刻意的两套约定，别混用心智模型。
    pub fn from_ratio(num: i32, den: i32) -> Fx {
        Fx((((num as i64) << FRAC_BITS) / (den as i64)) as i32)
    }

    /// floor 到格坐标。算术右移在二补码下就是整数域的 floor 除法，负数同样
    /// 成立：`Fx(-1)`（位模式 -1，代表 -1/65536 的极小负数）floor 到 -1，
    /// 而不是向零截断给出的 0（金值测试钉死）。
    pub fn to_cell(self) -> i32 {
        self.0 >> FRAC_BITS
    }

    /// 乘整数：k 不带小数位，量纲不变，直接整数乘（wrapping，避免 debug/release
    /// 因溢出检查开关而分叉）。
    pub fn mul_int(self, k: i32) -> Fx {
        Fx(self.0.wrapping_mul(k))
    }

    /// 乘 Fx：i64 中间量避免中途溢出，`>> 16` 收尾。这是算术右移（floor 截断，
    /// 负数亦然），**不是**向零截断的整数除法——两者在余数非零时结果不同，
    /// 由同名单测钉死。
    ///
    /// 特意做成裸方法而非 `impl Mul`（任务书 §2 签名如此），故 clippy 的
    /// "可能与 std::ops::Mul 混淆" 提示在此不适用。
    #[allow(clippy::should_implement_trait)]
    pub fn mul(self, o: Fx) -> Fx {
        let p = (self.0 as i64).wrapping_mul(o.0 as i64);
        Fx((p >> FRAC_BITS) as i32)
    }
}

impl Add for Fx {
    type Output = Fx;
    fn add(self, rhs: Fx) -> Fx {
        Fx(self.0.wrapping_add(rhs.0))
    }
}

impl Sub for Fx {
    type Output = Fx;
    fn sub(self, rhs: Fx) -> Fx {
        Fx(self.0.wrapping_sub(rhs.0))
    }
}

impl Neg for Fx {
    type Output = Fx;
    fn neg(self) -> Fx {
        Fx(self.0.wrapping_neg())
    }
}

/// 整数平方根，floor 语义：最大的 `r` 使 `r*r <= v`（爆炸径向归一用）。
///
/// 牛顿法，全程 `u64` 整数运算：初始猜测 `x0 = 2^ceil(bits/2)`（`bits` 是 `v`
/// 的有效位数）保证 `x0 >= floor(sqrt(v))` 严格成立，序列单调递减收敛到不动点
/// 即为 floor(sqrt(v))（标准整数牛顿法性质）。`v == 0` 单独处理避免除零。
pub fn isqrt(v: u64) -> u32 {
    if v == 0 {
        return 0;
    }
    let bits = 64 - v.leading_zeros();
    let mut x: u64 = 1u64 << bits.div_ceil(2);
    loop {
        let y = (x + v / x) / 2;
        if y >= x {
            break;
        }
        x = y;
    }
    x as u32
}

/// BAM 角（binary angle measurement）：无符号 16 位，65536 = 360°，0 = +x。
/// 选它而非度数/弧度是因为**加减法天然环绕**，无需取模，且与架构 §3
/// `bridge-input` 条目定的编码一致。
pub type Bam = u16;

/// BAM 角 → 单位方向向量 `(cos, sin)`，查 1024 项表（角分辨率 0.35°）。
/// 核心禁用系统超越函数（总纲 §6），故这里是唯一的三角来源。
///
/// **坐标约定**：屏幕坐标系，+y 朝下。角度从 +x 轴起算，随值增大朝 +y 转
/// （视觉上为顺时针）——0 = +x，16384（90°）= +y，32768（180°）= -x，
/// 49152（270°）= -y（`dir_of_cardinals_are_exact` 钉死这四点）。
pub fn dir_of(a: Bam) -> (Fx, Fx) {
    let i = (a >> 6) as usize; // 65536 / 1024 = 64
    let sin = Fx(SIN_TABLE[i]);
    let cos = Fx(SIN_TABLE[(i + 256) & 1023]); // cos θ = sin(θ + 90°)
    (cos, sin)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 构造与位模式 ----

    #[test]
    fn from_int_shifts_left_16() {
        assert_eq!(Fx::from_int(1).0, 0x0001_0000);
        assert_eq!(Fx::from_int(-1).0, -0x0001_0000);
        assert_eq!(Fx::from_int(0).0, 0);
        assert_eq!(Fx::ZERO.0, 0);
    }

    #[test]
    fn from_ratio_bit_pattern() {
        // GRAVITY 初值（spec §2）：0.25 格/tick² = from_ratio(1, 4)
        assert_eq!(Fx::from_ratio(1, 4).0, 0x0000_4000);
        assert_eq!(Fx::from_ratio(1, 2).0, 0x0000_8000);
        assert_eq!(Fx::from_ratio(3, 1).0, 0x0003_0000);
        // 负值经分子或分母传入，符号一致
        assert_eq!(Fx::from_ratio(-1, 4).0, -0x0000_4000);
        assert_eq!(Fx::from_ratio(1, -4).0, -0x0000_4000);
    }

    #[test]
    fn from_ratio_truncates_toward_zero_not_floor() {
        // 65536/3 = 21845.33…，正数下截断与 floor 一致
        assert_eq!(Fx::from_ratio(1, 3).0, 21845);
        // -65536/3 = -21845.33…，向零截断给 -21845；若是 floor 会是 -21846。
        // 钉死 from_ratio 走 `/`（截断），与 mul/to_cell 的 `>>`（floor）不同。
        assert_eq!(Fx::from_ratio(-1, 3).0, -21845);
    }

    // ---- floor 到格坐标 ----

    #[test]
    fn to_cell_floors_positive() {
        assert_eq!(Fx::from_int(3).to_cell(), 3);
        assert_eq!(Fx(0x0003_8000).to_cell(), 3); // 3.5 格 floor 到 3
    }

    #[test]
    fn to_cell_floors_negative_not_toward_zero() {
        // Fx(-1) 的位模式是 -1，代表 -1/65536 的极小负数；floor 必须是 -1，
        // 而不是向零截断会给出的 0。
        assert_eq!(Fx(-1).to_cell(), -1);
        assert_eq!(Fx::from_int(-3).to_cell(), -3);
        // -3.5 格 floor 到 -4，不是向零截断的 -3
        assert_eq!(Fx(-0x0003_8000).to_cell(), -4);
    }

    // ---- mul_int ----

    #[test]
    fn mul_int_scales_without_shift() {
        assert_eq!(Fx::from_ratio(1, 4).mul_int(4), Fx::from_int(1));
        assert_eq!(Fx::from_int(-2).mul_int(3), Fx::from_int(-6));
        assert_eq!(Fx::ZERO.mul_int(100), Fx::ZERO);
    }

    // ---- mul：i64 中间量、算术右移（钉死负值 floor 行为）----

    #[test]
    fn mul_exact_values() {
        assert_eq!(Fx::from_int(2).mul(Fx::from_int(3)), Fx::from_int(6));
        assert_eq!(
            Fx::from_ratio(1, 2).mul(Fx::from_ratio(1, 2)),
            Fx::from_ratio(1, 4)
        );
        assert_eq!(Fx::from_int(-2).mul(Fx::from_int(3)), Fx::from_int(-6));
    }

    #[test]
    fn mul_negative_is_floor_not_round_to_zero() {
        // raw 乘积 -1*3 = -3；算术右移 16 位 floor(-3/65536) = -1。
        // 若走向零截断的整数除法 -3/65536，Rust 语义会给 0——两者在此分叉，
        // 这里钉死实现必须是 `>>` 而不是 `/`。
        assert_eq!(Fx(-1).mul(Fx(3)).0, -1);
        assert_eq!(Fx(3).mul(Fx(-1)).0, -1);
    }

    // ---- Add/Sub/Neg ----

    #[test]
    fn add_sub_neg() {
        let a = Fx::from_int(3);
        let b = Fx::from_ratio(1, 2);
        assert_eq!((a + b).0, 0x0003_8000);
        assert_eq!((a - b).0, 0x0002_8000);
        assert_eq!((-a).0, -0x0003_0000);
        assert_eq!(a - a, Fx::ZERO);
    }

    #[test]
    fn ordering() {
        assert!(Fx::from_int(-1) < Fx::ZERO);
        assert!(Fx::ZERO < Fx::from_int(1));
        assert!(Fx::from_ratio(1, 4) < Fx::from_ratio(1, 2));
    }

    // ---- isqrt 边界 ----

    #[test]
    fn isqrt_small_values() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(1), 1);
        assert_eq!(isqrt(2), 1);
        assert_eq!(isqrt(3), 1);
        assert_eq!(isqrt(4), 2);
    }

    #[test]
    fn isqrt_perfect_squares_and_neighbors() {
        for r in [2u64, 3, 10, 100, 1000, 65536, 100_000, u32::MAX as u64] {
            let sq = r * r;
            assert_eq!(isqrt(sq), r as u32, "isqrt({sq}) 应为 {r}");
            assert_eq!(isqrt(sq + 1), r as u32, "平方 +1 仍 floor 到 {r}");
            assert_eq!(isqrt(sq - 1), (r - 1) as u32, "平方 -1 floor 到 {}", r - 1);
        }
    }

    #[test]
    fn isqrt_u64_max() {
        assert_eq!(isqrt(u64::MAX), u32::MAX);
    }

    // ---- sin 表 + dir_of（M4 Task 1）----

    #[test]
    fn sin_table_golden_checksum() {
        // 金值：表一旦被改动即失败（生成物不得手改）
        let mut h = xxhash_rust::xxh3::Xxh3::new();
        for v in crate::sin_table::SIN_TABLE {
            h.update(&v.to_le_bytes());
        }
        assert_eq!(h.digest(), 0x02ac_6746_2a72_29c9, "sin 表被改动——重跑生成脚本或恢复");
    }

    #[test]
    fn dir_of_cardinals_are_exact() {
        assert_eq!(dir_of(0), (Fx::from_int(1), Fx::ZERO), "0° = +x");
        assert_eq!(dir_of(16384), (Fx::ZERO, Fx::from_int(1)), "90° = +y（屏幕坐标向下）");
        assert_eq!(dir_of(32768), (Fx::from_int(-1), Fx::ZERO), "180° = -x");
        assert_eq!(dir_of(49152), (Fx::ZERO, Fx::from_int(-1)), "270° = -y");
    }

    #[test]
    fn dir_of_is_unit_length_within_tolerance() {
        // 查表 + 定点截断的误差上界：逐项检查 |v|² 落在 1.0 ± 1/256 内
        for a in (0u32..65536).step_by(37) {
            let (cx, cy) = dir_of(a as u16);
            let n = (cx.mul(cx) + cy.mul(cy)).0 as i64;
            let one = 1i64 << 16;
            assert!((n - one).abs() < one / 256, "角 {a} 的模平方 {n} 偏离 1.0 过多");
        }
    }
}
