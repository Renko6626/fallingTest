//! 玩家意图的**唯一**入核通道（架构 §1 铁律 1、§3 `bridge-input`）。
//! 生物控制器只吃本结构——这让 P2"Godot → 核心唯一写入路径 = InputFrame"
//! 在 M4 就获得类型级担保，不必等 bridge 落地。

use crate::fixed::Bam;

pub const BTN_LEFT: u8 = 1 << 0;
pub const BTN_RIGHT: u8 = 1 << 1;
pub const BTN_JUMP: u8 = 1 << 2;
pub const BTN_FIRE: u8 = 1 << 3;
pub const BTN_DOWN: u8 = 1 << 4;

/// loadout 槽位数（spec §3.2/§6.1）。
pub const MAX_SLOTS: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct InputFrame {
    pub buttons: u8,
    pub aim: Bam,
    pub slot: u8,
}

impl InputFrame {
    /// `slot` 在构造点 clamp 进 `0..MAX_SLOTS`——脏值不得进状态。
    pub fn new(buttons: u8, aim: Bam, slot: u8) -> InputFrame {
        InputFrame { buttons, aim, slot: slot.min((MAX_SLOTS - 1) as u8) }
    }

    pub fn held(self, mask: u8) -> bool {
        self.buttons & mask != 0
    }

    /// 网络/回放编码：4 字节小端打包（架构 §3 定的"约 8 字节"上限内）。
    pub fn pack(self) -> u32 {
        (self.buttons as u32) | ((self.aim as u32) << 8) | ((self.slot as u32) << 24)
    }

    pub fn unpack(v: u32) -> InputFrame {
        InputFrame::new((v & 0xFF) as u8, ((v >> 8) & 0xFFFF) as u16, ((v >> 24) & 0xFF) as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_roundtrip_is_identity() {
        for &(b, a, s) in &[(0u8, 0u16, 0u8), (0b1_0101, 12345, 3), (0xFF, 65535, 3)] {
            let f = InputFrame::new(b, a, s);
            assert_eq!(InputFrame::unpack(f.pack()), f, "buttons={b} aim={a} slot={s}");
        }
    }

    #[test]
    fn slot_is_clamped_into_range_at_construction() {
        // 越界槽位不得进状态：加载期/桥侧可能传脏值，构造点收口
        assert_eq!(InputFrame::new(0, 0, 200).slot, (MAX_SLOTS - 1) as u8);
    }

    #[test]
    fn held_reads_the_right_bit() {
        let f = InputFrame::new(BTN_LEFT | BTN_FIRE, 0, 0);
        assert!(f.held(BTN_LEFT) && f.held(BTN_FIRE));
        assert!(!f.held(BTN_RIGHT) && !f.held(BTN_JUMP));
    }
}
