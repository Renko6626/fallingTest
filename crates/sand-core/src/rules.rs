//! 沙/水运动规则（spec §4）。分派走材料表 Category，禁 if-else 硬编码材料名。

use crate::cell::Cell;
use crate::chunk::DirtyRect;
use crate::material::{Category, MaterialTable, DISPERSION_MAX, MAT_AIR};
use crate::rng::{rng_u32, scan_flip, STREAM_DIAG};
use crate::window::WriteWindow;

/// 单 chunk 扫描的规则上下文。
struct Ctx<'a> {
    win: &'a WriteWindow,
    table: &'a MaterialTable,
    fseed: u32,
    stamp: u8,
}

/// 扫描一个 chunk。ox/oy 为 chunk 全局原点；`start` 为起始扫描矩形——
/// Full/ChunkSleep 传 FULL（扩张天然无效），LiveRect 传 dirty ∪ next_dirty 快照。
///
/// 单代码路径 + 动态边界（O1 spec §2.2–§2.3）：扫描中本 chunk 的写入 ±1 实时并入
/// 活矩形（window 追踪），行边界与上边界每步重读——"前方"（上方行、本行未访问侧）
/// 被本遍接住，"后方/下方"留给 next_dirty 下 tick（全扫按访问序也不回访，逐位等价）。
///
/// **该等价性论证只依赖"起点用快照、终点每步重读"这个非对称结构，不依赖行方向
/// 取哪个值**——方向只决定"未访问侧"是左还是右。但它**要求方向在三种 ScanMode
/// 下取值一致**，故行方向必须是 `(tick, y)` 的纯函数：`flip` 来自 `fseed`
/// （= `(seed, tick)` 的纯函数），`y` 是全局行号，两者都与起始矩形、脏状态、
/// chunk 是否唤醒、线程调度无关。**禁止**把活矩形/脏状态/chunk 索引掺进方向判定
/// （charter §11 实施期决策第 3 条的红线，详见 `rng::STREAM_SCANDIR`）。
pub(crate) fn update_chunk(
    win: &WriteWindow,
    table: &MaterialTable,
    tick: u64,
    fseed: u32,
    start: DirtyRect,
    ox: i32,
    oy: i32,
) {
    if start.is_empty() {
        return;
    }
    win.seed_live(start);
    // 本 tick 的行方向全局相位（charter §11 实施期决策第 3 条）。每 tick 掷一次，
    // 与 chunk 无关——同一行在所有 chunk 必须同向，见 rng::STREAM_SCANDIR 文档。
    let flip = scan_flip(fseed);
    let ctx = Ctx { win, table, fseed, stamp: (tick % 256) as u8 };
    // 自下而上；底边在扫描开始时固定（向下写入必属已访问区）
    let mut ly = start.y1 as i32;
    while ly >= win.live_rect().y0 as i32 {
        let y = oy + ly;
        let ltr = (y as u64 ^ flip) & 1 == 0;
        let row = win.live_rect();
        if ltr {
            let mut lx = row.x0 as i32;
            while lx <= win.live_rect().x1 as i32 {
                ctx.eval(ox + lx, y);
                lx += 1;
            }
        } else {
            let mut lx = row.x1 as i32;
            while lx >= win.live_rect().x0 as i32 {
                ctx.eval(ox + lx, y);
                lx -= 1;
            }
        }
        ly -= 1;
    }
}

impl Ctx<'_> {
    fn eval(&self, x: i32, y: i32) {
        let c = self.win.get(x, y);
        let m = c.material();
        if !self.table.is_static(m) && c.stamp() != self.stamp {
            match self.table.category(m) {
                Category::Powder => self.powder_step(x, y, c),
                Category::Liquid => self.liquid_step(x, y, c),
                Category::Static => unreachable!(),
            }
        }
    }

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
        // 横移至多 dispersion 格（Layer G Task 1），仅入 AIR；方向承诺不变量：
        // 侧移成功后记忆 = 实际移动方向（2026-06-14 液面冻结修复的 Rust 版语义，
        // M0 spec §4.3）。失败则翻向再试一次——翻向后同样吃满色散距离。
        let d = c.dir();
        let _ = self.side(x, y, c, d) || self.side(x, y, c, -d);
    }

    /// 沿方向 `d` 探至多 `dispersion` 格，遇非 AIR 即停，移到**最远可达空格**
    /// （Layer G Task 1，spec §3.2）。
    ///
    /// **clamp 不是防御性编程而是 P4 写域论证的一部分**（spec §3.1 评审修订）：
    /// 本函数的探测半径 = 写入半径 = 色散距离，越界即写出 `WriteWindow`——
    /// debug 撞窗口断言、release 变同相数据竞争 → SyncTest 分叉。harness 加载期
    /// 有 `1..=DISPERSION_MAX` 校验，但那只是用户可见报错，直接构表的调用方
    /// （测试、未来的程序化材料表）绕得过去，故半径上界在**使用点**兜死。
    ///
    /// 掠过的中途格子不写入、不标脏——它们的内容确实没变（spec §3.2 脏矩形条）。
    /// `dispersion` 缺省 1 时与改动前逐位等价：循环只跑 i=1 一轮，`far_cell`
    /// 就是原来的 `t`。
    fn side(&self, x: i32, y: i32, c: Cell, d: i32) -> bool {
        let reach = self.table.dispersion(c.material()).min(DISPERSION_MAX) as i32;
        let mut far = x;
        let mut far_cell = Cell::AIR;
        for i in 1..=reach {
            let t = self.win.get(x + d * i, y);
            if t.material() != MAT_AIR {
                break;
            }
            far = x + d * i;
            far_cell = t;
        }
        if far == x {
            return false;
        }
        // 方向承诺不变量：记忆 = 实际移动方向（2026-06-14 液面冻结修复语义）
        self.win.set(far, y, c.with_dir(d > 0).with_stamp(self.stamp));
        self.win.set(x, y, far_cell.with_stamp(self.stamp));
        true
    }
}
