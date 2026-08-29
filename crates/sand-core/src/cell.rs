//! Cell = u32 位段（spec §1.1）。
//! bits 0–7 material / 8–15 世代戳 / 16 方向记忆 / 17–31 aux（M0 恒 0）。

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell(pub u32);

const STAMP_SHIFT: u32 = 8;
const STAMP_MASK: u32 = 0xFF << STAMP_SHIFT;
const DIR_BIT: u32 = 1 << 16;

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

    #[test]
    fn air_is_zero() {
        assert_eq!(Cell::AIR.0, 0);
        assert_eq!(Cell::pack(0, 0), Cell::AIR);
    }
}
