//! Cell = u32 位段（spec §1.1；Layer G Task 2 补齐 17–21 速度位段）。
//!
//! | bits | 宽 | 字段 |
//! |---|---|---|
//! | 0–7 | 8 | `material` |
//! | 8–15 | 8 | 世代戳 `stamp` |
//! | 16 | 1 | 横向方向记忆 `dir` |
//! | 17–21 | 5 | `vy` 竖直速度，Q3.2 无符号，单位 ¼ 格/tick（Layer G Task 2） |
//! | 22 | 1 | `free_falling`（O3 粉末惯性）——预留，恒 0，不读不写 |
//! | 23–31 | 9 | 留白（durability / 染色 / 温度句柄候选），未分配 |
//!
//! 布局由 `docs/superpowers/specs/2026-08-31-layer-g-velocity-design.md` §2
//! 一次性定死，避免每加一个字段抢一次位。

use crate::fixed::{Fx, FRAC_BITS};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell(pub u32);

const STAMP_SHIFT: u32 = 8;
const STAMP_MASK: u32 = 0xFF << STAMP_SHIFT;
const DIR_BIT: u32 = 1 << 16;

/// 速度位段起始位（见本文件头部布局表）。
pub const VEL_SHIFT: u32 = 17;
/// 速度位段宽度。5 位是 `V_MAX_CELL = 16` 的硬下限。
pub const VEL_BITS: u32 = 5;
const VEL_MASK: u32 = ((1 << VEL_BITS) - 1) << VEL_SHIFT;

/// 1.0 格/tick = 4 个 ¼ 格单位（Q3.2 的定标）。**必须是 2 的幂**——
/// `rules::substeps` 的概率取整靠 `% VEL_ONE` 无偏取位，非 2 的幂会引入取模偏置。
pub const VEL_ONE: u8 = 4;
const _: () = assert!(VEL_ONE.is_power_of_two(), "VEL_ONE 必须是 2 的幂（substeps 无偏取模的前提）");

/// 终端速度 = 4.0 格/tick。上界由 `window.rs` 的 r ≤ HALO 编译期断言把关。
pub const V_MAX_CELL: u8 = 4 * VEL_ONE;
const _: () = assert!((V_MAX_CELL as u32) < (1 << VEL_BITS), "V_MAX_CELL 装不进 VEL_BITS");

/// 每 tick 重力增量 = 0.25 格/tick²（16 tick ≈ 267ms 达终端速度，曲线与
/// jason.today 的 `accel 0.4 / maxSpeed 8` 同构，见 `docs/reference/noita-deep-dive.md:174`）。
///
/// **`zero-gravity` feature 把它压成 0**，供 spec §0 验收第 2 项的「零加速旁路」
/// 取证使用：`G_ACCEL = 0` ⇒ 速度恒 0 ⇒ 子步数恒 1 ⇒ 全 sim 必须与 Task 2
/// 之前逐位相同。**该 feature 只用于取证，绝不可进产品构建**——它改变物理，
/// 两端 feature 不一致即分叉（`sand-harness` 在 `G_ACCEL == 0` 时打 stderr 警告）。
pub const G_ACCEL: u8 = if cfg!(feature = "zero-gravity") { 0 } else { 1 };

/// 速度位段 ↔ `Fx` 的量纲换算移位量。位段单位是 ¼ 格/tick、`Fx` 是 Q16.16
/// 的格/tick，故差 `FRAC_BITS − log2(VEL_ONE)` 位。写成推导式而非字面量 14：
/// 谁改了 `VEL_ONE`，两个方向的换算自动跟着走。
const VEL_FRAC_SHIFT: u32 = FRAC_BITS - VEL_ONE.trailing_zeros();

/// 速度位段 → `Fx`（格/tick）。
pub(crate) fn vel_to_fx(v: u8) -> Fx {
    Fx((v as i32) << VEL_FRAC_SHIFT)
}

/// `Fx`（格/tick，**向下为正**）→ 速度位段：floor 量化 + clamp 到
/// `[0, V_MAX_CELL]`。
///
/// 向上运动（`vy < 0`）与纯水平运动一律记 0 —— 位段是**无符号竖直**速度
/// （见本文件头部布局表）。这意味着横向高速撞墙的动量目前会被丢弃：网格没有
/// 水平速度场，补它要再开一个位段，属 M2 之后的事，此处如实记录而不假装处理。
pub(crate) fn fx_to_vel(vy: Fx) -> u8 {
    (vy.0 >> VEL_FRAC_SHIFT).clamp(0, V_MAX_CELL as i32) as u8
}

/// 撞击溅射的最低速度（Layer G Task 3，spec §6.1②）：2.0 格/tick。
/// 低于此速度的撞停照旧停住，不脱格——否则每一粒静置沙落地都会炸出水花。
pub const SPLASH_MIN_SPEED: u8 = 2 * VEL_ONE;
const _: () = assert!(SPLASH_MIN_SPEED <= V_MAX_CELL, "溅射阈值高于终端速度 ⇒ 永不触发");

impl Cell {
    pub const AIR: Cell = Cell(0);

    pub fn pack(material: u8, stamp: u8) -> Cell {
        Cell(material as u32 | ((stamp as u32) << STAMP_SHIFT))
    }

    pub fn material(self) -> u8 {
        self.0 as u8
    }

    pub fn stamp(self) -> u8 {
        (self.0 >> STAMP_SHIFT) as u8
    }

    /// 横向方向记忆：-1 = 左（位 0），+1 = 右（位 1）。
    pub fn dir(self) -> i32 {
        if self.0 & DIR_BIT != 0 { 1 } else { -1 }
    }

    pub fn with_stamp(self, stamp: u8) -> Cell {
        Cell((self.0 & !STAMP_MASK) | ((stamp as u32) << STAMP_SHIFT))
    }

    pub fn with_dir(self, right: bool) -> Cell {
        if right { Cell(self.0 | DIR_BIT) } else { Cell(self.0 & !DIR_BIT) }
    }

    /// 竖直速度（Q3.2 无符号，单位 ¼ 格/tick；`0..=V_MAX_CELL`）。
    /// 无符号是有意的：粉末与液体只向下，向上运动走 Layer F（气体）或
    /// Layer P（脱格粒子），省一位（spec §2）。
    pub fn vel(self) -> u8 {
        ((self.0 & VEL_MASK) >> VEL_SHIFT) as u8
    }

    /// 写入竖直速度。调用方保证 `v <= V_MAX_CELL`（`rules::eval` 用 `min` 夹住）；
    /// 这里仍按位段宽度掩码，避免越界值污染 22 位以上的预留区。
    pub fn with_vel(self, v: u8) -> Cell {
        Cell((self.0 & !VEL_MASK) | (((v as u32) << VEL_SHIFT) & VEL_MASK))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitfield_roundtrip() {
        let c = Cell::pack(3, 0xAB).with_dir(true);
        assert_eq!(c.material(), 3);
        assert_eq!(c.stamp(), 0xAB);
        assert_eq!(c.dir(), 1);
        let c = c.with_stamp(0x01).with_dir(false);
        assert_eq!(c.material(), 3);
        assert_eq!(c.stamp(), 0x01);
        assert_eq!(c.dir(), -1);
    }

    /// Layer G Task 2：`vel` 位段（17–21）与 material/stamp/dir 互不干扰，
    /// 且能表示 `0..=V_MAX_CELL` 的全部取值（5 位是 16 的硬下限）。
    #[test]
    fn vel_roundtrip_does_not_disturb_other_fields() {
        let base = Cell::pack(3, 0xAB).with_dir(true);
        for v in 0..=V_MAX_CELL {
            let c = base.with_vel(v);
            assert_eq!(c.vel(), v, "vel 往返失败：{v}");
            assert_eq!(c.material(), 3, "vel 写入污染了 material（v={v}）");
            assert_eq!(c.stamp(), 0xAB, "vel 写入污染了 stamp（v={v}）");
            assert_eq!(c.dir(), 1, "vel 写入污染了 dir（v={v}）");
        }
        // 反向：改 material/stamp/dir 不得动 vel
        let c = base.with_vel(V_MAX_CELL).with_stamp(7).with_dir(false);
        assert_eq!(c.vel(), V_MAX_CELL, "with_stamp/with_dir 污染了 vel");
    }

    /// 量纲换算往返（Layer G Task 3 的 P→G 通路依赖它）：位段 → `Fx` → 位段
    /// 必须是恒等变换，否则粒子落格的撞击速度会在换算里悄悄丢一档。
    #[test]
    fn vel_fx_roundtrip_is_identity() {
        for v in 0..=V_MAX_CELL {
            assert_eq!(fx_to_vel(vel_to_fx(v)), v, "位段 {v} 的 Fx 往返不是恒等");
        }
        assert_eq!(vel_to_fx(VEL_ONE), Fx::from_int(1), "VEL_ONE 必须正好是 1.0 格/tick");
    }

    /// 向上/水平运动记 0，超速 clamp 到 `V_MAX_CELL`——两端都不许溢出位段。
    #[test]
    fn fx_to_vel_clamps_both_ends() {
        assert_eq!(fx_to_vel(Fx::from_int(-5)), 0, "向上运动必须记 0（位段无符号）");
        assert_eq!(fx_to_vel(Fx::ZERO), 0);
        assert_eq!(fx_to_vel(Fx::from_int(99)), V_MAX_CELL, "超速必须 clamp 到终端速度");
    }

    /// 位段预算执法：`vel` 必须真的落在 17–21，不许溢到 22（`free_falling` 预留位）。
    #[test]
    fn vel_occupies_only_bits_17_to_21() {
        assert_eq!(Cell(0).with_vel(V_MAX_CELL).0, (V_MAX_CELL as u32) << VEL_SHIFT);
        assert_eq!(VEL_SHIFT + VEL_BITS, 22);
        // "V_MAX_CELL 装得进 VEL_BITS" 由本文件顶部的 const 断言在编译期兜死，
        // 这里再写一遍只会被 clippy 判为恒真断言。
    }

    #[test]
    fn air_is_zero() {
        assert_eq!(Cell::AIR.0, 0);
        assert_eq!(Cell::pack(0, 0), Cell::AIR);
    }
}
