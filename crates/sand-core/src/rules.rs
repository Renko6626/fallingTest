//! 沙/水运动规则（spec §4）。分派走材料表 Category，禁 if-else 硬编码材料名。

use crate::cell::{vel_to_fx, Cell, G_ACCEL, SPLASH_MIN_SPEED, VEL_ONE, V_MAX_CELL};
use crate::chunk::DirtyRect;
use crate::emit::emit_jitter;
use crate::fixed::{Fx, HALF_CELL};
use crate::material::{Category, MaterialTable, DISPERSION_MAX, MAT_AIR};
use crate::particle::clamp_speed;
use crate::rng::{rng_u32, scan_flip, STREAM_DIAG, STREAM_FALLSTEP, STREAM_SPLASH};
use crate::window::WriteWindow;
use crate::world::SpawnRequest;

/// 溅射反弹系数（Layer G Task 3，spec §6.3）：脱格粒子的竖直初速
/// = −v1 × 本值。0.5 = 撞击动能一半转成向上的水花。
const SPLASH_RESTITUTION: Fx = Fx(0x0000_8000);

/// 溅射速度的逐轴抖动幅度（格/tick）。取 0.5 是为了让水花散开又不至于翻向
/// 下：竖直初速在阈值处已是 `SPLASH_MIN_SPEED × VEL_UNIT × RESTITUTION`
/// = 1.0 格/tick，减去至多 0.5 的抖动仍然向上。同名单测钉死该不等式。
const SPLASH_JITTER: Fx = Fx(0x0000_8000);

/// 三颗溅射骰的 `attempt` 标号（共用 [`STREAM_SPLASH`]，体例同
/// `explode.rs` 的 `EXPLODE_ROLL_*`）。
const SPLASH_ROLL_TRIGGER: u32 = 0;
const SPLASH_ROLL_VX: u32 = 1;
const SPLASH_ROLL_VY: u32 = 2;

/// 概率骰的量化分母。`splash_chance` 由 harness 按 `×255 round` 量化，
/// 故这里也除以 255：`chance = 255` ⇒ `roll % 255` 恒 `< 255` ⇒ 必溅射；
/// `chance = 0` ⇒ 恒不溅射。两个端点都精确，中间值有 `2^32 mod 255` 量级的
/// 取模偏置（相对 2^-24），远低于任何可观测阈值——同 `emit_jitter` 文档记的
/// "残余非均匀性"处理方式，不做额外校正。
const SPLASH_CHANCE_DEN: u32 = 255;

/// 单个子步的结果（Layer G Task 2，spec §4.1）。带上落点坐标，外层循环据此
/// 推进 `cur`——判定逻辑一行不改，只是把"成功/失败"回传给子步循环。
enum Step {
    /// 竖直或斜下移动成功，速度保留，继续下一子步。
    ///
    /// **斜滑不清零速度**是有意选择（spec §4.2④，2026-08-31 用户裁定采纳默认）：
    /// jason.today / Noita 都是这个行为，沙堆坍塌因此明显更快。若日后目检认为
    /// 塌得太夸张，改成"斜滑即 stalled"只是把这里换成 `MovedSide`，
    /// 且 §5 的水平半径余量只会变大。
    Moved(i32, i32),
    /// 色散横移成功。**走到这一步本身就意味着撞停**——下方与两个斜下都被挡了，
    /// 竖直动能已经耗尽，故循环终止且速度清零（spec §4.1）。
    MovedSide(i32, i32),
    /// 无处可去。
    Blocked,
}

/// 本 tick 的子步数 `n = max(1, v1/VEL_ONE + frac_roll)`（spec §4.1）。
///
/// **纯整数**（charter §6 数值红线：网格逻辑禁浮点）。小数部分用一次掷骰兑换
/// 成"多走一格"的概率，等价于零存储的子像素精度：长期平均位移 = `v1/VEL_ONE`。
/// `VEL_ONE` 是 2 的幂（`cell.rs` 有编译期断言）⇒ `% VEL_ONE` 是无偏取位，
/// 不存在取模偏置。
///
/// `max(1, ..)` 是**可退化性的来源**（spec §4.2①）：`v1 < VEL_ONE` 时 n = 1，
/// 走的就是 Task 2 之前那条路径；`G_ACCEL = 0` 时速度恒 0 ⇒ 全 sim 逐位不变。
///
/// key 取 cell 的**起始坐标**、不带 salt/attempt，理由见 [`STREAM_FALLSTEP`]。
fn substeps(fseed: u32, v1: u8, x: i32, y: i32) -> u32 {
    let whole = (v1 / VEL_ONE) as u32;
    let frac = (v1 % VEL_ONE) as u32;
    let roll = rng_u32(fseed, STREAM_FALLSTEP, x, y, 0, 0) % VEL_ONE as u32;
    (whole + u32::from(roll < frac)).max(1)
}

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
    /// 一个 cell 的一个 tick：重力积分 + 子步循环（Layer G Task 2，spec §4.1）。
    ///
    /// ```text
    /// v1 = min(v0 + G_ACCEL, V_MAX_CELL)
    /// n  = max(1, v1/VEL_ONE + frac_roll)      // 概率取整 = 零存储子像素精度
    /// 逐子步走 powder_step / liquid_step，撞停（Blocked / MovedSide）即终止
    /// v_final = if stalled { 0 } else { v1 }
    /// ```
    ///
    /// **写回纪律 = 休眠的生命线**（spec §4.2②，不是优化而是正确性红线）：
    /// 只在 `v_final ≠ 落点已存值` 时写。静止堆体 `v0 = 0` → 第 0 子步即
    /// `Blocked` → `v_final = 0` = 已存值 → **零写入** → `next_dirty` 空 →
    /// chunk 照旧入睡（`scheduler.rs:74`）。若照 jason.today 原样无条件
    /// `v += accel` 写回，静止沙的速度会从 0 涨起来 → 每 tick 一次 `set()` →
    /// `mark_dirty_around` → 整张图永不入睡，M0 建立的稀疏性能当场退回全量扫描。
    /// 执法测试：`tests/rules_behavior.rs::resting_pile_lets_every_chunk_sleep`。
    fn eval(&self, x: i32, y: i32) {
        let c = self.win.get(x, y);
        let m = c.material();
        if self.table.is_static(m) || c.stamp() == self.stamp {
            return;
        }
        let cat = self.table.category(m);
        let v1 = (c.vel() + G_ACCEL).min(V_MAX_CELL);
        let n = substeps(self.fseed, v1, x, y);
        let moving = c.with_vel(v1);
        let (mut cx, mut cy) = (x, y);
        let mut stalled = false;
        for k in 0..n {
            let step = match cat {
                Category::Powder => self.powder_step(cx, cy, moving, k),
                Category::Liquid => self.liquid_step(cx, cy, moving, k),
                Category::Static => unreachable!(),
            };
            match step {
                Step::Moved(nx, ny) => (cx, cy) = (nx, ny),
                Step::MovedSide(nx, ny) => {
                    (cx, cy) = (nx, ny);
                    stalled = true;
                    break;
                }
                Step::Blocked => {
                    stalled = true;
                    break;
                }
            }
        }
        // 撞击溅射脱格（Layer G Task 3，spec §6.1）：三条件全中才走。放在速度
        // 写回**之前**——脱格后该格已是 AIR，再写速度就是往空气里写垃圾。
        if stalled && self.try_splash(cx, cy, x, y, c, v1) {
            return;
        }
        let v_final = if stalled { 0 } else { v1 };
        let landed = self.win.get(cx, cy);
        if landed.vel() != v_final {
            self.win.set(cx, cy, landed.with_vel(v_final));
        }
    }

    /// 撞击溅射的三条件判定与执行（spec §6.1/§6.3）。返回是否真的脱格了。
    ///
    /// `(cx, cy)` = 撞停格（粒子出生点），`(sx, sy)` = 该 cell 本 tick 的
    /// **起始**坐标（三颗骰的 RNG key）。两者必须分开传：撞停坐标同 tick 内
    /// 不唯一，用它当 key 会让整列连锁同进同退，详见 [`STREAM_SPLASH`]。
    ///
    /// **`Blocked` 与 `MovedSide` 都算撞停，是有意为之**（spec §6.1①）：
    /// 瀑布砸进水面走的正是"下方被挡 → 色散走开"这条 `MovedSide` 路径，而它
    /// 恰是水花的主要来源。副作用是高速水贴地横流也会冒向上的水花——此项
    /// 列入目检清单，若认为不对，改成"仅 `Blocked` 触发"是一行判别的事。
    fn try_splash(&self, cx: i32, cy: i32, sx: i32, sy: i32, c: Cell, v1: u8) -> bool {
        if v1 < SPLASH_MIN_SPEED {
            return false;
        }
        let chance = self.table.splash_chance(c.material());
        if chance == 0 {
            return false;
        }
        let roll = rng_u32(self.fseed, STREAM_SPLASH, sx, sy, 0, SPLASH_ROLL_TRIGGER);
        if roll % SPLASH_CHANCE_DEN >= chance as u32 {
            return false;
        }
        let speed = vel_to_fx(v1).mul(SPLASH_RESTITUTION);
        let rx = rng_u32(self.fseed, STREAM_SPLASH, sx, sy, 0, SPLASH_ROLL_VX);
        let ry = rng_u32(self.fseed, STREAM_SPLASH, sx, sy, 0, SPLASH_ROLL_VY);
        let vx = clamp_speed(emit_jitter(rx, SPLASH_JITTER));
        let vy = clamp_speed(-speed + emit_jitter(ry, SPLASH_JITTER));
        self.eject_cell(cx, cy, c, vx, vy)
    }

    /// **G→P 通路本身**（spec §6.6）：把一格网格内容变成一颗粒子——置 AIR +
    /// 盖当前戳，并往本 chunk 的生成缓冲追加请求。M3 刚体撞击、M4 法术冲量
    /// 届时加一个 Op 分支调它即可，不预建接口。
    ///
    /// 先 push 后置 AIR：本地限流拒绝时 cell 原样留在网格里（照旧停住）。
    ///
    /// ⚠️ **质量守恒缺口**：脱格已把网格置 air，若该请求随后被
    /// `Particles::spawn` 的全局容量（`MAX_PARTICLES`）确定性拒绝，这份质量
    /// 就凭空消失了。这与 M1 爆炸路径是同一个**已知**权衡（`explode.rs` 同款
    /// 就地注释），两端行为一致故不破坏确定性；真要补，得在粒子层加"拒绝即
    /// 回填网格"的反向通路，那是 M2 之后的事。
    fn eject_cell(&self, x: i32, y: i32, c: Cell, vx: Fx, vy: Fx) -> bool {
        let req = SpawnRequest {
            material: c.material(),
            x: Fx::from_int(x) + HALF_CELL,
            y: Fx::from_int(y) + HALF_CELL,
            vx,
            vy,
        };
        if !self.win.push_spawn(req) {
            return false;
        }
        self.win.set(x, y, Cell::AIR.with_stamp(self.stamp));
        true
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

    /// 斜向偏好掷骰。`attempt` = 子步序号 `k`（Layer G Task 2，spec §4.2③）——
    /// charter §11 翻案 4 点名要求保留的维度：同一 cell 同一 tick 内的多次
    /// 掷骰必须带不同参数，否则 4 个子步会全部滑向同一侧。`k = 0` 时取值与
    /// Task 2 之前相同，故不破坏可退化性。
    fn diag_side(&self, x: i32, y: i32, k: u32) -> i32 {
        if rng_u32(self.fseed, STREAM_DIAG, x, y, 0, k) & 1 == 0 { 1 } else { -1 }
    }

    fn powder_step(&self, x: i32, y: i32, c: Cell, k: u32) -> Step {
        if self.displace(x, y, c, x, y + 1) {
            return Step::Moved(x, y + 1);
        }
        let s = self.diag_side(x, y, k);
        if self.displace(x, y, c, x + s, y + 1) {
            return Step::Moved(x + s, y + 1);
        }
        if self.displace(x, y, c, x - s, y + 1) {
            return Step::Moved(x - s, y + 1);
        }
        Step::Blocked
    }

    fn liquid_step(&self, x: i32, y: i32, c: Cell, k: u32) -> Step {
        if self.displace(x, y, c, x, y + 1) {
            return Step::Moved(x, y + 1);
        }
        let s = self.diag_side(x, y, k);
        if self.displace(x, y, c, x + s, y + 1) {
            return Step::Moved(x + s, y + 1);
        }
        if self.displace(x, y, c, x - s, y + 1) {
            return Step::Moved(x - s, y + 1);
        }
        // 横移至多 dispersion 格（Layer G Task 1），仅入 AIR；方向承诺不变量：
        // 侧移成功后记忆 = 实际移动方向（2026-06-14 液面冻结修复的 Rust 版语义，
        // M0 spec §4.3）。失败则翻向再试一次——翻向后同样吃满色散距离。
        let d = c.dir();
        if let Some(nx) = self.side(x, y, c, d) {
            return Step::MovedSide(nx, y);
        }
        if let Some(nx) = self.side(x, y, c, -d) {
            return Step::MovedSide(nx, y);
        }
        Step::Blocked
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
    /// 成功则返回落点的 x（Layer G Task 2 起，外层子步循环需要落点坐标）。
    fn side(&self, x: i32, y: i32, c: Cell, d: i32) -> Option<i32> {
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
            return None;
        }
        // 方向承诺不变量：记忆 = 实际移动方向（2026-06-14 液面冻结修复语义）
        self.win.set(far, y, c.with_dir(d > 0).with_stamp(self.stamp));
        self.win.set(x, y, far_cell.with_stamp(self.stamp));
        Some(far)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::{VEL_ONE, V_MAX_CELL};
    use crate::rng::frame_seed;

    /// v1 ≤ 1.0 格/tick 时子步数恒为 1——`max(1, ..)` 的下限，也是
    /// 可退化性（spec §4.2①）的算术根据。
    #[test]
    fn substeps_is_one_below_one_cell_per_tick() {
        let f = frame_seed(42, 7);
        for v1 in 0..=VEL_ONE {
            for x in 0..64i32 {
                assert_eq!(substeps(f, v1, x, 3), 1, "v1={v1} x={x} 应恒为 1 子步");
            }
        }
    }

    /// 终端速度下恰好 4 子步（frac = 0，无概率成分）。
    #[test]
    fn substeps_at_terminal_speed_is_exactly_four() {
        let f = frame_seed(42, 7);
        for x in 0..64i32 {
            assert_eq!(substeps(f, V_MAX_CELL, x, 3), (V_MAX_CELL / VEL_ONE) as u32);
        }
    }

    /// 概率取整：v1 = 1.5 格/tick（6 个 ¼ 格单位，frac = 2/4）⇒ 子步数只能是
    /// 1 或 2，且长期比例约 50%。VEL_ONE 是 2 的幂 ⇒ `% VEL_ONE` 无取模偏置。
    #[test]
    fn substeps_probabilistic_rounding_matches_fraction() {
        let f = frame_seed(0x00C0_FFEE, 11);
        let (mut twos, mut n) = (0u32, 0u32);
        for y in 0..128i32 {
            for x in 0..128i32 {
                let s = substeps(f, 6, x, y);
                assert!(s == 1 || s == 2, "v1=6 的子步数越界：{s}");
                if s == 2 {
                    twos += 1;
                }
                n += 1;
            }
        }
        let p = twos as f64 / n as f64;
        // n = 16384 ⇒ σ = 0.5/√n ≈ 0.39%，取 4σ ≈ 1.6%
        assert!((p - 0.5).abs() < 0.016, "frac=2/4 的取整比例 {p:.4} 偏离 0.5 超 4σ");
    }

    /// 定点常量的金值（手写位模式，必须与 `from_ratio` 一致——写错一位就是
    /// 全场水花速度错一倍，且不会有任何测试自然发现）。
    #[test]
    fn splash_fixed_point_constants_match_their_ratios() {
        assert_eq!(SPLASH_RESTITUTION, Fx::from_ratio(1, 2));
        assert_eq!(SPLASH_JITTER, Fx::from_ratio(1, 2));
    }

    /// 阈值速度下的竖直初速减去最大抖动后**仍然向上**——否则"溅射"会有一部分
    /// 粒子朝下钻进地里，那不是水花是穿模。
    #[test]
    fn splash_stays_upward_even_at_worst_case_jitter() {
        let speed = vel_to_fx(SPLASH_MIN_SPEED).mul(SPLASH_RESTITUTION);
        assert!(speed > SPLASH_JITTER, "阈值速度 {speed:?} 必须大于抖动幅度 {SPLASH_JITTER:?}");
        // 抖动闭区间上界即 +SPLASH_JITTER（emit_jitter 的两端可达性已有单测）
        assert!((-speed + SPLASH_JITTER).0 < 0, "最坏抖动下溅射粒子仍须向上");
    }

    /// 纯函数：同 (fseed, v1, x, y) 必复现同值（charter §2 随机性法典）。
    #[test]
    fn substeps_is_a_pure_function() {
        let f = frame_seed(1, 2);
        assert_eq!(substeps(f, 7, 13, 21), substeps(f, 7, 13, 21));
    }
}
