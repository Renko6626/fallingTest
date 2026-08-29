//! 沙/水运动规则（spec §4）。分派走材料表 Category，禁 if-else 硬编码材料名。

use crate::cell::Cell;
use crate::chunk::DirtyRect;
use crate::material::{Category, MaterialTable, MAT_AIR};
use crate::rng::{rng_u32, STREAM_DIAG};
use crate::window::WriteWindow;

/// 扫描一个 chunk 的脏矩形。ox/oy 为 chunk 全局原点。
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
    let stamp = (tick % 256) as u8;
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
            if !table.is_static(m) && c.stamp() != stamp {
                match table.category(m) {
                    Category::Powder => powder_step(win, table, x, y, c, fseed, stamp),
                    Category::Liquid => liquid_step(win, table, x, y, c, fseed, stamp),
                    Category::Static => unreachable!(),
                }
            }
            lx += step;
        }
    }
}

/// 目标是 AIR → 移入；目标非 Static 且密度更小 → 置换。双方盖戳。
fn try_displace(
    win: &WriteWindow,
    table: &MaterialTable,
    x: i32,
    y: i32,
    c: Cell,
    nx: i32,
    ny: i32,
    stamp: u8,
) -> bool {
    let t = win.get(nx, ny);
    let tm = t.material();
    let ok = tm == MAT_AIR
        || (!table.is_static(tm) && table.density(tm) < table.density(c.material()));
    if ok {
        win.set(nx, ny, c.with_stamp(stamp));
        win.set(x, y, t.with_stamp(stamp));
    }
    ok
}

fn diag_side(fseed: u32, x: i32, y: i32) -> i32 {
    if rng_u32(fseed, STREAM_DIAG, x, y, 0, 0) & 1 == 0 { 1 } else { -1 }
}

fn powder_step(
    win: &WriteWindow,
    table: &MaterialTable,
    x: i32,
    y: i32,
    c: Cell,
    fseed: u32,
    stamp: u8,
) {
    if try_displace(win, table, x, y, c, x, y + 1, stamp) {
        return;
    }
    let s = diag_side(fseed, x, y);
    if try_displace(win, table, x, y, c, x + s, y + 1, stamp) {
        return;
    }
    let _ = try_displace(win, table, x, y, c, x - s, y + 1, stamp);
}

fn liquid_step(
    win: &WriteWindow,
    table: &MaterialTable,
    x: i32,
    y: i32,
    c: Cell,
    fseed: u32,
    stamp: u8,
) {
    if try_displace(win, table, x, y, c, x, y + 1, stamp) {
        return;
    }
    let s = diag_side(fseed, x, y);
    if try_displace(win, table, x, y, c, x + s, y + 1, stamp)
        || try_displace(win, table, x, y, c, x - s, y + 1, stamp)
    {
        return;
    }
    // 横移 1 格，仅入 AIR；方向承诺不变量：侧移成功后记忆 = 实际移动方向
    // （2026-06-14 液面冻结修复的 Rust 版语义，spec §4.3）
    let d = c.dir();
    let _ = try_side(win, x, y, c, d, stamp) || try_side(win, x, y, c, -d, stamp);
}

fn try_side(win: &WriteWindow, x: i32, y: i32, c: Cell, d: i32, stamp: u8) -> bool {
    let t = win.get(x + d, y);
    if t.material() == MAT_AIR {
        win.set(x + d, y, c.with_dir(d > 0).with_stamp(stamp));
        win.set(x, y, t.with_stamp(stamp));
        true
    } else {
        false
    }
}
