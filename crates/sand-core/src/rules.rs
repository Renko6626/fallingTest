//! 沙/水运动规则（spec §4）。分派走材料表 Category，禁 if-else 硬编码材料名。

use crate::cell::Cell;
use crate::chunk::DirtyRect;
use crate::material::{Category, MaterialTable, MAT_AIR};
use crate::rng::{rng_u32, STREAM_DIAG};
use crate::window::WriteWindow;

/// 单 chunk 扫描的规则上下文。
struct Ctx<'a> {
    win: &'a WriteWindow,
    table: &'a MaterialTable,
    fseed: u32,
    stamp: u8,
}

/// 扫描一个 chunk（活跃 chunk 全量扫描，spec §1.4 修订）。ox/oy 为 chunk 全局原点。
pub(crate) fn update_chunk(
    win: &WriteWindow,
    table: &MaterialTable,
    tick: u64,
    fseed: u32,
    scan: DirtyRect,
    ox: i32,
    oy: i32,
) {
    if scan.is_empty() {
        return;
    }
    let ctx = Ctx { win, table, fseed, stamp: (tick % 256) as u8 };
    // 自下而上；行内方向按 (y + tick) 奇偶交替（spec §3.2）
    for ly in (scan.y0..=scan.y1).rev() {
        let y = oy + ly as i32;
        let ltr = (y as u64 + tick) & 1 == 0;
        let (mut lx, end, step) = if ltr {
            (scan.x0 as i32, scan.x1 as i32 + 1, 1)
        } else {
            (scan.x1 as i32, scan.x0 as i32 - 1, -1)
        };
        while lx != end {
            let x = ox + lx;
            let c = win.get(x, y);
            let m = c.material();
            if !table.is_static(m) && c.stamp() != ctx.stamp {
                match table.category(m) {
                    Category::Powder => ctx.powder_step(x, y, c),
                    Category::Liquid => ctx.liquid_step(x, y, c),
                    Category::Static => unreachable!(),
                }
            }
            lx += step;
        }
    }
}

impl Ctx<'_> {
    /// 目标是 AIR → 移入；目标非 Static 且密度更小 → 置换。双方盖戳。
    fn displace(&self, x: i32, y: i32, c: Cell, nx: i32, ny: i32) -> bool {
        let t = self.win.get(nx, ny);
        let tm = t.material();
        let ok = tm == MAT_AIR
            || (!self.table.is_static(tm)
                && self.table.density(tm) < self.table.density(c.material()));
        if ok {
            self.win.set(nx, ny, c.with_stamp(self.stamp));
            self.win.set(x, y, t.with_stamp(self.stamp));
        }
        ok
    }

    fn diag_side(&self, x: i32, y: i32) -> i32 {
        if rng_u32(self.fseed, STREAM_DIAG, x, y, 0, 0) & 1 == 0 { 1 } else { -1 }
    }

    fn powder_step(&self, x: i32, y: i32, c: Cell) {
        if self.displace(x, y, c, x, y + 1) {
            return;
        }
        let s = self.diag_side(x, y);
        let _ = self.displace(x, y, c, x + s, y + 1) || self.displace(x, y, c, x - s, y + 1);
    }

    fn liquid_step(&self, x: i32, y: i32, c: Cell) {
        if self.displace(x, y, c, x, y + 1) {
            return;
        }
        let s = self.diag_side(x, y);
        if self.displace(x, y, c, x + s, y + 1) || self.displace(x, y, c, x - s, y + 1) {
            return;
        }
        // 横移 1 格，仅入 AIR；方向承诺不变量：侧移成功后记忆 = 实际移动方向
        // （2026-06-14 液面冻结修复的 Rust 版语义，spec §4.3）
        let d = c.dir();
        let _ = self.side(x, y, c, d) || self.side(x, y, c, -d);
    }

    fn side(&self, x: i32, y: i32, c: Cell, d: i32) -> bool {
        let t = self.win.get(x + d, y);
        if t.material() == MAT_AIR {
            self.win.set(x + d, y, c.with_dir(d > 0).with_stamp(self.stamp));
            self.win.set(x, y, t.with_stamp(self.stamp));
            true
        } else {
            false
        }
    }
}
