//! `Op::Explode`（spec §6，Noita 射线模型，M1 Task 6）——从 `world.rs` 纯
//! 搬移（M1 收口，2026-08-30，一行逻辑未改）：`circle_offsets`/`fire_ray`/
//! `ray_fluct`/`explode_attempt`、`EXPLODE_*` 全部常量、`REF_BLAST_DENSITY`
//! 与 `apply_explode`。

use crate::dda::CellWalk;
use crate::emit::emit_jitter;
use crate::fixed::{isqrt, Fx, HALF_CELL};
use crate::material::{MaterialTable, MAT_AIR};
use crate::particle::clamp_speed;
use crate::rng;
use crate::world::{SpawnRequest, World};

/// `Op::Explode` 里 vx 抖动用的骰子标号，语义与 `EMIT_ROLL_VX` 相同——两者
/// 数值巧合相等（都是 0/1）不代表可以共用常量：`Op::Emit`/`Op::Explode` 是
/// 两个不同的调用点，各自的 `attempt` 编码独立演化，未来任一方改动骰子数量
/// 不该牵连另一方。
const EXPLODE_ROLL_VX: u32 = 0;
/// `Op::Explode` 里 vy 抖动用的骰子标号（同上，见 [`EXPLODE_ROLL_VX`]）。
const EXPLODE_ROLL_VY: u32 = 1;
/// 每射线能量涨落骰（2026-08-30 用户裁决"方向相关涨落"：完美圆坑不自然）。
const EXPLODE_ROLL_RAY_POWER: u32 = 2;
/// 每射线射程涨落骰。
const EXPLODE_ROLL_RAY_RANGE: u32 = 3;

/// `Op::Explode` 专用 attempt 编码：`stamp` 占高位、骰子标号占**低 2 位**
/// （射线涨落骰加入后 Explode 有 4 颗骰）。与 `emit::emit_attempt`（低 1 位）
/// 分道扬镳正是 [`EXPLODE_ROLL_VX`] 文档预言的"任一方改动骰子数量不该
/// 牵连另一方"——本次扩位只作废爆炸场景的 RNG 序列，Emit（瀑布）不动。
fn explode_attempt(stamp: u8, roll: u32) -> u32 {
    ((stamp as u32) << 2) | roll
}

/// 射线涨落幅度分母：涨落量 = `v / EXPLODE_FLUCT_DIV`（即 ±25% 或 −25%）。
const EXPLODE_FLUCT_DIV: u32 = 4;

/// 确定性整数涨落映射（乘移法，同 `emit::emit_jitter` 的数学，作用在 u32 上）：
/// - `sym = true`：返回 `v + d`，`d ∈ [-q, +q]`（能量：方向间可强可弱）；
/// - `sym = false`：返回 `v + d`，`d ∈ [-q, 0]`（射程：只缩不涨——
///   `CellWalk` 的终点是圆周格，无法越过目标延长，故射程涨落取单边）。
///
/// 其中 `q = v / EXPLODE_FLUCT_DIV`；结果下限 1（防零能量/零射程退化）。
fn ray_fluct(v: u32, roll: u32, sym: bool) -> u32 {
    let q = (v / EXPLODE_FLUCT_DIV) as u64;
    let span = if sym { 2 * q + 1 } else { q + 1 };
    let d = ((roll as u64).wrapping_mul(span) >> 32) as i64 - q as i64;
    ((v as i64) + d).max(1) as u32
}

/// 溅射速度抖动幅度（spec §6 point 3"调参项"）：`Fx::from_ratio(1, 2)`
/// 的位模式（写成字面量是因为 `from_ratio` 非 `const fn`，理由同
/// `particle.rs::GRAVITY`；`explode_jitter_matches_from_ratio_one_half`
/// 单测钉死等价性）。复用 `emit::emit_jitter` 做区间映射——同一套抖动数学，
/// 只是幅度常量与调用点（stream/salt/attempt）不同。
const EXPLODE_JITTER: Fx = Fx(0x0000_8000);

/// 爆炸出射速度上限（调参项，2026-08-30 用户目检裁决"粒子更重"：16→8）——
/// 溅射速度 = `EXPLODE_SPEED × 剩余能量/power`。与 `particle.rs::MAX_SPEED`
/// （飞行 clamp 上限）解耦：前者管"炸得多猛"的手感，后者是 DDA 步数上界的
/// 数值纪律，不随手感调。位模式 = `Fx::from_int(8)`（`from_int` 非 `const fn`，
/// `explode_speed_matches_from_int_eight` 单测钉死等价性）。
const EXPLODE_SPEED: Fx = Fx(8 << 16);

/// 爆炸冲量的参考密度（2026-08-30 用户裁决"冲量物理"：同一冲量下
/// v ∝ 1/密度）。出射速度按 `参考密度/材质密度` 缩放；取沙的密度 40 为
/// 参考 → 沙的系数恒为 1（手感锚点不动），水（密度 16）系数 2.5（受
/// `clamp_speed` 封顶 ±MAX_SPEED）。密度取 `max(1)` 防御除零（air 不会
/// 走到溅射路径，纯防御）。
const REF_BLAST_DENSITY: i32 = 40;

/// Bresenham 圆周（半径 `r` 格，圆心偏移量）：返回圆心到每个周长格的整数
/// 偏移 `(dx, dy)`，**定序、无重复**（spec §6 point 1 + §10 单测
/// `explode_circle_offsets_is_stable_and_has_no_duplicates`）。
///
/// 算法：经典 Bresenham/midpoint 圆算法（决策变量 `d = 3 - 2r`，八分圆
/// `0 <= x <= y` 递推），每步产出的八分圆点 `(a, b)` 按固定顺序展开成全圆的
/// 8 个镜像点：
/// `[(a,b), (b,a), (-b,a), (-a,b), (-a,-b), (-b,-a), (b,-a), (a,-b)]`。
///
/// **确定性论证**：递推变量 `x/y/d` 全整数、无浮点、无随机源，同一 `r` 每次
/// 调用产出完全相同的序列（纯算术，不依赖遍历顺序以外的任何状态）。
///
/// **无重复论证**：八分圆递推里 `y` 严格单调递减、`x` 单调不减（标准
/// Bresenham 圆性质），故每步的 `(a, b)` 互不相同；对不同的 `(a,b)`，其 8
/// 个镜像点集合两两不相交——除非 `(a1,b1)` 与 `(a2,b2)` 互为交换
/// （`a1=b2 且 b1=a2`），但循环全程维持 `a<=b`，只有 `a=b`（对角线）才可能
/// 自交换，那正是同一步内部的退化情形。步内去重（[`push_octant_mirror`]）
/// 用长度 ≤8 的线性 `contains` 扫描——退化情形（`a=0` 轴上或 `a=b` 对角线）
/// 产出的重复索引在候选数组里不保证相邻（`a=0` 时重复出现在 idx0/idx3 而非
/// 相邻位置，手工验证过），故不能只比较"相邻 + 首尾折返"，必须是全量成员
/// 检测；`Vec` 而非 `HashSet` 承载（候选数不超过 8，线性扫描足够快，也不
/// 触碰"禁 std HashSet 默认 hasher"红线）。
///
/// `r <= 0` 特例：圆退化为圆心一点，返回 `vec![(0, 0)]`（半径为 0 的"爆炸"
/// 只处理爆心格自身，见 [`fire_ray`] 起点格计费口径）。
pub(crate) fn circle_offsets(r: i32) -> Vec<(i32, i32)> {
    if r <= 0 {
        return vec![(0, 0)];
    }
    let mut offsets = Vec::new();
    let mut x: i32 = 0;
    let mut y: i32 = r;
    let mut d: i32 = 3 - 2 * r;
    while y >= x {
        push_octant_mirror(&mut offsets, x, y);
        x += 1;
        if d > 0 {
            y -= 1;
            d += 4 * (x - y) + 10;
        } else {
            d += 4 * x + 6;
        }
    }
    offsets
}

/// 单个八分圆点 `(a, b)` 展开为全圆最多 8 个镜像点，去重（退化情形：
/// `a == 0` 或 `a == b` 时实际只有 4 个不同点，且重复项在候选数组里未必
/// 相邻——`a == 0` 时是 idx0 与 idx3 重复，见 [`circle_offsets`] 文档的
/// 手工验证，故用全量 `contains` 而非相邻比较）。展开顺序固定（先出现者
/// 保留），是圆周格的最终遍历序。
fn push_octant_mirror(offsets: &mut Vec<(i32, i32)>, a: i32, b: i32) {
    let candidates: [(i32, i32); 8] =
        [(a, b), (b, a), (-b, a), (-a, b), (-a, -b), (-b, -a), (b, -a), (a, -b)];
    let mut group: Vec<(i32, i32)> = Vec::with_capacity(8);
    for &p in &candidates {
        if !group.contains(&p) {
            group.push(p);
        }
    }
    offsets.extend(group);
}

/// 单条爆炸射线（spec §6 point 2/3）：从圆心 `(cx, cy)` 出发，沿方向
/// `(dx, dy)` 逐格消耗能量，能量 ≥ 格消耗即摧毁（置 air + 溅射粒子，或按
/// `MaterialTable::vaporize_threshold` 判定汽化——置 air、不溅射，见下方
/// "近心汽化"节），能量不足或撞 `BLAST_COST_INFINITE` 材料即断线。
///
/// **爆心格自身的口径**（任务书 + spec §6 point 1"起点格按第一格计费处理"）：
/// `(cx, cy)` 本身作为该射线的第一格纳入能量结算——与 [`CellWalk`]"不含
/// 起点格"的粒子 DDA 语义相反，这里手动 `std::iter::once((cx, cy))` 前置，
/// 再接 `CellWalk` 产出的后续格。`r>=1` 时每条射线都独立从爆心起算，第一条
/// 处理到的射线摧毁爆心（若能量足够）；后续射线到达时爆心已是 air，
/// `blast_cost(air)=0` 恒满足、且已 air 不重复摧毁/不重复溅射（spec §6
/// point 4），对能量预算而言等同免费经过。
///
/// **能量衰减用于溅射速度**：摧毁某格后取*该格消耗完成后的剩余能量*
/// （`remaining = energy_before - cost`）参与速度合成——"剩余能量/power"
/// 随射线深入单调递减，天然实现"爆心附近溅得快、边缘溅得慢"的线性衰减
/// （spec §6 point 3）。
///
/// **近心汽化**（`vaporize_threshold`，spec §6 汽化小节，用户裁决
/// 2026-08-30）：同一个 `remaining`（即上一段的剩余能量，两处共享同一变量、
/// 不做区分）若使比例 `remaining/power` 严格超过材质阈值，该格直接删除、
/// **不**产出 `SpawnRequest`——做出"近心没了、外圈飞溅"的观感，质量在此
/// 确定性蒸发（`World::vaporized_total` 计数，不入哈希）。
///
/// `salt = op_idx`（充分性论证见 `rng.rs::STREAM_EXPLODE` 文档：坐标本身
/// `(gx, gy)` 已经是"一次 Explode 应用内至多摧毁一次"的天然唯一键，
/// `op_idx` 只需区分同 tick 内的不同 `Op::Explode`）。
#[allow(clippy::too_many_arguments)]
fn fire_ray(
    world: &mut World,
    table: &MaterialTable,
    cx: i32,
    cy: i32,
    dx: i32,
    dy: i32,
    power: u32,
    // 本射线最多走过的格数（含爆心格；`u32::MAX` = 不设限，走满到圆周目标）。
    // 涨落策略在 apply_explode（调用方）结算——fire_ray 保持"给定预算走射线"
    // 的纯原语，白盒测试不受涨落干扰。
    max_cells: u32,
    stamp: u8,
    fseed: u32,
    op_idx: usize,
    spawns: &mut Vec<SpawnRequest>,
) {
    debug_assert!(power != 0, "fire_ray 要求 power != 0（调用方 apply_explode 已在判零）");
    let mag_sq = (dx as i64) * (dx as i64) + (dy as i64) * (dy as i64);
    let mag = isqrt(mag_sq as u64) as i32;
    let (unit_dx, unit_dy) =
        if mag == 0 { (Fx::ZERO, Fx::ZERO) } else { (Fx::from_ratio(dx, mag), Fx::from_ratio(dy, mag)) };

    let center = (Fx::from_int(cx) + HALF_CELL, Fx::from_int(cy) + HALF_CELL);
    let ray_cells = std::iter::once((cx, cy))
        .chain(CellWalk::new(center, (Fx::from_int(dx), Fx::from_int(dy))));

    let mut energy = power;
    let salt = op_idx as u32;
    for (cell_i, (gx, gy)) in ray_cells.enumerate() {
        if cell_i as u32 >= max_cells {
            break; // 射程涨落封顶（见 max_cells 参数注释）。
        }
        if !world.in_bounds(gx, gy) {
            break; // 出界断线，同"撞不可摧毁材料"一样直接停止该射线。
        }
        let material = world.cell(gx, gy).material();
        let cost = table.blast_cost(material);
        if energy < cost {
            break; // 能量不足以摧毁这一格（含 BLAST_COST_INFINITE 撞线）。
        }
        energy -= cost;
        if material == MAT_AIR {
            continue; // 已是 air（原生或已被前序射线炸掉）：计零费、不重复溅射。
        }

        // 近心汽化（vaporize_threshold，spec §6 汽化小节，用户裁决
        // 2026-08-30）：`energy` 此刻已完成 `cost` 扣减——这正是下面
        // `speed_ratio` 用的同一个"剩余能量"值，口径钉死不做区分（不存在
        // "扣费前"的候选口径：`fire_ray_vaporize_*` 单测锁定这一点）。比例
        // `energy/power` 一旦**严格超过**材质阈值 `threshold/255`，格子直接
        // 删除、不产出 `SpawnRequest`（质量确定性蒸发，`vaporized_total`
        // 计数，不入哈希）——纯整数比较避免除法：
        // `energy/power > threshold/255` 等价于 `energy*255 > power*threshold`
        // （`power != 0` 已由函数入口 `debug_assert` 保证，两侧同乘不改变
        // 不等号方向；两边最大约 `u32::MAX * 255 ≈ 1.1e12`，`i64` 内不会溢出）。
        // 严格大于是关键：`threshold=255`（RON 缺省 1.0）时条件退化为
        // `energy > power`，而 `energy <= power` 恒成立（`cost` 是无符号扣减），
        // 故缺省材质永不汽化，即便 `energy == power`（`cost == 0`）也不触发。
        let threshold = table.vaporize_threshold(material);
        if (energy as i64) * 255 > (power as i64) * (threshold as i64) {
            world.set_cell_stamped(table, gx, gy, MAT_AIR, stamp);
            world.vaporized_total += 1;
            continue; // 汽化：不生成粒子，跳过下面的速度合成与 spawn。
        }

        let speed_ratio = Fx::from_ratio(energy as i32, power as i32);
        // 冲量→速度按材质密度缩放（v ∝ 1/m，见 REF_BLAST_DENSITY）：
        // from_ratio 是确定性整数除法，每摧毁格一次，成本可忽略。
        let mass_factor =
            Fx::from_ratio(REF_BLAST_DENSITY, table.density(material).max(1) as i32);
        let speed_mag = EXPLODE_SPEED.mul(speed_ratio).mul(mass_factor);
        let rx = rng::rng_u32(fseed, rng::STREAM_EXPLODE, gx, gy, salt, explode_attempt(stamp, EXPLODE_ROLL_VX));
        let ry = rng::rng_u32(fseed, rng::STREAM_EXPLODE, gx, gy, salt, explode_attempt(stamp, EXPLODE_ROLL_VY));
        let vx = clamp_speed(unit_dx.mul(speed_mag) + emit_jitter(rx, EXPLODE_JITTER));
        let vy = clamp_speed(unit_dy.mul(speed_mag) + emit_jitter(ry, EXPLODE_JITTER));

        world.set_cell_stamped(table, gx, gy, MAT_AIR, stamp);
        spawns.push(SpawnRequest {
            material,
            x: Fx::from_int(gx) + HALF_CELL,
            y: Fx::from_int(gy) + HALF_CELL,
            vx,
            vy,
        });
    }
}

/// `Op::Explode` 分支体（从 `World::apply_op` 纯搬移）：圆周格定序遍历
/// （见 [`circle_offsets`] 文档的确定性/无重复论证），每格一条独立射线，
/// salt = op_idx（见 [`fire_ray`] 文档：坐标本身已是天然唯一键，op_idx
/// 只需区分同 tick 内不同 `Op::Explode`，charter §11 翻案 4 + Task 5 I1
/// 同款纪律）。
///
/// `power == 0`：没有能量可摧毁任何格（哪怕 blast_cost=0 的 air，"摧毁"
/// 逻辑本身不会为 air 触发），且 `fire_ray` 的 `Fx::from_ratio(energy,
/// power)` 除数不能为零——提前判零，语义上等价于"零能量爆炸 = 无操作"，
/// 不依赖 fire_ray 内部的分支顺序侥幸绕开除零。
///
/// **质量守恒缺口**（终审观察，非 bug）：`fire_ray` 对每个命中格先
/// `set_cell_stamped(.., MAT_AIR, ..)` 清格，再把同一份质量以
/// `SpawnRequest` 追加进 `spawns`；`spawns` 之后由调用方（`Sim::step`/
/// `apply_setup`）drain 进 `Particles::spawn`。若彼时粒子池已在
/// `MAX_PARTICLES` 上限，`spawn` 会确定性拒绝——格子已经变 air，粒子却没
/// 能生成，这份质量永久丢失（不返还、不回滚已清的格）。两端状态一致
/// （drain 序定序、拒绝条件是纯函数），不破坏确定性，但需知悉：拒绝事件
/// 计入 `Particles::rejected_total()`，可观测、可断言。
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_explode(
    world: &mut World,
    table: &MaterialTable,
    x: i32,
    y: i32,
    r: i32,
    power: u32,
    stamp: u8,
    fseed: u32,
    op_idx: usize,
    spawns: &mut Vec<SpawnRequest>,
) {
    if power != 0 {
        for (dx, dy) in circle_offsets(r) {
            // 方向相关涨落（2026-08-30 用户裁决：完美圆坑不自然）：
            // 每射线独立掷两骰——能量 ±25%、射程 −25%..0——key 锚点
            // 是射线方向 offset (dx,dy)（circle_offsets 保证每 op 内
            // 唯一），salt=op_idx 区分同 tick 多爆，attempt 用射线骰
            // 编码（explode_attempt，与逐格 vx/vy 骰不同码位）。
            // fire_ray 的 speed_ratio / 汽化比较均以涨落后的
            // ray_power 为分母——每条射线自洽如一条"额定功率不同"
            // 的正常射线，弱射线挖得浅、汽化圈也浅，坑沿与汽化边界
            // 一起毛糙。
            let salt = op_idx as u32;
            let rp = rng::rng_u32(fseed, rng::STREAM_EXPLODE, dx, dy, salt,
                explode_attempt(stamp, EXPLODE_ROLL_RAY_POWER));
            let rr = rng::rng_u32(fseed, rng::STREAM_EXPLODE, dx, dy, salt,
                explode_attempt(stamp, EXPLODE_ROLL_RAY_RANGE));
            let ray_power = ray_fluct(power, rp, true);
            // 射程按半径取单边涨落再 +1 含爆心格。
            let max_cells = ray_fluct(r as u32, rr, false) + 1;
            fire_ray(world, table, x, y, dx, dy, ray_power, max_cells, stamp, fseed, op_idx, spawns);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::{Category, MaterialDef, MaterialTable};
    use crate::world::Op;

    fn test_table() -> MaterialTable {
        // blast_cost 取 spec §6 的口径值（air 0 / water 1 / sand 2 / wall
        // 免疫），供本文件的 Op::Explode 测试直接复用；Emit 测试不关心该
        // 字段取值。
        use crate::material::BLAST_COST_INFINITE;
        // 255 = 永不汽化（base 缺省）：本表供 blast_cost/断线/守恒等既有行为
        // 测试复用，不应引入意料之外的汽化分支——专门测汽化差异的用例另建
        // 材料表（见"vaporize_threshold"分节）。
        let def = |id: u8, name: &str, category: Category, density: u16, blast_cost: u32| MaterialDef {
            blast_cost,
            ..MaterialDef::base(id, name, category, density)
        };
        MaterialTable::new(vec![
            def(0, "air", Category::Static, 0, 0),
            def(1, "wall", Category::Static, 100, BLAST_COST_INFINITE),
            def(2, "water", Category::Liquid, 16, 1),
            def(3, "sand", Category::Powder, 40, 2),
        ])
        .unwrap()
    }

    // ==================== circle_offsets：单测（M1 Task 6，spec §10）====================

    #[test]
    fn circle_offsets_r0_is_center_only() {
        assert_eq!(circle_offsets(0), vec![(0, 0)]);
        assert_eq!(circle_offsets(-3), vec![(0, 0)], "负半径按 0 处理");
    }

    #[test]
    fn circle_offsets_is_deterministic_across_repeated_calls() {
        for r in [1, 2, 3, 5, 8, 13, 20, 50] {
            assert_eq!(circle_offsets(r), circle_offsets(r), "r={r} 重复调用必须给出完全相同序列");
        }
    }

    #[test]
    fn circle_offsets_has_no_duplicate_cells() {
        for r in 1..=40 {
            let offs = circle_offsets(r);
            let mut sorted = offs.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), offs.len(), "r={r} 圆周格出现重复：{offs:?}");
        }
    }

    #[test]
    fn circle_offsets_every_point_within_one_cell_of_radius() {
        // Bresenham 圆是整数近似，允许 sqrt(dx²+dy²) 与 r 有 <1 格的栅格化
        // 误差，但不应离谱偏离（回归防线：算法写错通常表现为半径整体跑偏）。
        for r in [1, 5, 12, 30] {
            for (dx, dy) in circle_offsets(r) {
                let dist_sq = (dx as i64) * (dx as i64) + (dy as i64) * (dy as i64);
                let dist = isqrt(dist_sq as u64) as i64;
                assert!((dist - r as i64).abs() <= 1, "r={r} 的点 ({dx},{dy}) 距圆心 {dist}，偏离过大");
            }
        }
    }

    #[test]
    fn circle_offsets_r1_matches_four_axis_neighbors() {
        let mut offs = circle_offsets(1);
        offs.sort();
        let mut want = vec![(1, 0), (-1, 0), (0, 1), (0, -1)];
        want.sort();
        assert_eq!(offs, want);
    }

    // ==================== fire_ray：白盒（能量衰减 + 断线语义）====================

    #[test]
    fn fire_ray_speed_decays_monotonically_outward_along_axis() {
        // power=16、sand cost=2：每摧毁一格衰减 EXPLODE_SPEED*2/16=1.0 格/tick
        // 的"真值"速度，与 EXPLODE_JITTER 的最坏抖动摆幅（1.0 格/tick）持平，
        // 故单调性断言按**步长 2** 比较（真值差 2.0 > 抖动 1.0），不受噪声干扰。
        // （EXPLODE_SPEED 16→8 后逐格真值差减半，断言从相邻比较改为隔格比较。）
        let t = test_table();
        let mut w = World::new(1, 1, 0xABC);
        let (cx, cy) = (10, 10);
        for dx in 1..=6 {
            w.set_cell_stamped(&t, cx + dx, cy, 3, 0); // sand
        }
        let fseed = rng::frame_seed(w.seed, w.tick);
        let mut spawns = Vec::new();
        fire_ray(&mut w, &t, cx, cy, 8, 0, 16, u32::MAX, 0, fseed, 0, &mut spawns);

        assert_eq!(spawns.len(), 6, "6 个沙格应全部被摧毁：{spawns:?}");
        for i in 0..6i32 {
            assert_eq!(w.cell(cx + 1 + i, cy).material(), MAT_AIR, "沙格应变 air");
        }
        for i in 0..spawns.len() - 2 {
            assert!(
                spawns[i].vx.0 > spawns[i + 2].vx.0,
                "沿射线向外速度应单调衰减（隔格比较）：{:?}",
                spawns.iter().map(|s| s.vx.0).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn fire_ray_stops_at_wall_shields_cells_behind_and_does_not_destroy_wall() {
        let t = test_table();
        let mut w = World::new(1, 1, 0);
        let (cx, cy) = (10, 10);
        w.set_cell_stamped(&t, cx + 3, cy, 1, 0); // wall
        w.set_cell_stamped(&t, cx + 4, cy, 3, 0); // sand behind wall
        let fseed = rng::frame_seed(w.seed, w.tick);
        let mut spawns = Vec::new();
        fire_ray(&mut w, &t, cx, cy, 8, 0, 1000, u32::MAX, 0, fseed, 0, &mut spawns);

        assert_eq!(w.cell(cx + 3, cy).material(), 1, "wall 本身不可摧毁");
        assert_eq!(w.cell(cx + 4, cy).material(), 3, "wall 后方沙格应逐格完好（遮挡）");
        assert!(spawns.is_empty(), "撞墙前只有 air，撞墙即断线，不应产出任何溅射");
    }

    #[test]
    fn fire_ray_already_air_cells_cost_zero_and_do_not_respawn() {
        // 模拟"已被前序射线炸掉"：整条路径预先就是 air，射线应无阻碍走完
        // 全程且不产出任何 spawn（air 的 blast_cost=0，从不触发摧毁分支）。
        let t = test_table();
        let mut w = World::new(1, 1, 0);
        let fseed = rng::frame_seed(w.seed, w.tick);
        let mut spawns = Vec::new();
        fire_ray(&mut w, &t, 10, 10, 8, 0, 5, u32::MAX, 0, fseed, 0, &mut spawns);
        assert!(spawns.is_empty());
    }

    // ==================== fire_ray：近心汽化（vaporize_threshold，用户裁决 2026-08-30）====================

    /// 两个材质专为边界测试构造：`blast_cost` 分别精确算到 power=255 时
    /// `remaining` 落在阈值 128 的正上方一格（129）与恰好持平（128）——
    /// 隔离出"严格大于"判定的两侧，不受材质其他属性干扰。
    fn vaporize_boundary_table() -> MaterialTable {
        use crate::material::BLAST_COST_INFINITE;
        let def = |id: u8, name: &str, category: Category, blast_cost: u32, vaporize_threshold: u8| MaterialDef {
            blast_cost,
            vaporize_threshold,
            ..MaterialDef::base(id, name, category, 40)
        };
        MaterialTable::new(vec![
            def(0, "air", Category::Static, 0, 255),
            def(1, "wall", Category::Static, BLAST_COST_INFINITE, 255),
            // power=255 时 remaining=255-127=128，恰好等于阈值 128。
            def(2, "target_at_threshold", Category::Powder, 127, 128),
            // power=255 时 remaining=255-126=129，比阈值多 1。
            def(3, "target_above_threshold", Category::Powder, 126, 128),
        ])
        .unwrap()
    }

    #[test]
    fn fire_ray_vaporize_boundary_remaining_at_threshold_does_not_vaporize() {
        // remaining(128)*255 == power(255)*threshold(128)：判定是"严格大于"，
        // 持平不算，仍走正常摧毁 + 溅射路径。
        let t = vaporize_boundary_table();
        let mut w = World::new(1, 1, 0);
        let (cx, cy) = (10, 10);
        w.set_cell_stamped(&t, cx, cy, 2, 0);
        let fseed = rng::frame_seed(w.seed, w.tick);
        let mut spawns = Vec::new();
        fire_ray(&mut w, &t, cx, cy, 1, 0, 255, u32::MAX, 0, fseed, 0, &mut spawns);
        assert_eq!(spawns.len(), 1, "阈值恰好持平，不应汽化");
        assert_eq!(w.vaporized_total(), 0);
        assert_eq!(w.cell(cx, cy).material(), MAT_AIR, "仍应正常摧毁为 air");
    }

    #[test]
    fn fire_ray_vaporize_boundary_remaining_just_above_threshold_vaporizes() {
        // remaining(129)*255 > power(255)*threshold(128)：越过阈值一格即汽化。
        let t = vaporize_boundary_table();
        let mut w = World::new(1, 1, 0);
        let (cx, cy) = (10, 10);
        w.set_cell_stamped(&t, cx, cy, 3, 0);
        let fseed = rng::frame_seed(w.seed, w.tick);
        let mut spawns = Vec::new();
        fire_ray(&mut w, &t, cx, cy, 1, 0, 255, u32::MAX, 0, fseed, 0, &mut spawns);
        assert!(spawns.is_empty(), "越过阈值应汽化，不生成粒子");
        assert_eq!(w.vaporized_total(), 1);
        assert_eq!(w.cell(cx, cy).material(), MAT_AIR, "汽化仍需清空格子（质量蒸发）");
    }

    #[test]
    fn fire_ray_default_threshold_255_never_vaporizes_even_at_remaining_equals_power() {
        // RON 缺省 1.0 → 量化 255。blast_cost=0（非 air/wall 材质里的边界配置，
        // 纯为逼出 remaining==power 这个比例=1.0 的极端输入，现实材质不会
        // 这样配）依然不触发汽化——"严格大于"是关键：threshold=255 时条件
        // 退化为 `remaining > power`，而 `remaining <= power` 恒成立（cost
        // 是无符号扣减），故缺省材质在任何输入下都不汽化。
        use crate::material::BLAST_COST_INFINITE;
        let def = |id: u8, name: &str, category: Category, blast_cost: u32| MaterialDef {
            blast_cost,
            ..MaterialDef::base(id, name, category, 40)
        };
        let t = MaterialTable::new(vec![
            def(0, "air", Category::Static, 0),
            def(1, "wall", Category::Static, BLAST_COST_INFINITE),
            def(2, "target_zero_cost", Category::Powder, 0),
        ])
        .unwrap();
        let mut w = World::new(1, 1, 0);
        let (cx, cy) = (10, 10);
        w.set_cell_stamped(&t, cx, cy, 2, 0);
        let fseed = rng::frame_seed(w.seed, w.tick);
        let mut spawns = Vec::new();
        fire_ray(&mut w, &t, cx, cy, 1, 0, 1000, u32::MAX, 0, fseed, 0, &mut spawns);
        assert_eq!(spawns.len(), 1, "缺省阈值下 remaining==power 也不应汽化");
        assert_eq!(w.vaporized_total(), 0);
    }

    // ==================== Op::Explode：apply_op 行为测试（spec §10）====================

    fn explode_spawn_positions(spawns: &[SpawnRequest]) -> Vec<(i32, i32)> {
        let mut v: Vec<(i32, i32)> = spawns.iter().map(|s| (s.x.to_cell(), s.y.to_cell())).collect();
        v.sort();
        v
    }

    #[test]
    fn explode_center_cell_is_destroyed_when_r_at_least_one() {
        // 爆心口径钉死（任务书要求）：r>=1 时爆心格自身按第一格计费——只要
        // 爆心原本非 air 且能量足够，必被摧毁 + 溅射，不因"起点格豁免"跳过
        // （那是 DDA 粒子飞行语义，爆炸射线的口径明确相反，见 fire_ray 文档）。
        let t = test_table();
        let mut w = World::new(1, 1, 0);
        let (cx, cy) = (30, 30);
        w.set_cell_stamped(&t, cx, cy, 3, 0); // 爆心本身预置为沙
        let fseed = rng::frame_seed(w.seed, w.tick);
        let op = Op::Explode { x: cx, y: cy, r: 3, power: 50 };
        let mut spawns = Vec::new();
        w.apply_op(&t, &op, 0, fseed, 0, &mut spawns);

        assert_eq!(w.cell(cx, cy).material(), MAT_AIR, "爆心格必须被摧毁");
        assert!(
            explode_spawn_positions(&spawns).contains(&(cx, cy)),
            "爆心格必须产出溅射粒子：{:?}",
            explode_spawn_positions(&spawns)
        );
    }

    #[test]
    fn explode_thin_wall_shields_sand_behind_it() {
        // 行为测试（任务书）：薄墙遮挡——1 格 wall 后方的沙逐格完好。
        let t = test_table();
        let mut w = World::new(2, 2, 0);
        let (cx, cy) = (60, 60);
        w.set_cell_stamped(&t, cx + 4, cy, 1, 0); // 薄墙（1 格）
        for dx in 5..=9 {
            w.set_cell_stamped(&t, cx + dx, cy, 3, 0); // 墙后一整排沙
        }
        let fseed = rng::frame_seed(w.seed, w.tick);
        let op = Op::Explode { x: cx, y: cy, r: 12, power: 10_000 }; // 能量充裕，仍不能穿墙
        let mut spawns = Vec::new();
        w.apply_op(&t, &op, 0, fseed, 0, &mut spawns);

        assert_eq!(w.cell(cx + 4, cy).material(), 1, "wall 不可摧毁");
        for dx in 5..=9 {
            assert_eq!(w.cell(cx + dx, cy).material(), 3, "墙后第 {dx} 格沙应完好");
        }
    }

    #[test]
    fn explode_pit_conservation_destroyed_cells_equal_spawn_count() {
        // 挖坑守恒（任务书；口径随 vaporize_threshold 更新，用户裁决
        // 2026-08-30）：炸掉的格数 == 生成的溅射请求数 + 汽化计数
        // （汽化格既不生成粒子也不算"未处理"，质量确定性蒸发）。`test_table()`
        // 全部材质 `vaporize_threshold=255`（永不汽化），故本例
        // `vaporized_total` 恒为 0——断言写成通用形式，覆盖两条路径。
        let t = test_table();
        let mut w = World::new(2, 2, 0);
        let (cx, cy) = (60, 60);
        let r = 6;
        // 圆心为中心的一个方块实心沙，半径足够覆盖整个爆炸圆盘。
        for dy in -(r + 1)..=(r + 1) {
            for dx in -(r + 1)..=(r + 1) {
                w.set_cell_stamped(&t, cx + dx, cy + dy, 3, 0);
            }
        }
        let before = w.count_material(3);
        let fseed = rng::frame_seed(w.seed, w.tick);
        let op = Op::Explode { x: cx, y: cy, r, power: 1000 };
        let mut spawns = Vec::new();
        w.apply_op(&t, &op, 0, fseed, 0, &mut spawns);
        let after = w.count_material(3);

        let destroyed = before - after;
        assert!(destroyed > 0, "应至少摧毁一些沙格");
        assert_eq!(
            destroyed,
            spawns.len() + w.vaporized_total() as usize,
            "摧毁格数必须等于（生成的溅射请求数 + 汽化计数）"
        );
        // 每个 spawn 必须落在一个"曾是沙、现在是 air"的坐标上，且坐标互不重复
        // （每格至多被摧毁一次，spec §6 point 4）。
        let positions = explode_spawn_positions(&spawns);
        let mut dedup = positions.clone();
        dedup.dedup();
        assert_eq!(dedup.len(), positions.len(), "spawn 坐标不应重复：{positions:?}");
        for &(x, y) in &positions {
            assert_eq!(w.cell(x, y).material(), MAT_AIR, "spawn 坐标处网格应已是 air");
        }
    }

    // ==================== Op::Explode：材质差异化汽化（用户裁决 2026-08-30）====================

    /// 与 `data/materials.ron` 初值同口径：water 阈值 0.4（量化 102），
    /// sand 阈值 0.7（量化 179）。
    fn mixed_vaporize_table() -> MaterialTable {
        use crate::material::BLAST_COST_INFINITE;
        let def = |id: u8, name: &str, category: Category, density: u16, blast_cost: u32, vaporize_threshold: u8| {
            MaterialDef { blast_cost, vaporize_threshold, ..MaterialDef::base(id, name, category, density) }
        };
        MaterialTable::new(vec![
            def(0, "air", Category::Static, 0, 0, 255),
            def(1, "wall", Category::Static, 100, BLAST_COST_INFINITE, 255),
            def(2, "water", Category::Liquid, 16, 1, 102),
            def(3, "sand", Category::Powder, 40, 2, 179),
        ])
        .unwrap()
    }

    #[test]
    fn explode_mixed_water_sand_target_vaporizes_more_water_than_sand() {
        // 沙水混合目标（任务书要求）：同一次 Op::Explode 命中两种材质，water
        // 阈值（0.4）比 sand（0.7）低，预期 water 的汽化比例更高。几何：圆心
        // 左侧（dx<=0）填水、右侧（dx>0）填沙——每条射线沿固定方向走，dx 的
        // 符号沿途不变，故每条射线全程只穿一种材质，互不干扰。
        //
        // 参数口径（power=100）：water cost=1，remaining=100-d，
        // ratio>0.4 等价 d<60——目标区域内所有射线 d 远小于 60，water 应
        // 全汽化；sand cost=2，remaining=100-2d，ratio>0.702 等价 d<14.9，
        // 目标区域 d 覆盖到 ~26+，故 sand 应"近心汽化、远心正常溅射"两段
        // 都有——用整数交叉相乘比较汽化比例，不引入浮点。
        let t = mixed_vaporize_table();
        let mut w = World::new(2, 2, 0);
        let (cx, cy) = (60, 60);
        let r = 24;
        for dy in -(r + 2)..=(r + 2) {
            for dx in -(r + 2)..=(r + 2) {
                let mat = if dx <= 0 { 2 } else { 3 };
                w.set_cell_stamped(&t, cx + dx, cy + dy, mat, 0);
            }
        }
        let before_water = w.count_material(2);
        let before_sand = w.count_material(3);
        let fseed = rng::frame_seed(w.seed, w.tick);
        let op = Op::Explode { x: cx, y: cy, r, power: 100 };
        let mut spawns = Vec::new();
        w.apply_op(&t, &op, 0, fseed, 0, &mut spawns);
        let after_water = w.count_material(2);
        let after_sand = w.count_material(3);

        let destroyed_water = before_water - after_water;
        let destroyed_sand = before_sand - after_sand;
        let spawn_water = spawns.iter().filter(|s| s.material == 2).count();
        let spawn_sand = spawns.iter().filter(|s| s.material == 3).count();
        let vaporized_water = destroyed_water - spawn_water;
        let vaporized_sand = destroyed_sand - spawn_sand;

        assert!(destroyed_water > 0 && destroyed_sand > 0, "两种材质都应被摧毁一些");
        assert!(vaporized_water > 0, "water 阈值更低，近心应有汽化");
        assert!(spawn_sand > 0, "sand 阈值更高，远心应有未汽化、正常溅射的部分");
        // vaporized_water/destroyed_water > vaporized_sand/destroyed_sand
        // <=> 交叉相乘（两边都是非负整数，不改变不等号方向）。
        assert!(
            vaporized_water * destroyed_sand > vaporized_sand * destroyed_water,
            "water 汽化比例应高于 sand：water {vaporized_water}/{destroyed_water}，\
             sand {vaporized_sand}/{destroyed_sand}"
        );
    }

    #[test]
    fn explode_zero_power_is_a_no_op() {
        let t = test_table();
        let mut w = World::new(1, 1, 0);
        let (cx, cy) = (10, 10);
        w.set_cell_stamped(&t, cx, cy, 3, 0);
        let fseed = rng::frame_seed(w.seed, w.tick);
        let op = Op::Explode { x: cx, y: cy, r: 5, power: 0 };
        let mut spawns = Vec::new();
        w.apply_op(&t, &op, 0, fseed, 0, &mut spawns);
        assert_eq!(w.cell(cx, cy).material(), 3, "power=0 不应摧毁任何格子");
        assert!(spawns.is_empty());
    }

    #[test]
    fn explode_repeated_application_is_deterministic() {
        // 同 Op 重跑一致（任务书）：两个独立构造、内容完全相同的世界，喂同一
        // 个 Op::Explode（同 fseed/op_idx），必须产出逐位相同的溅射序列。
        let t = test_table();
        let (cx, cy) = (60, 60);
        let build = || {
            let mut w = World::new(2, 2, 0x77);
            for dy in -4..=4 {
                for dx in -4..=4 {
                    w.set_cell_stamped(&t, cx + dx, cy + dy, 3, 0);
                }
            }
            w
        };
        let mut wa = build();
        let mut wb = build();
        let fseed = rng::frame_seed(wa.seed, wa.tick);
        let op = Op::Explode { x: cx, y: cy, r: 4, power: 40 };

        let mut a = Vec::new();
        wa.apply_op(&t, &op, 0, fseed, 0, &mut a);
        let mut b = Vec::new();
        wb.apply_op(&t, &op, 0, fseed, 0, &mut b);

        let av: Vec<(i32, i32, i32, i32)> = a.iter().map(|s| (s.x.0, s.y.0, s.vx.0, s.vy.0)).collect();
        let bv: Vec<(i32, i32, i32, i32)> = b.iter().map(|s| (s.x.0, s.y.0, s.vx.0, s.vy.0)).collect();
        assert_eq!(av, bv, "同一 Op::Explode 重复应用必须给出逐位相同的溅射序列");
    }

    #[test]
    fn explode_same_tick_two_explodes_have_different_jitter_sequences() {
        // I1 同款测试（任务书）：同 tick 内两个参数完全相同、圆心重合的
        // Op::Explode（op_idx 不同），抖动序列必须不同——即便坐标键相同，
        // salt=op_idx 这一维必须生效（rng.rs::STREAM_EXPLODE 文档）。
        let t = test_table();
        let (cx, cy) = (60, 60);
        let build = || {
            let mut w = World::new(2, 2, 0xC0FFEE);
            for dy in -3..=3 {
                for dx in -3..=3 {
                    w.set_cell_stamped(&t, cx + dx, cy + dy, 3, 0);
                }
            }
            w
        };
        let mut wa = build();
        let mut wb = build();
        let fseed = rng::frame_seed(wa.seed, wa.tick);
        let op = Op::Explode { x: cx, y: cy, r: 3, power: 30 };

        let mut a = Vec::new();
        wa.apply_op(&t, &op, 0, fseed, 0, &mut a);
        let mut b = Vec::new();
        wb.apply_op(&t, &op, 0, fseed, 1, &mut b);

        let av: Vec<(i32, i32)> = a.iter().map(|s| (s.vx.0, s.vy.0)).collect();
        let bv: Vec<(i32, i32)> = b.iter().map(|s| (s.vx.0, s.vy.0)).collect();
        assert_ne!(av, bv, "op_idx 不同时，同参数同圆心的两个 Explode 抖动序列必须不同（I1 回归）");
    }

    #[test]
    fn explode_jitter_matches_from_ratio_one_half() {
        assert_eq!(EXPLODE_JITTER, Fx::from_ratio(1, 2));
    }

    #[test]
    fn explode_speed_matches_from_int_eight() {
        assert_eq!(EXPLODE_SPEED, Fx::from_int(8));
    }

    #[test]
    fn ray_fluct_mapping_bounds() {
        // v=200，q=200/4=50：sym 端点 [150,250]；非 sym 端点 [150,200]。
        assert_eq!(ray_fluct(200, 0, true), 150);
        assert_eq!(ray_fluct(200, u32::MAX, true), 250);
        assert_eq!(ray_fluct(200, 0, false), 150);
        assert_eq!(ray_fluct(200, u32::MAX, false), 200);
        // q=0（v<4 整除截断）：无涨落；下限保护恒 ≥1。
        assert_eq!(ray_fluct(3, 0, true), 3);
        assert_eq!(ray_fluct(1, 0, true), 1);
    }

    #[test]
    fn fire_ray_max_cells_caps_ray_length() {
        // 8 格沙、能量充足，max_cells=4（含爆心格）→ 只摧毁前 3 格沙。
        let t = test_table();
        let mut w = World::new(1, 1, 0);
        let (cx, cy) = (10, 10);
        for dx in 1..=8 {
            w.set_cell_stamped(&t, cx + dx, cy, 3, 0);
        }
        let fseed = rng::frame_seed(w.seed, w.tick);
        let mut spawns = Vec::new();
        fire_ray(&mut w, &t, cx, cy, 9, 0, 1000, 4, 0, fseed, 0, &mut spawns);
        assert_eq!(spawns.len(), 3, "max_cells=4 含爆心，应只摧毁 3 格沙");
        assert_eq!(w.cell(cx + 3, cy).material(), MAT_AIR);
        assert_eq!(w.cell(cx + 4, cy).material(), 3, "第 4 格沙应因射程封顶幸存");
    }

    #[test]
    fn explode_crater_is_not_perfectly_circular() {
        // 方向涨落（能量 ±25% + 射程 −25%..0）应让四个轴向的摧毁半径不全等。
        let t = test_table();
        let mut w = World::new(2, 2, 0x77);
        for y in 32..96 {
            for x in 32..96 {
                w.set_cell_stamped(&t, x, y, 3, 0);
            }
        }
        let fseed = rng::frame_seed(w.seed, w.tick);
        let mut spawns = Vec::new();
        // r=16、power=1000：能量远超 16 格沙的成本，射程涨落是约束项。
        w.apply_op(&t, &Op::Explode { x: 64, y: 64, r: 16, power: 1000 }, 0, fseed, 0, &mut spawns);
        let extent = |sx: i32, sy: i32| -> i32 {
            let mut d = 0;
            while w.cell(64 + sx * (d + 1), 64 + sy * (d + 1)).material() == MAT_AIR {
                d += 1;
            }
            d
        };
        let exts = [extent(1, 0), extent(-1, 0), extent(0, 1), extent(0, -1)];
        assert!(
            exts.iter().any(|e| *e != exts[0]),
            "四轴摧毁半径全等 = 完美圆，方向涨落未生效：{exts:?}"
        );
        // 涨落有界：射程最短 3/4 r，最长 r。
        for e in exts {
            assert!((12..=16).contains(&e), "轴向半径 {e} 超出 [12,16] 涨落界：{exts:?}");
        }
    }

    #[test]
    fn blast_mass_factor_golden_values() {
        // 参考密度 40：沙（40）系数恒 1.0——手感锚点；水（16）2.5 = 0x28000。
        assert_eq!(Fx::from_ratio(REF_BLAST_DENSITY, 40), Fx::from_int(1));
        assert_eq!(Fx::from_ratio(REF_BLAST_DENSITY, 16), Fx(0x0002_8000));
    }

    #[test]
    fn fire_ray_lighter_material_launches_faster_than_heavier() {
        // 冲量物理（v ∝ 1/密度）：同 power、同距离，水（密度 16）出射速度
        // 必须高于沙（密度 40）。水 8×2.5=20 会被 clamp 到 16，沙 ≤8+抖动 0.5，
        // 差距远大于抖动摆幅，断言稳定。
        let t = test_table();
        let mut w = World::new(1, 1, 0xD5);
        let (cx, cy) = (10, 10);
        w.set_cell_stamped(&t, cx + 1, cy, 3, 0); // sand
        w.set_cell_stamped(&t, cx - 1, cy, 2, 0); // water（test_table 里 id 自查为准）
        let fseed = rng::frame_seed(w.seed, w.tick);
        let mut spawns = Vec::new();
        fire_ray(&mut w, &t, cx, cy, 1, 0, 255, u32::MAX, 0, fseed, 0, &mut spawns);
        fire_ray(&mut w, &t, cx, cy, -1, 0, 255, u32::MAX, 0, fseed, 0, &mut spawns);
        assert_eq!(spawns.len(), 2, "沙、水各一格应各溅射一颗：{spawns:?}");
        let sand_speed = spawns[0].vx.0.abs();
        let water_speed = spawns[1].vx.0.abs();
        assert!(
            water_speed > sand_speed,
            "轻材质应飞更快：water |vx|={water_speed:#x} vs sand |vx|={sand_speed:#x}"
        );
    }
}
