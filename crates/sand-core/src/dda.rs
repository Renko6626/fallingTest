//! DDA 网格穿越（M1 spec §5，逐句执行）：从 `pos` 到 `pos + vel`，按跨越边界顺序
//! 逐格检查；边界跨越比较用 i64 交叉相乘，全程无除法。
//!
//! **只检查离开起点格之后进入的格子**——起点格本身永不检查（spec §5 明文）。
//! 起点格完全可能非 air：例如某粒子这一 tick 速度太小、`pos+vel` 没跨出本格
//! （零穿越，见下方 `step==0`/`vel==(0,0)` 情形），而网格四相在本 tick 早些
//! 时候恰好把材料移进了这一格——粒子仍在其中"漂浮"一瞬，下一 tick DDA 必须
//! 允许它从此处正常出发，不能因为起点非 air 就误判为立即阻挡（Task 4 修复轮
//! 1 C1 教训：起点豁免与落格冲突消解的交互一旦想当然会活锁，见
//! `particle.rs::resolve_landing` 文档）。
//!
//! 出界与阻挡是两个不同结局：世界边界外一律 [`Trace::Gone`]（哪怕沿途一直在
//! 网格内，只是终点越界也算出界，不算阻挡）；阻挡专指网格内的非 air 格
//! （wall/sand/water 一视同仁，spec §5：“第一个非 air 格即阻挡”）。
//!
//! # 算法（标准整数网格穿越，Amanatides–Woo 变体，无除法版）
//!
//! 对每根轴独立维护「到下一条网格线的剩余距离」`rem`（raw Q16.16 单位）与
//! 「本轴总位移」`total = |v|`（同单位）。下一次跨越发生在哪根轴，等价于比较
//! 两个分数 `remX/totalX` 与 `remY/totalY` 的大小——**用 i64 交叉相乘代替除法**：
//! `remX*totalY` 与 `remY*totalX` 比较大小即可，避免任何运行时除法链
//! （唯一允许的除法点是 `fixed.rs::from_ratio` 的常量构造，此处不碰）。
//! 每跨一次该轴的格线，`rem` 累加一整格（`CELL_RAW = 1<<16`），如此往复直到
//! 格坐标到达终点格 `to_cell(pos+vel)`。
//!
//! 单轴速度为 0 时该轴永不参与比较（`step == 0` 直接从候选中剔除，不产生
//! `0/0` 或除零问题）；两轴都为 0（`vel == (0,0)`）时起点格即终点格，循环体
//! 不执行，直接返回 `Clear`。

use crate::fixed::Fx;
use crate::material::MAT_AIR;
use crate::world::World;

/// 一格的原始定点宽度（Q16.16：`1 << 16`）。
const CELL_RAW: i64 = 1 << 16;

/// DDA 穿越结局。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Trace {
    /// 命中阻挡：候选落格 = 阻挡格前最后一个 air 格（可能是起点格本身）。
    Blocked { land_cell: (i32, i32) },
    /// 全程无阻挡，终点未越界：`end_pos = pos + vel`（连续坐标，非格心）。
    Clear { end_pos: (Fx, Fx) },
    /// 终点格越界（世界边界外，不算阻挡——spec §5 明文）。
    Gone,
}

/// 单轴穿越状态：`step` = 该轴格坐标步进方向（-1/0/+1）；`rem` = 到下一条待跨
/// 边界的剩余距离（raw，i64，非负）；`total` = 本轴速度大小（raw，i64）。
/// `step == 0` 时 `rem`/`total` 恒为 0，且从不参与跨越比较（见 [`x_crosses_first`]）。
struct AxisState {
    step: i32,
    rem: i64,
    total: i64,
}

fn sign(v: Fx) -> i32 {
    if v.0 > 0 {
        1
    } else if v.0 < 0 {
        -1
    } else {
        0
    }
}

/// 初始化单轴状态：`p` 为该轴起点坐标，`v` 为该轴速度分量。
fn axis_init(p: Fx, v: Fx) -> AxisState {
    let step = sign(v);
    if step == 0 {
        return AxisState { step: 0, rem: 0, total: 0 };
    }
    let cell = (p.to_cell()) as i64;
    // 沿运动方向的下一条格线：+方向是本格右/下边界，-方向是本格左/上边界
    // （若恰好已站在该边界上，-方向的 rem 会是 0——意味着立即跨越，这是对的：
    // 恰好贴着左边界向左走，下一步就已经进入左邻格）。
    let bound = if step > 0 { (cell + 1) * CELL_RAW } else { cell * CELL_RAW };
    let rem = if step > 0 { bound - p.0 as i64 } else { p.0 as i64 - bound };
    let total = (v.0 as i64).abs();
    AxisState { step, rem, total }
}

/// 下一次跨越先发生在 x 轴还是 y 轴（true = x）。恰好同时到达两条边界（对角线
/// 穿过格角）时钦定 x 先跨——确定性 tie-break，不影响正确性只影响遍历序，
/// 该分支由 `diagonal_tie_break_is_deterministic` 单测钉死。
fn x_crosses_first(ax: &AxisState, ay: &AxisState) -> bool {
    match (ax.step != 0, ay.step != 0) {
        (true, false) => true,
        (false, true) => false,
        (false, false) => unreachable!("双轴都静止时调用方循环体不应执行（起点=终点）"),
        (true, true) => ax.rem * ay.total <= ay.rem * ax.total,
    }
}

/// 纯几何格序列迭代器（**不含起点格**）：从 `pos` 出发按 `vel` 逐次跨越格线
/// （与 [`trace`] 同一套 i64 交叉相乘算法），直到到达终点格
/// `to_cell(pos+vel)`（含终点格，作为最后一个产出项）。不做任何越界/材质
/// 判断——那是各调用方自己的语义。供 [`trace`]（粒子阻挡语义）与爆炸射线
/// （能量语义，`world.rs::World::apply_op` 的 `Op::Explode` 分支）共享底层
/// 穿越算法（M1 Task 6：避免在 world.rs 里重新实现一遍同款轴跨越逻辑）。
/// 双轴都静止（`vel == (0,0)`）时起点=终点，`next()` 立即返回 `None`。
pub(crate) struct CellWalk {
    cx: i32,
    cy: i32,
    target_cx: i32,
    target_cy: i32,
    ax: AxisState,
    ay: AxisState,
}

impl CellWalk {
    pub(crate) fn new(pos: (Fx, Fx), vel: (Fx, Fx)) -> CellWalk {
        let end = (pos.0 + vel.0, pos.1 + vel.1);
        CellWalk {
            cx: pos.0.to_cell(),
            cy: pos.1.to_cell(),
            target_cx: end.0.to_cell(),
            target_cy: end.1.to_cell(),
            ax: axis_init(pos.0, vel.0),
            ay: axis_init(pos.1, vel.1),
        }
    }
}

impl Iterator for CellWalk {
    type Item = (i32, i32);

    fn next(&mut self) -> Option<(i32, i32)> {
        if (self.cx, self.cy) == (self.target_cx, self.target_cy) {
            return None;
        }
        if x_crosses_first(&self.ax, &self.ay) {
            self.cx += self.ax.step;
            self.ax.rem += CELL_RAW;
        } else {
            self.cy += self.ay.step;
            self.ay.rem += CELL_RAW;
        }
        Some((self.cx, self.cy))
    }
}

/// 从 `pos` 到 `pos + vel` 的 DDA 穿越（spec §5）。`world` 是本 tick 网格终态的
/// 只读快照——纯函数，同输入同输出，任意线程数/调度顺序不影响结果。
pub(crate) fn trace(pos: (Fx, Fx), vel: (Fx, Fx), world: &World) -> Trace {
    let end = (pos.0 + vel.0, pos.1 + vel.1);
    // 初值 = 起点格。起点格从不检查（本文件顶部注释），所以这个初值完全可能
    // 本身就是非 air（见顶部注释的零穿越场景）。若离开起点后第一步就撞阻挡，
    // `Blocked` 会原样把这个非 air 的起点格当候选返回；调用方
    // （`particle::resolve_landing`）因此不能假设候选默认可落，必须重新判定
    // 候选本身是否 air——这正是 Task 4 修复轮 1 C1 活锁的根源之一（另一半在
    // 旧版"全占转悬浮"路径，已废除，见 `particle.rs::resolve_landing` 文档）。
    let mut last_air = (pos.0.to_cell(), pos.1.to_cell());

    // 安全上限：MAX_SPEED=16 时单轴最坏跨越数 ~17，双轴交织最坏 ~34；
    // 256 留出充裕余量，纯粹是回归防线（算法本身有界，触发即视为 bug）。
    const MAX_STEPS: u32 = 256;
    let mut steps = 0u32;

    for (cx, cy) in CellWalk::new(pos, vel) {
        steps += 1;
        debug_assert!(steps <= MAX_STEPS, "DDA 步数超出安全上限，检查速度 clamp 或算法");
        if !world.in_bounds(cx, cy) {
            return Trace::Gone;
        }
        if world.cell(cx, cy).material() != MAT_AIR {
            return Trace::Blocked { land_cell: last_air };
        }
        last_air = (cx, cy);
    }
    Trace::Clear { end_pos: end }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::{Category, MaterialDef, MaterialTable};

    const WALL: u8 = 1;

    fn table() -> MaterialTable {
        MaterialTable::new(vec![
            MaterialDef { id: 0, name: "air".into(), category: Category::Static, density: 0, color: (0, 0, 0), blast_cost: 0, vaporize_threshold: 255, dispersion: 1, splash_chance: 0 },
            MaterialDef { id: 1, name: "wall".into(), category: Category::Static, density: 100, color: (0, 0, 0), blast_cost: crate::material::BLAST_COST_INFINITE, vaporize_threshold: 255, dispersion: 1, splash_chance: 0 },
        ])
        .unwrap()
    }

    fn world_with_walls(wc: usize, hc: usize, walls: &[(i32, i32)]) -> World {
        let mut w = World::new(wc, hc, 0);
        let t = table();
        for &(x, y) in walls {
            w.set_cell_stamped(&t, x, y, WALL, 0);
        }
        w
    }

    fn fx(v: i32) -> Fx {
        Fx::from_int(v)
    }

    // ---- 无阻挡：水平/垂直/对角穿越 ----

    #[test]
    fn horizontal_clear_traversal() {
        let w = world_with_walls(2, 2, &[]);
        let r = trace((fx(5), fx(5)), (fx(10), Fx::ZERO), &w);
        assert_eq!(r, Trace::Clear { end_pos: (fx(15), fx(5)) });
    }

    #[test]
    fn vertical_clear_traversal() {
        let w = world_with_walls(2, 2, &[]);
        let r = trace((fx(5), fx(5)), (Fx::ZERO, fx(8)), &w);
        assert_eq!(r, Trace::Clear { end_pos: (fx(5), fx(13)) });
    }

    #[test]
    fn diagonal_clear_traversal() {
        let w = world_with_walls(3, 3, &[]);
        let r = trace((fx(5), fx(5)), (fx(6), fx(6)), &w);
        assert_eq!(r, Trace::Clear { end_pos: (fx(11), fx(11)) });
    }

    // ---- 阻挡停点：候选 = 阻挡格前最后一个 air 格 ----

    #[test]
    fn horizontal_blocked_stops_before_wall() {
        let w = world_with_walls(2, 2, &[(20, 5)]);
        let r = trace((fx(15), fx(5)), (fx(10), Fx::ZERO), &w);
        assert_eq!(r, Trace::Blocked { land_cell: (19, 5) });
    }

    #[test]
    fn vertical_blocked_stops_before_wall() {
        let w = world_with_walls(2, 2, &[(5, 20)]);
        let r = trace((fx(5), fx(10)), (Fx::ZERO, fx(15)), &w);
        assert_eq!(r, Trace::Blocked { land_cell: (5, 19) });
    }

    #[test]
    fn diagonal_blocked_stops_at_correct_cell_in_crossing_order() {
        // 从格心 (5,5) 出发，速度 (3,3)：非对角起点分数相同——tie 每次都选 x 先跨
        // （见 diagonal_tie_break_is_deterministic），故穿越序为
        // (6,5)(6,6)(7,6)(7,7)(8,7)(8,8)。在 (7,7) 放墙，候选应停在 (7,6)。
        let w = world_with_walls(3, 3, &[(7, 7)]);
        let start = (Fx((5 << 16) + 0x8000), Fx((5 << 16) + 0x8000)); // 格心，起点分数对称
        let r = trace(start, (fx(3), fx(3)), &w);
        assert_eq!(r, Trace::Blocked { land_cell: (7, 6) });
    }

    #[test]
    fn immediate_block_candidate_is_start_cell() {
        // 起点格右邻即墙：第一次跨越就阻挡，候选 = 起点格本身。
        let w = world_with_walls(2, 2, &[(6, 5)]);
        let r = trace((fx(5), fx(5)), (fx(3), Fx::ZERO), &w);
        assert_eq!(r, Trace::Blocked { land_cell: (5, 5) });
    }

    // ---- 起点格豁免：起点格本身即使非 air 也不检查 ----

    #[test]
    fn start_cell_is_never_checked_even_if_blocking() {
        // 粒子可能就位于非 air 格（本文件顶部注释：零穿越 tick + 网格四相
        // 同 tick 移料进格）；起点是墙，但只要离开后进入的格子是 air，就必须
        // 正常 Clear，不能因起点非 air 而误判阻挡。
        let w = world_with_walls(2, 2, &[(5, 5)]);
        let r = trace((fx(5), fx(5)), (fx(3), Fx::ZERO), &w);
        assert_eq!(r, Trace::Clear { end_pos: (fx(8), fx(5)) });
    }

    // ---- 出界 vs 阻挡：越界不算阻挡 ----

    #[test]
    fn out_of_bounds_target_is_gone_not_blocked() {
        // 2×2 chunk 世界宽 128；从 x=120 出发飞出右边界，沿途全是 air。
        let w = world_with_walls(2, 2, &[]);
        let r = trace((fx(120), fx(5)), (fx(20), Fx::ZERO), &w);
        assert_eq!(r, Trace::Gone);
    }

    #[test]
    fn wall_encountered_before_boundary_is_blocked_not_gone() {
        // 沿途先撞墙，即使墙之后紧邻世界边界，判定仍是 Blocked（先发生的先算数）。
        let w = world_with_walls(2, 2, &[(125, 5)]);
        let r = trace((fx(120), fx(5)), (fx(20), Fx::ZERO), &w);
        assert_eq!(r, Trace::Blocked { land_cell: (124, 5) });
    }

    // ---- 零速与 tie-break ----

    #[test]
    fn zero_velocity_is_clear_at_same_pos() {
        let w = world_with_walls(2, 2, &[]);
        let r = trace((fx(5), fx(5)), (Fx::ZERO, Fx::ZERO), &w);
        assert_eq!(r, Trace::Clear { end_pos: (fx(5), fx(5)) });
    }

    #[test]
    fn diagonal_tie_break_is_deterministic() {
        // 格心出发、等速对角线：连续两次调用（乃至反复调用）必须给出完全相同结果——
        // tie-break 是纯函数决定，不依赖任何外部状态。
        let w = world_with_walls(4, 4, &[]);
        let start = (Fx((1 << 16) + 0x8000), Fx((1 << 16) + 0x8000));
        let a = trace(start, (fx(2), fx(2)), &w);
        let b = trace(start, (fx(2), fx(2)), &w);
        assert_eq!(a, b);
        assert_eq!(a, Trace::Clear { end_pos: (Fx((3 << 16) + 0x8000), Fx((3 << 16) + 0x8000)) });
    }

    // ---- 负方向：左/上运动的边界符号处理 ----

    #[test]
    fn leftward_blocked_stops_before_wall() {
        let w = world_with_walls(2, 2, &[(10, 5)]);
        let r = trace((fx(15), fx(5)), (fx(-10), Fx::ZERO), &w);
        assert_eq!(r, Trace::Blocked { land_cell: (11, 5) });
    }

    #[test]
    fn upward_clear_traversal() {
        let w = world_with_walls(2, 2, &[]);
        let r = trace((fx(5), fx(15)), (Fx::ZERO, fx(-8)), &w);
        assert_eq!(r, Trace::Clear { end_pos: (fx(5), fx(7)) });
    }

    #[test]
    fn negative_diagonal_blocked() {
        // 左上方向对角线：从 (10,10) 走向 (7,7)，(8,8) 放墙。
        // 手工推演穿越序（tie 定 x 先跨）：(9,10)(9,9)(8,9)(8,8)==墙；
        // 候选 = 撞墙前最后一个 air 格 = (8,9)（M4：钉死具体坐标而非范围断言）。
        let w = world_with_walls(3, 3, &[(8, 8)]);
        let r = trace((fx(10), fx(10)), (fx(-3), fx(-3)), &w);
        assert_eq!(r, Trace::Blocked { land_cell: (8, 9) });
    }

    // ---- CellWalk（M1 Task 6：爆炸射线共享的公共 helper）----

    #[test]
    fn cell_walk_matches_trace_crossing_sequence_exactly() {
        // CellWalk 是 trace() 内部循环体的直接抽取——用对角线穿越序反推验证
        // 抽取没有改变任何步进语义（diagonal_blocked_stops_at_correct_cell_in_crossing_order
        // 手工推演的同一条穿越序）。
        let start = (Fx((5 << 16) + 0x8000), Fx((5 << 16) + 0x8000));
        let seq: Vec<(i32, i32)> = CellWalk::new(start, (fx(3), fx(3))).collect();
        assert_eq!(seq, vec![(6, 5), (6, 6), (7, 6), (7, 7), (8, 7), (8, 8)]);
    }

    #[test]
    fn cell_walk_excludes_start_cell_and_includes_end_cell() {
        let seq: Vec<(i32, i32)> = CellWalk::new((fx(5), fx(5)), (fx(3), Fx::ZERO)).collect();
        assert_eq!(seq, vec![(6, 5), (7, 5), (8, 5)], "不含起点格 (5,5)，含终点格 (8,5)");
    }

    #[test]
    fn cell_walk_zero_velocity_yields_empty_sequence() {
        let seq: Vec<(i32, i32)> = CellWalk::new((fx(5), fx(5)), (Fx::ZERO, Fx::ZERO)).collect();
        assert!(seq.is_empty(), "起点即终点，序列为空");
    }

    #[test]
    fn cell_walk_is_deterministic_across_repeated_calls() {
        let start = (Fx((2 << 16) + 0x8000), Fx((2 << 16) + 0x8000));
        let vel = (fx(9), fx(4));
        let a: Vec<(i32, i32)> = CellWalk::new(start, vel).collect();
        let b: Vec<(i32, i32)> = CellWalk::new(start, vel).collect();
        assert_eq!(a, b);
    }
}
