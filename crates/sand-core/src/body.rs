//! 像素刚体同步态本体（M3 spec §2/§3/§6）——架构 §3 的 `stamp` 子系统。
//!
//! **全系统唯一同时读写 grid 与 physics 的模块**（架构 §4 白名单）。运行在 tick
//! 管线的串行阶段（第 3 步在网格四相之前、第 7 步在粒子相之后），直接操作
//! `World`，不经 `WriteWindow`。
//!
//! 所有权 = `Cell::BODY_FLAG`（bit 23）：只做地形掩码排除与对账识别，**不豁免
//! 任何 CA 规则**——盖章格就是其材质的 Static 格，可燃、可灭、烧尽即变 air
//! （标志随 `pack` 消失）⇒ 对账视作像素被毁。
//!
//! 盖章 = **实心光栅化（逆映射）**：对变换后 AABB 内每格，格心逆变换回局部坐标
//! 查位图——天然无洞（FSS Issue #4 的坑）。变换未变（`to_bits` 相等）或睡眠的
//! 刚体**跳过反盖章/盖章**：零写入、零溅射，chunk 照常入睡。

use crate::cell::Cell;
use crate::fixed::{Fx, HALF_CELL};
use crate::geom::{components4, rect_cover, Rect};
use crate::chunk::CHUNK;
use crate::material::{self, Category, MaterialTable, MAT_AIR};
use crate::physics::GRAVITY_CELLS_PER_S2;
use crate::particle::clamp_speed;
use crate::physics::{BodyHandle, PhysicsWorld};
use crate::world::{SpawnRequest, World};

/// 最小刚体面积（spec §6/§8）：低于此的碎片脱格成粒子。
pub const MIN_BODY_PIXELS: usize = 12;
/// 每 tick 重提取限额（spec §6）。
pub const MAX_REEXTRACT_PER_TICK: usize = 2;
/// 刚体数上限（spec §8）：超限 `SpawnBody` 确定性拒绝并计数（粒子池先例）。
pub const MAX_BODIES: usize = 256;
/// 刚体 AABB 整体出界超过此格数即确定性移除——掉出世界的箱子不永久占用引擎。
pub const OUT_OF_WORLD_MARGIN: i32 = 64;
/// 地形碰撞只为刚体 AABB 外扩这么多 chunk 的范围生成（spec §4）。
pub const TERRAIN_MARGIN: i32 = 1;
/// 水面线采样从接触格沿接触行向外最多再走这么多格（spec §5）：紧邻列被自身溅水污染，
/// 远列读数才稳；连通性由接触行保证。
pub const SURFACE_REACH: i32 = 5;
/// 睡眠刚体的唤醒门槛（spec §5）：水面线 `h` 与上次清醒时相差达到这么多行即唤醒。
/// 水不是碰撞体，退掉/涨上来不会经引擎唤醒刚体；浮力又只施于清醒刚体——不查这一条，
/// 池壁炸穿后浮着的木箱会挂在半空（2026-09-03 目检实测）。用 `h` 而不用淹没量比例：
/// 池面常年有 ±1 行抖动，一行对 32 宽木条就是 17% 体积，按比例设门槛会睡了又醒（实测
/// 3000 tick 内 11 次）；2 行滞回对尺寸无关、对一行抖动免疫。
pub const WAKE_H_ROWS: i32 = 2;
/// `Body::last_h` 的"没有水面线"哨兵。
const NO_SURFACE: i32 = i32::MAX;
/// "沉降液体"的竖直速度上限（`Cell::vel` 原始值，Q3.2，`VEL_ONE = 4` = 1 格/tick）：低于此
/// 才算能给浮力/载荷的水。落水流几格内就到 ≥ 2 格/tick；入水冲击扰动的池水只有 1 格/tick 上下，
/// 用 0 会把入水那几十 tick 的接触全滤掉、木条不减速砸到池底（实测）。
pub const SETTLED_VEL_MAX: u8 = 2 * crate::cell::VEL_ONE;
/// 顶面载荷（决策记录第 16 条）：堆在顶面像素上、且在水面线之上的沉降液体按格计重往下压；
/// 睡眠刚体顶上堆到这么多格（≈ 一整行）即唤醒让它沉一沉、把水丘滑掉。
pub const WAKE_TOP_LOAD_CELLS: f32 = 16.0;
/// 爆炸推刚体的系数（spec 决策记录第 17 条，Noita `ConfigExplosion.physics_explosion_power` 的
/// 对应物）：每个半径内的盖章像素贡献冲量 `BLAST_BODY_FACTOR × REF_BLAST_DENSITY × EXPLODE_SPEED
/// × (1 − d/r)`，方向爆心 → 像素，合力施于受击像素的加权中心。与粒子同一套"同一冲量、v ∝ 1/ρ"
/// 口径（`explode.rs`）：整箱在爆心附近时 Δv ≈ 0.25 × 8 格/tick × 40/ρ_body。目检可调。
pub const BLAST_BODY_FACTOR: f32 = 0.25;
/// 逐淹没像素的阻力系数（spec §5）：线 `F = −K_DRAG × n_sub × v`，角
/// `τ = −K_DRAG × Σ|r_i|² × ω`（同一系数，是"每个淹没像素受 −K·v_i"的合力与合力矩）。
/// 取 200：对 16×12 木箱（浮力"弹簧"ω ≈ 10 rad/s）阻尼比 ≈ 0.8，近临界——入水后
/// 一两个来回即静止；全淹没时角阻尼率与线阻尼率相同（≈ K/ρ_body）。目检可调。
pub const K_DRAG: f32 = 200.0;

pub struct Body {
    /// 单调分配，入哈希。
    pub id: u16,
    /// 必须 Static 类别（`spawn_rect` 校验）。
    pub material: u8,
    pub w: u16,
    pub h: u16,
    /// 局部位图占位（行主序 `j*w + i`）。
    pub mask: Vec<bool>,
    /// 局部像素的燃烧 counter——反盖章时从格子读回、盖章时写回，燃烧进度随刚体走。
    pub counter: Vec<u8>,
    /// 上一次盖章的格清单 `(x, y, 局部索引)`：反盖章与对账都按清单驱动，不扫 AABB。
    pub stamped: Vec<(i32, i32, u32)>,
    /// 对账发现像素被毁 ⇒ 待重提取（Task 4）。
    pub dirty: bool,
    pub(crate) handle: BodyHandle,
    /// 上一次盖章时的 `transform().to_bits()`；`None` = 尚未盖过章。
    last_xf: Option<(u32, u32, u32)>,
    /// 上一次清醒时采到的水面线 `h`（无水面 = `NO_SURFACE`；入哈希）：睡眠期间与之比较
    /// 决定是否唤醒。
    last_h: i32,
}

impl Body {
    /// 旋转中心 = 位图中心（局部坐标，格单位）。
    fn pivot(&self) -> (f32, f32) {
        (self.w as f32 * 0.5, self.h as f32 * 0.5)
    }

    /// 当前位图的矩形覆盖（重提取重建碰撞形状用）。
    pub(crate) fn rects(&self) -> Vec<Rect> {
        rect_cover(&self.mask, self.w as usize, self.h as usize)
    }
}

#[derive(Default)]
pub struct Bodies {
    /// 按 id 升序。
    pub list: Vec<Body>,
    pub next_id: u16,
    /// 待重提取队列（按 id 序，入状态、入哈希）。
    pub reextract_queue: Vec<u16>,
    /// `SpawnBody` 被上限/契约拒绝的次数（诊断，不入哈希）。
    pub rejected_total: u64,
    /// 地形碰撞体重建次数（诊断，不入哈希；缓存命中执法测试用）。
    pub terrain_rebuilds: u64,
    /// 本 tick 的爆炸（`Op::Explode` 的 `(x, y, r)`，入队序）：第 7 步重提取**之后**才施冲量，
    /// 让被炸开的两半各自受力飞开（tick 内消费完，不跨 tick、不入哈希）。
    pub(crate) pending_blasts: Vec<(i32, i32, i32)>,
    /// 已缓存的地形 chunk → 上次交给引擎的矩形（含空）。**矩形没变就不碰引擎**：
    /// 删/重建静态碰撞体会重置接触、唤醒压在上面的刚体，形成"盖章标脏 → 重建 →
    /// 唤醒 → 再盖章"的死循环（Task 3 实测：静止箱子永远睡不着）。
    terrain_cached: std::collections::BTreeMap<(u32, u32), Vec<Rect>>,
}

fn xf_bits(t: (f32, f32, f32)) -> (u32, u32, u32) {
    (t.0.to_bits(), t.1.to_bits(), t.2.to_bits())
}

/// 引擎速度（格/秒）→ 网格粒子速度（格/tick，`Fx`）。唯一的 f32 → 定点转换点：
/// `as i32` 截断在 IEEE 下确定。
fn vel_to_fx(v: f32) -> Fx {
    clamp_speed(Fx((v / 60.0 * 65536.0) as i32))
}

impl Bodies {
    pub fn new() -> Bodies {
        Bodies::default()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn get(&self, id: u16) -> Option<&Body> {
        self.list.iter().find(|b| b.id == id)
    }

    /// 生成矩形刚体（spec §8）：`(x, y)` 左上角格坐标，`w×h` 格，`angle_deg` 绕位图
    /// 中心的初始旋转（整数度）。契约：材质 Static、面积 ≥ `MIN_BODY_PIXELS`、数量 <
    /// `MAX_BODIES`；违反即确定性拒绝并计数。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn_rect(
        &mut self,
        phys: &mut PhysicsWorld,
        table: &MaterialTable,
        material: u8,
        x: i32,
        y: i32,
        w: u16,
        h: u16,
        angle_deg: i16,
    ) -> bool {
        let area = w as usize * h as usize;
        if self.list.len() >= MAX_BODIES
            || area < MIN_BODY_PIXELS
            || table.category(material) != Category::Static
            || material == MAT_AIR
        {
            self.rejected_total += 1;
            return false;
        }
        let mask = vec![true; area];
        let rects = rect_cover(&mask, w as usize, h as usize);
        let pivot = (w as f32 * 0.5, h as f32 * 0.5);
        let pos = (x as f32 + pivot.0, y as f32 + pivot.1);
        // 整数度 → f32 弧度：唯一的一次转换，f32 乘法在 IEEE 下确定。
        let angle = angle_deg as f32 * (std::f32::consts::PI / 180.0);
        let handle = phys.insert_body(&rects, pivot, table.density(material) as f32, pos, angle, (0.0, 0.0), 0.0);
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.list.push(Body {
            id,
            material,
            w,
            h,
            mask,
            counter: vec![0; area],
            stamped: Vec::new(),
            dirty: false,
            handle,
            last_xf: None,
            last_h: NO_SURFACE,
        });
        true
    }

    /// 第 3 步后半（spec §3）：对每个刚体按 id 序——变换未变则跳过；否则反盖章
    /// 旧脚印（读回 counter）→ 实心盖章新脚印。整体出界的刚体在此移除。
    pub(crate) fn stamp_all(
        &mut self,
        world: &mut World,
        table: &MaterialTable,
        phys: &mut PhysicsWorld,
        stamp: u8,
        spawns: &mut Vec<SpawnRequest>,
    ) {
        let mut gone: Vec<u16> = Vec::new();
        for body in self.list.iter_mut() {
            let xf = xf_bits(phys.transform(body.handle));
            if body.last_xf == Some(xf) {
                continue; // 防抖：零写入、零溅射（睡眠刚体天然落入此分支）
            }
            unstamp(body, world, table, stamp);
            let (x0, y0, x1, y1) = aabb_cells(body, phys);
            if x1 < -OUT_OF_WORLD_MARGIN
                || y1 < -OUT_OF_WORLD_MARGIN
                || x0 > world.width() + OUT_OF_WORLD_MARGIN
                || y0 > world.height() + OUT_OF_WORLD_MARGIN
            {
                gone.push(body.id);
                continue;
            }
            stamp_body(body, world, table, phys, stamp, spawns, (x0, y0, x1, y1));
            body.last_xf = Some(xf);
        }
        for id in gone {
            self.remove(id, phys);
        }
    }

    /// 第 3 步前半之一（spec §4，B′）：只为刚体 AABB 外扩 `TERRAIN_MARGIN` chunk 范围
    /// 内的 chunk 生成硬格碰撞体，按 chunk 缓存；失效 = 该 chunk 上一 tick `dirty` 非空
    /// （封帧时交换的现成位）。离开范围的 chunk 从引擎移除。键按 `BTreeSet` 序遍历。
    pub(crate) fn refresh_terrain(&mut self, world: &World, table: &MaterialTable, phys: &mut PhysicsWorld) {
        let mut needed = std::collections::BTreeSet::new();
        for body in &self.list {
            let (x0, y0, x1, y1) = aabb_cells(body, phys);
            let c = CHUNK as i32;
            let (cx0, cy0) = ((x0.div_euclid(c) - TERRAIN_MARGIN).max(0), (y0.div_euclid(c) - TERRAIN_MARGIN).max(0));
            let (cx1, cy1) = (
                (x1.div_euclid(c) + TERRAIN_MARGIN).min(world.width_chunks as i32 - 1),
                (y1.div_euclid(c) + TERRAIN_MARGIN).min(world.height_chunks as i32 - 1),
            );
            for cy in cy0..=cy1 {
                for cx in cx0..=cx1 {
                    needed.insert((cx as u32, cy as u32));
                }
            }
        }
        let stale: Vec<(u32, u32)> = self.terrain_cached.keys().copied().filter(|k| !needed.contains(k)).collect();
        for key in stale {
            phys.clear_terrain(key);
            self.terrain_cached.remove(&key);
        }
        for &(cx, cy) in &needed {
            let ci = world.chunk_index(cx as usize, cy as usize);
            let cached = self.terrain_cached.get(&(cx, cy));
            if cached.is_some() && world.chunks[ci].dirty.is_empty() {
                continue;
            }
            let rects = terrain_rects(world, table, cx, cy);
            if cached.is_some_and(|c| *c == rects) {
                continue; // 硬格没变（脏的是液体/气体/刚体自身格）：不碰引擎
            }
            phys.set_terrain((cx, cy), &rects);
            self.terrain_cached.insert((cx, cy), rects);
            self.terrain_rebuilds += 1;
        }
    }

    /// 第 3 步前半之二（spec §5，采样式阿基米德）：对每个刚体，水面线采样得 `h`
    /// （`surface_line`，接触门控）；上次盖章脚印的每个像素按其**连续**世界中心（局部像素
    /// 中心经刚体变换，随位姿连续变化，不是盖章格的整数坐标）算被 `y ≥ h` 半平面盖住的
    /// 分数 `w ∈ [0, 1]`。清醒刚体：`F_浮 = Σw × ρ_liq × g` 逆重力施于加权质心，阻力
    /// `−K_DRAG × Σw × v`、角阻力 `−K_DRAG × Σ w|r|² × ω`。**睡眠刚体**：只在其 AABB
    /// （外扩采样触达）所在 chunk 上一 tick 有写入时评估，水面线 `h` 与上次清醒时相差
    /// ≥ `WAKE_H_ROWS` 行即唤醒并照常施力——水位变了浮体得跟着走，平静的池子零成本。
    ///
    /// 为什么按分数（2026-09-03 目检修订）：整格计数下浮力是位姿的阶梯函数，一行就是
    /// 一整排像素的力阶（32 宽木条 = 200 格/s²），平衡点落在台阶立面上 ⇒ 每 tick 在
    /// ±1.7 格/s 间抖、永远过不了 1 格/s 的睡眠阈值；连续计权才有真平衡点。没有角阻尼
    /// 时浮体的横摇永不衰减，任何扰动注入的能量全留在自旋里，故角阻力同时补上。
    /// 浮点只在此处与引擎边界活动，累加按清单序，确定。
    pub(crate) fn apply_buoyancy(&mut self, world: &World, table: &MaterialTable, phys: &mut PhysicsWorld) {
        for body in self.list.iter_mut() {
            if body.stamped.is_empty() {
                continue;
            }
            let aabb = aabb_cells(body, phys);
            let sleeping = phys.is_sleeping(body.handle);
            if sleeping && !any_chunk_dirty(world, aabb, SURFACE_REACH + 1) {
                continue;
            }
            let (px, py, _) = phys.transform(body.handle);
            let (pvx, pvy) = body.pivot();
            let w = body.w as i32;
            let line = surface_line(world, table, body, aabb);
            let h_now = line.as_ref().map_or(NO_SURFACE, |(h, _, _)| *h);
            let load = top_load(world, table, body, line.as_ref().map_or(&[][..], |(_, _, c)| c.as_slice()));
            if sleeping {
                let moved = match (h_now, body.last_h) {
                    (NO_SURFACE, NO_SURFACE) => false,
                    (NO_SURFACE, _) | (_, NO_SURFACE) => true,
                    (a, b) => (a - b).abs() >= WAKE_H_ROWS,
                };
                let heaped = load.is_some_and(|(n, _, _)| n >= WAKE_TOP_LOAD_CELLS);
                if !moved && !heaped {
                    continue;
                }
                phys.wake(body.handle);
            }
            body.last_h = h_now;
            if let Some((n_top, at, rho_top)) = load {
                phys.apply_force_at(body.handle, (0.0, n_top * rho_top as f32 * GRAVITY_CELLS_PER_S2), at);
            }
            if line.is_none() {
                if let Some(n_bottom) = sealed_bottom(world, table, body) {
                    // 封闭水柱不可压缩：向下速度截断（撞上即停）+ 抵消重力 + 阻力（横向/微动）
                    phys.stop_downward(body.handle);
                    phys.apply_force(body.handle, (0.0, -phys.mass(body.handle) * GRAVITY_CELLS_PER_S2));
                    phys.apply_drag(body.handle, K_DRAG * n_bottom);
                }
                continue;
            }
            let (mut n, mut sx, mut sy, mut r2) = (0f32, 0f32, 0f32, 0f32);
            let mut rho = 0u16;
            if let Some((h, r, _)) = line {
                rho = r;
                let hf = h as f32;
                for &(_, _, idx) in &body.stamped {
                    let (i, j) = ((idx as i32 % w) as f32, (idx as i32 / w) as f32);
                    let (cx, cy) = phys.local_to_world(body.handle, (i + 0.5 - pvx, j + 0.5 - pvy));
                    // 像素竖向占 [cy − 0.5, cy + 0.5]，与 y ≥ h 的重叠长度
                    let wgt = (cy + 0.5 - hf).clamp(0.0, 1.0);
                    if wgt <= 0.0 {
                        continue;
                    }
                    n += wgt;
                    sx += wgt * cx;
                    sy += wgt * cy;
                    let (dx, dy) = (cx - px, cy - py);
                    r2 += wgt * (dx * dx + dy * dy);
                }
            }
            if n <= 0.0 {
                continue;
            }
            let centroid = (sx / n, sy / n);
            let f_up = n * rho as f32 * GRAVITY_CELLS_PER_S2;

            phys.apply_force_at(body.handle, (0.0, -f_up), centroid);
            // 逐像素阻力 −K·w·v_i、v_i = v + ω×r_i 的合力/合力矩：线 −K·Σw·v，角 −K·Σw|r|²·ω。
            phys.apply_drag(body.handle, K_DRAG * n);
            phys.apply_angular_drag(body.handle, K_DRAG * r2);
        }
    }

    /// `Op::Explode` 的刚体侧（spec 决策记录第 17 条，Noita `physics_throw_enabled`）：第 1 步
    /// 只入队 `pending_blasts`，第 7 步对账/重提取**之后**才调本函数——爆心在箱子里时对整箱求和
    /// 左右抵消、两半原地不动（crate_yard tick 400 实测）；切开后各半的像素都在爆心一侧，各自飞开。
    /// 对每个刚体按 id 序，脚印里落在半径内的像素各贡献一份冲量 `(1 − d/r)` 沿爆心 → 像素方向，
    /// 乘 `BLAST_BODY_FACTOR × REF_BLAST_DENSITY × EXPLODE_SPEED` 施于受击像素的加权中心（远近像素
    /// 不等 ⇒ 扭矩白送）；`apply_impulse_at` 唤醒。被炸掉的像素已由对账剔除，不计。
    pub(crate) fn apply_blast(&mut self, phys: &mut PhysicsWorld, x: i32, y: i32, r: i32) {
        use crate::explode::{EXPLODE_SPEED, REF_BLAST_DENSITY};
        if r <= 0 {
            return;
        }
        let (cx, cy, rf) = (x as f32 + 0.5, y as f32 + 0.5, r as f32);
        let per_pixel = BLAST_BODY_FACTOR * REF_BLAST_DENSITY as f32 * (EXPLODE_SPEED.0 as f32 / 65536.0) * 60.0;
        for body in &self.list {
            let (mut jx, mut jy, mut sx, mut sy, mut sw) = (0f32, 0f32, 0f32, 0f32, 0f32);
            for &(px, py, _) in &body.stamped {
                let (dx, dy) = (px as f32 + 0.5 - cx, py as f32 + 0.5 - cy);
                let d = (dx * dx + dy * dy).sqrt();
                if d >= rf || d <= 0.0 {
                    continue;
                }
                let w = 1.0 - d / rf;
                jx += w * dx / d;
                jy += w * dy / d;
                sx += w * (px as f32 + 0.5);
                sy += w * (py as f32 + 0.5);
                sw += w;
            }
            if sw <= 0.0 {
                continue;
            }
            phys.apply_impulse_at(body.handle, (jx * per_pixel, jy * per_pixel), (sx / sw, sy / sw));
        }
    }

    /// 按 body id 序（`self.list` 本就按 id 升序）查"这一格属于哪个刚体"
    /// （M4 Task 6 spec §5.5，弹体单点冲量落点反查）：线性扫描每个刚体的
    /// 盖章清单——刚体数以十计、清单数以百计，且只在弹体命中格判定里按需
    /// 调用一次（不是逐格热路径），不值得为它建一张坐标索引结构（CLAUDE.md
    /// 红线 4：禁 HashMap，本函数天然不需要）。
    fn body_index_at(&self, x: i32, y: i32) -> Option<usize> {
        self.list.iter().position(|b| b.stamped.iter().any(|&(px, py, _)| px == x && py == y))
    }

    /// 单点冲量原语（M4 spec §5.5，Interfaces 一节点名的签名）：在网格坐标
    /// `(x, y)` 施加 `(jx, jy)`（引擎单位，f32）——命中格属于哪个 body 由
    /// [`body_index_at`] 反查，查不到（这一格不属于任何刚体的盖章清单，
    /// 调用方判定有误或该刚体本 tick 刚好被对账剔除）即静默无操作，不 panic
    /// ——与 `apply_blast` 对空清单的处理同一体例（`sw <= 0.0` 时 `continue`）。
    /// 不做半径加权（`apply_blast` 才有"越靠爆心权重越高"的加权中心），
    /// 单点直接施于命中像素中心 `(x+0.5, y+0.5)`。
    pub(crate) fn apply_point_impulse(&mut self, phys: &mut PhysicsWorld, x: i32, y: i32, jx: f32, jy: f32) {
        if let Some(bi) = self.body_index_at(x, y) {
            phys.apply_impulse_at(self.list[bi].handle, (jx, jy), (x as f32 + 0.5, y as f32 + 0.5));
        }
    }

    /// 弹体侧调用入口（M4 spec §5.5，Noita `physics_impulse_coeff`：
    /// `Impulse = coeff × velocity`）：`coeff_milli` 是 `SpellDef::
    /// physics_impulse`（千分位整数），`vel` 是命中那一刻的弹体速度（格/tick，
    /// `Fx`）。**`Fx → f32` 的转换只发生在本函数内**（"浮点转换只发生在
    /// physics 适配层边界"的落地——`body.rs` 是架构 §5 唯一同时接触 grid
    /// 与 physics 的模块，调用方 `projectile.rs` 因此不需要、也不允许自己
    /// 摸 f32），换算公式与 [`apply_blast`] 的 `per_pixel` 那行同一套边界
    /// 常量：`Fx` 原始值 `/65536` 还原成"格/tick"的浮点值，再 `×60` 换成
    /// 引擎的"格/秒"（`physics.rs` 头注：引擎单位 1 格 = 1 物理单位、tick
    /// 与引擎步长同步 60Hz），最后乘上千分位系数还原成小数。算出的
    /// `(jx, jy)` 交给 [`apply_point_impulse`] 这个不关心单位换算的原语。
    ///
    /// **`coeff` 的量级是这套换算公式的隐含契约，不是随便填的**（TDD 阶段
    /// 实测撞见，写进来防止未来照抄 20.0 这种"看着像 Noita 配置"的数字）：
    /// `apply_blast` 对一整个刚体求和、除以刚体质量后得到的是**平均**速度
    /// 增量；单点冲量不做半径加权，`coeff × velocity` 是直接施于一个点的
    /// 原始冲量，对小刚体（十几到几十像素）而言 `coeff` 稍大就会让
    /// `Δv = J/mass` 冲出合理范围，一两 tick 内把刚体推穿世界边界、被墙
    /// 弹回来，观测到的位移方向反而是错的。`data/spells.ron`/
    /// `common::test_spell_table` 的 `expensive_bolt` 都定在 `0.3`——
    /// `projectile_pushes_a_rigid_body_it_hits` 实测过这个值在 12×12 木箱上
    /// 60 tick 内稳定、方向正确；调大前先跑一遍那条测试。
    pub(crate) fn apply_projectile_impulse(
        &mut self,
        phys: &mut PhysicsWorld,
        x: i32,
        y: i32,
        coeff_milli: i32,
        vel: (Fx, Fx),
    ) {
        fn fx_to_engine_vel(v: Fx) -> f32 {
            (v.0 as f32 / 65536.0) * 60.0
        }
        let coeff = coeff_milli as f32 / 1000.0;
        let (jx, jy) = (coeff * fx_to_engine_vel(vel.0), coeff * fx_to_engine_vel(vel.1));
        self.apply_point_impulse(phys, x, y, jx, jy);
    }

    /// 第 7 步前半（spec §6 对账）：**含睡眠刚体**——凡清单上的格不再是
    /// `material | BODY_FLAG`（被炸成 air、烧尽、被别的刚体抢占）即像素被毁：清位图、
    /// 从清单剔除（否则反盖章会把别人的格写成 air）、标 dirty 入队（按 id 序保持）。
    pub(crate) fn reconcile(&mut self, world: &World) {
        for body in self.list.iter_mut() {
            let before = body.stamped.len();
            let (mask, counter, material) = (&mut body.mask, &mut body.counter, body.material);
            body.stamped.retain(|&(x, y, idx)| {
                let c = world.cell(x, y);
                let alive = c.is_body() && c.material() == material;
                if !alive {
                    mask[idx as usize] = false;
                    counter[idx as usize] = 0;
                }
                alive
            });
            if body.stamped.len() != before && !body.dirty {
                body.dirty = true;
            }
            if body.dirty && !self.reextract_queue.contains(&body.id) {
                self.reextract_queue.push(body.id);
            }
        }
        self.reextract_queue.sort_unstable();
    }

    /// 第 7 步后半（spec §6 重提取，限额 `MAX_REEXTRACT_PER_TICK`）：位图 4 连通分量
    /// 分解——单分量且 ≥ 阈值 ⇒ 就地换形（滞回，id 不变）；多分量 ⇒ ≥ 阈值者各成新
    /// body（继承父变换与速度）、< 阈值者逐像素脱格成粒子；父 body 移除。
    pub(crate) fn reextract(
        &mut self,
        world: &mut World,
        table: &MaterialTable,
        phys: &mut PhysicsWorld,
        stamp: u8,
        spawns: &mut Vec<SpawnRequest>,
    ) {
        let mut budget = MAX_REEXTRACT_PER_TICK;
        while budget > 0 {
            let Some(id) = self.reextract_queue.first().copied() else { break };
            self.reextract_queue.remove(0);
            budget -= 1;
            let Some(pos) = self.list.iter().position(|b| b.id == id) else { continue };
            let (w, h) = (self.list[pos].w as usize, self.list[pos].h as usize);
            let comps = components4(&self.list[pos].mask, w, h);
            let density = table.density(self.list[pos].material) as f32;
            if comps.len() == 1 && comps[0].len() >= MIN_BODY_PIXELS {
                let body = &mut self.list[pos];
                body.dirty = false;
                let rects = body.rects();
                phys.replace_shape(body.handle, &rects, body.pivot(), density);
                continue;
            }
            // 拆分：父 body 出列，按分量序分配新 id / 脱格
            let parent = self.list.remove(pos);
            let (px, py, angle) = phys.transform(parent.handle);
            let ((vx, vy), angvel) = phys.velocity(parent.handle);
            let (bvx, bvy) = (vel_to_fx(vx), vel_to_fx(vy));
            let by_idx: std::collections::BTreeMap<u32, (i32, i32)> =
                parent.stamped.iter().map(|&(x, y, idx)| (idx, (x, y))).collect();
            let mut children: Vec<Body> = Vec::new();
            for comp in comps {
                if comp.len() >= MIN_BODY_PIXELS {
                    let mut mask = vec![false; w * h];
                    let mut counter = vec![0u8; w * h];
                    for &i in &comp {
                        mask[i] = true;
                        counter[i] = parent.counter[i];
                    }
                    let rects = rect_cover(&mask, w, h);
                    let handle = phys.insert_body(&rects, parent.pivot(), density, (px, py), angle, (vx, vy), angvel);
                    let id = self.next_id;
                    self.next_id = self.next_id.wrapping_add(1);
                    let stamped = comp
                        .iter()
                        .filter_map(|&i| by_idx.get(&(i as u32)).map(|&(x, y)| (x, y, i as u32)))
                        .collect();
                    children.push(Body {
                        id,
                        material: parent.material,
                        w: parent.w,
                        h: parent.h,
                        mask,
                        counter,
                        stamped,
                        dirty: false,
                        handle,
                        last_xf: None, // 下一 tick 反盖章/盖章一次，接管格子
                        last_h: NO_SURFACE,
                    });
                } else {
                    for &i in &comp {
                        if let Some(&(x, y)) = by_idx.get(&(i as u32)) {
                            spawns.push(SpawnRequest {
                                material: table.debris_to(parent.material),
                                x: Fx::from_int(x) + HALF_CELL,
                                y: Fx::from_int(y) + HALF_CELL,
                                vx: bvx,
                                vy: bvy,
                            });
                            world.set_cell_stamped(table, x, y, MAT_AIR, stamp);
                        }
                    }
                }
            }
            phys.remove_body(parent.handle);
            self.list.extend(children);
            self.list.sort_by_key(|b| b.id);
        }
    }

    pub(crate) fn remove(&mut self, id: u16, phys: &mut PhysicsWorld) {
        if let Some(pos) = self.list.iter().position(|b| b.id == id) {
            let b = self.list.remove(pos);
            phys.remove_body(b.handle);
        }
        self.reextract_queue.retain(|&q| q != id);
    }

    /// 刚体层哈希（spec §7）：按 id 序折 `(id, material, w, h, mask, counter,
    /// transform bits, linvel bits, angvel bits, sleeping)` + `next_id` + 队列。
    /// 引擎内部状态是派生量，不折（另有 checksum 巡检）。
    pub(crate) fn hash_into(&self, phys: &PhysicsWorld) -> u64 {
        use xxhash_rust::xxh3::Xxh3;
        let mut h = Xxh3::new();
        for b in &self.list {
            h.update(&b.id.to_le_bytes());
            h.update(&[b.material]);
            h.update(&b.w.to_le_bytes());
            h.update(&b.h.to_le_bytes());
            let bits: Vec<u8> = b.mask.iter().map(|&m| m as u8).collect();
            h.update(&bits);
            h.update(&b.counter);
            let (x, y, a) = phys.transform(b.handle);
            let ((vx, vy), av) = phys.velocity(b.handle);
            for f in [x, y, a, vx, vy, av] {
                h.update(&f.to_bits().to_le_bytes());
            }
            h.update(&[phys.is_sleeping(b.handle) as u8]);
            h.update(&b.last_h.to_le_bytes());
        }
        h.update(&self.next_id.to_le_bytes());
        for q in &self.reextract_queue {
            h.update(&q.to_le_bytes());
        }
        h.digest()
    }
}

/// 硬格判定（spec §4，B′）：非 air、非 Gas、非 Liquid、非刚体格、材质非 `body_passable`。
/// 薄包装：本体已抽到 `material::is_solid`（M4 spec §2，与生物碰撞共用），
/// 这里固定传 `include_bodies = false`——刚体做自己的地形缓存时不与自身格碰撞
/// （M3 既定语义，纯搬移不改行为）。
fn is_hard(cell: Cell, table: &MaterialTable) -> bool {
    material::is_solid(cell, table, false)
}

/// 某 chunk 的硬格 → 世界坐标矩形覆盖。
fn terrain_rects(world: &World, table: &MaterialTable, cx: u32, cy: u32) -> Vec<Rect> {
    let n = CHUNK;
    let (ox, oy) = ((cx as usize * n) as i32, (cy as usize * n) as i32);
    let mut mask = vec![false; n * n];
    for ly in 0..n {
        for lx in 0..n {
            mask[ly * n + lx] = is_hard(world.cell(ox + lx as i32, oy + ly as i32), table);
        }
    }
    rect_cover(&mask, n, n)
        .into_iter()
        .map(|r| Rect { x0: r.x0 + ox, y0: r.y0 + oy, x1: r.x1 + ox, y1: r.y1 + oy })
        .collect()
}

/// "沉降液体"：非本体 Liquid 且竖直速度位为 0（Layer G 的 `vel`）。落水流、溅落中的水
/// 带速度，不算——一股水擦过箱子不该给它浮力（决策记录第 16 条）。
fn settled_liquid(world: &World, table: &MaterialTable, x: i32, y: i32) -> bool {
    let c = world.cell(x, y);
    !c.is_body() && c.vel() < SETTLED_VEL_MAX && table.category(c.material()) == Category::Liquid
}

/// 顶面载荷（"5-lite"，决策记录第 16 条）：对每个上方是沉降液体的脚印像素，向上数连续沉降
/// 液体格，只计**高于周围自由面**的部分——判据是"这一行在所有采样列里都不是沉降液体"
/// （没有采样列 ⇒ 全计，如地上箱子顶着一摊水）。不能用"`y < h`"：`h` 只在 AABB 行带内扫，
/// 全淹没的箱子 `h` 就是箱顶那行，上面整根池水柱都会被当成水丘（实测把木箱压到池底）。
/// 返回 `(格数, 加权中心, ρ_liq)`；没有 ⇒ `None`。
fn top_load(world: &World, table: &MaterialTable, body: &Body, cols: &[i32]) -> Option<(f32, (f32, f32), u16)> {
    let (mut n, mut sx, mut sy) = (0f32, 0f32, 0f32);
    let mut mats: Vec<u8> = Vec::new();
    for &(x, y, _) in &body.stamped {
        let mut yy = y - 1;
        while yy >= 0 && settled_liquid(world, table, x, yy) {
            if !cols.iter().any(|&cx| settled_liquid(world, table, cx, yy)) {
                n += 1.0;
                sx += x as f32 + 0.5;
                sy += yy as f32 + 0.5;
                mats.push(world.cell(x, yy).material());
            }
            yy -= 1;
        }
    }
    if n <= 0.0 {
        return None;
    }
    Some((n, (sx / n, sy / n), table.density(most_common(&mut mats))))
}

/// 密封支撑（决策记录第 16 条）：没有侧面液体接触，但底面贴着沉降液体、且**所有**侧面邻格都是
/// 硬格（墙 / 粉末 / 他体）——卡在同宽槽里的塞子压着一段封闭水柱。不可压缩的水托住它
/// （向下速度截断 + 抵消重力）：返回底面接触像素数（阻力系数用）；侧面有空气/液体/气体 ⇒
/// 不密封 ⇒ `None`。
fn sealed_bottom(world: &World, table: &MaterialTable, body: &Body) -> Option<f32> {
    let mut n_bottom = 0usize;
    for &(x, y, _) in &body.stamped {
        for nx in [x - 1, x + 1] {
            let c = world.cell(nx, y);
            if c.is_body() {
                continue;
            }
            let m = c.material();
            if m == MAT_AIR || matches!(table.category(m), Category::Liquid | Category::Gas) {
                return None;
            }
        }
        if settled_liquid(world, table, x, y + 1) {
            n_bottom += 1;
        }
    }
    (n_bottom > 0).then_some(n_bottom as f32)
}

/// 出现最多的材质（并列取 id 小者）；调用方保证非空。
fn most_common(mats: &mut [u8]) -> u8 {
    mats.sort_unstable();
    let mut best = (mats[0], 0usize);
    let mut i = 0;
    while i < mats.len() {
        let mut j = i;
        while j < mats.len() && mats[j] == mats[i] {
            j += 1;
        }
        if j - i > best.1 {
            best = (mats[i], j - i);
        }
        i = j;
    }
    best.0
}

/// 水面线采样（spec §5，方案 1"接触门控"，2026-09-03 决策记录第 14 条）。两步：
///
/// **选列（接触门控）**：遍历脚印像素，左右邻格是非本体 Liquid 即一个接触；每个接触列
/// 沿接触行向外穿过连续液体，取边外第 2 格起最多 `SURFACE_REACH` 格的列为采样列（紧邻列
/// 只在一根远列都没有时兜底）。连通性由接触行保证——隔着玻璃壁的水槽、架高水槽里的水
/// 走不过来（方案 0 的远场采样会把它们当水面，地上的箱子悬浮到水槽水面高度，实测）。
///
/// **读数**：每根采样列在刚体 AABB 行带内**自上而下找第一个 Liquid 格**为该列水面；中途
/// 先碰到刚体格（本体或他体）的列作废（自由面在刚体另一侧、这列看不见）；各列取**最低者**
/// （y 最大）为 `h`；全部作废时退到接触格里最高的一格（至少那么高，偏低、自限）。
/// `ρ_liq` 取接触格里出现最多的液体材质（并列取 id 小者）。一个接触都没有 ⇒ `None`。
///
/// 为什么自上而下而不是从接触向上扫连续液体：刚体旁边的水里满是瞬时气泡（空腔回填、
/// 粒子落格），向上扫会塌到气泡处，"取最低"恰好选中它 ⇒ 力掉档 ⇒ 下沉 ⇒ 循环（实测
/// crate_yard 里木箱/木条前 1500 tick 每 tick 重盖章、弹水 5000 粒、睡不着）。自上而下
/// 对气泡免疫；溅到高处的水只会抬高单列读数，"取最低"把它排除。
/// 为什么不采紧邻列：那是自身溅水与空腔回填首先扰动的地方。
///
/// 已知限制：刚体两侧都贴着墙（正好卡在同宽的槽里）时无接触 ⇒ 无浮力。
/// 返回 `(h, ρ_liq, 采样列)`；采样列交给 `top_load` 判"这一行旁边还是不是水"。
fn surface_line(
    world: &World,
    table: &MaterialTable,
    body: &Body,
    (_, y0, _, _): (i32, i32, i32, i32),
) -> Option<(i32, u16, Vec<i32>)> {
    let is_liquid = |x: i32, y: i32| settled_liquid(world, table, x, y);
    // 接触列 x → (最高接触 y, 最低接触 y, 向外方向 ±1)（BTreeMap 定序）。
    let mut contact: std::collections::BTreeMap<i32, (i32, i32, i32)> = std::collections::BTreeMap::new();
    let mut mats: Vec<u8> = Vec::new();
    for &(x, y, _) in &body.stamped {
        for dir in [-1, 1] {
            let nx = x + dir;
            if !is_liquid(nx, y) {
                continue;
            }
            mats.push(world.cell(nx, y).material());
            contact
                .entry(nx)
                .and_modify(|e| {
                    e.0 = e.0.min(y);
                    if y > e.1 {
                        e.1 = y;
                        e.2 = dir;
                    }
                })
                .or_insert((y, y, dir));
        }
    }
    if contact.is_empty() {
        return None;
    }
    let mut cols: std::collections::BTreeSet<i32> = std::collections::BTreeSet::new();
    for (&x, &(_, y_low, dir)) in &contact {
        for k in 1..=SURFACE_REACH {
            let cx = x + dir * k;
            if !is_liquid(cx, y_low) {
                break;
            }
            cols.insert(cx);
        }
    }
    if cols.is_empty() {
        cols = contact.keys().copied().collect();
    }
    let ya = (y0 - 1).max(0);
    let mut h: Option<i32> = None;
    for &x in &cols {
        let mut y = ya;
        let found = loop {
            let c = world.cell(x, y);
            if c.is_body() {
                break None;
            }
            if is_liquid(x, y) {
                break Some(y);
            }
            if y >= world.height() - 1 {
                break None;
            }
            y += 1;
        };
        if let Some(yf) = found {
            h = Some(h.map_or(yf, |old: i32| old.max(yf)));
        }
    }
    let h = h.unwrap_or_else(|| contact.values().map(|e| e.0).min().expect("非空"));
    Some((h, table.density(most_common(&mut mats)), cols.into_iter().collect()))
}

/// 格 AABB 外扩 `margin` 格所覆盖的 chunk 里，是否有任何一个上一 tick `dirty` 非空
/// （睡眠刚体的浮力评估门控）。
fn any_chunk_dirty(world: &World, (x0, y0, x1, y1): (i32, i32, i32, i32), margin: i32) -> bool {
    let c = CHUNK as i32;
    let (cx0, cy0) = (((x0 - margin).div_euclid(c)).max(0), ((y0 - margin).div_euclid(c)).max(0));
    let (cx1, cy1) = (
        ((x1 + margin).div_euclid(c)).min(world.width_chunks as i32 - 1),
        ((y1 + margin).div_euclid(c)).min(world.height_chunks as i32 - 1),
    );
    for cy in cy0..=cy1 {
        for cx in cx0..=cx1 {
            if !world.chunks[world.chunk_index(cx as usize, cy as usize)].dirty.is_empty() {
                return true;
            }
        }
    }
    false
}

/// 变换后位图的格 AABB（闭区间，未裁剪到世界）。
fn aabb_cells(body: &Body, phys: &PhysicsWorld) -> (i32, i32, i32, i32) {
    let (px, py) = body.pivot();
    let corners = [(0.0, 0.0), (body.w as f32, 0.0), (0.0, body.h as f32), (body.w as f32, body.h as f32)];
    let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (cx, cy) in corners {
        let (wx, wy) = phys.local_to_world(body.handle, (cx - px, cy - py));
        minx = minx.min(wx);
        miny = miny.min(wy);
        maxx = maxx.max(wx);
        maxy = maxy.max(wy);
    }
    (minx.floor() as i32 - 1, miny.floor() as i32 - 1, maxx.ceil() as i32 + 1, maxy.ceil() as i32 + 1)
}

/// 反盖章：按清单把格写回 air，**先读回 counter**（燃烧进度随刚体走，spec §3）。
/// 清单上已不再是本体格的（被炸成 air / 烧尽 / 被别的刚体抢占）⇒ 像素被毁：
/// 清位图位、标 dirty，留给第 7 步重提取。
fn unstamp(body: &mut Body, world: &mut World, table: &MaterialTable, stamp: u8) {
    let stamped = std::mem::take(&mut body.stamped);
    for (x, y, idx) in stamped {
        let c = world.cell(x, y);
        let idx = idx as usize;
        if c.is_body() && c.material() == body.material {
            body.counter[idx] = c.counter();
            world.set_cell_stamped(table, x, y, MAT_AIR, stamp);
        } else {
            body.mask[idx] = false;
            body.counter[idx] = 0;
            body.dirty = true;
        }
    }
}

/// 实心光栅化盖章（spec §3）：AABB 内每格逆映射查位图。被盖住的液体/粉末脱格成
/// 粒子（带刚体速度）；Static 非刚体格与别的刚体格跳过（不盖、不记入清单）。
#[allow(clippy::too_many_arguments)]
fn stamp_body(
    body: &mut Body,
    world: &mut World,
    table: &MaterialTable,
    phys: &PhysicsWorld,
    stamp: u8,
    spawns: &mut Vec<SpawnRequest>,
    (x0, y0, x1, y1): (i32, i32, i32, i32),
) {
    let (px, py) = body.pivot();
    let (w, h) = (body.w as i32, body.h as i32);
    let ((vx, vy), _) = phys.velocity(body.handle);
    let (bvx, bvy) = (vel_to_fx(vx), vel_to_fx(vy));
    // 盖章格用**上一 tick 的戳**：`eval` 以 `stamp == 当前` 判"本 tick 已处理"，
    // 若用当前戳，清醒刚体每 tick 重盖章 ⇒ 其燃烧格永远轮不到 CA 评估，燃烧只在
    // 刚体睡着时推进（Task 4 实测：火场里的箱子烧到 103 像素后卡死）。盖章格是
    // Static、无二次移动风险，与 setup 用 255 让 tick 0 可动是同一招。
    let cell_stamp = stamp.wrapping_sub(1);
    let x0 = x0.max(0);
    let y0 = y0.max(0);
    let x1 = x1.min(world.width() - 1);
    let y1 = y1.min(world.height() - 1);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (lx, ly) = phys.world_to_local(body.handle, (x as f32 + 0.5, y as f32 + 0.5));
            let (i, j) = ((lx + px).floor() as i32, (ly + py).floor() as i32);
            if i < 0 || j < 0 || i >= w || j >= h {
                continue;
            }
            let idx = (j * w + i) as usize;
            if !body.mask[idx] {
                continue;
            }
            let t = world.cell(x, y);
            let tm = t.material();
            if t.is_body() {
                continue; // 别的刚体（id 更小者先盖）
            }
            match table.category(tm) {
                Category::Static if tm != MAT_AIR => continue, // 地形：不覆盖
                Category::Liquid | Category::Powder => {
                    // 排开：脱格成粒子，质量守恒 + 溅射（Noita 语义的一半，spec §1.2 第 4 条）。
                    // 出射速度只带质心线速度、**不带 ω×r**（决策记录第 16 条：试过按格点速度
                    // v + ω×r 出射，横摇中的木条把水甩得不对称 ⇒ 接触列 h 跟着抖 ⇒ 维持横摇，
                    // 3000 tick 一次都睡不着；撤回）。
                    spawns.push(SpawnRequest {
                        material: tm,
                        x: Fx::from_int(x) + HALF_CELL,
                        y: Fx::from_int(y) + HALF_CELL,
                        vx: bvx,
                        vy: bvy,
                    });
                }
                _ => {}
            }
            world.set_cell_stamped(table, x, y, body.material, cell_stamp);
            let c: Cell = world.cell(x, y).with_body(true).with_counter(body.counter[idx]);
            world.set_cell_raw(x, y, c);
            body.stamped.push((x, y, idx as u32));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::MaterialDef;

    const WALL: u8 = 1;
    const WATER: u8 = 3;
    const WOOD: u8 = 4;

    fn table() -> MaterialTable {
        MaterialTable::new(vec![
            MaterialDef::base(0, "air", Category::Static, 0),
            MaterialDef::base(WALL, "wall", Category::Static, 100),
            MaterialDef::base(2, "sand", Category::Powder, 40),
            MaterialDef::base(WATER, "water", Category::Liquid, 16),
            MaterialDef { fire_hp: 50, ..MaterialDef::base(WOOD, "wood", Category::Static, 12) },
        ])
        .unwrap()
    }

    fn setup() -> (World, MaterialTable, PhysicsWorld, Bodies) {
        (World::new(2, 2, 1), table(), PhysicsWorld::new(), Bodies::new())
    }

    fn stamped_cells(world: &World) -> Vec<(i32, i32)> {
        let mut v = Vec::new();
        for y in 0..128 {
            for x in 0..128 {
                if world.cell(x, y).is_body() {
                    v.push((x, y));
                }
            }
        }
        v
    }

    #[test]
    fn spawn_rect_enforces_contracts() {
        let (_, t, mut phys, mut bodies) = setup();
        assert!(!bodies.spawn_rect(&mut phys, &t, WATER, 10, 10, 8, 8, 0), "液体不能当刚体");
        assert!(!bodies.spawn_rect(&mut phys, &t, WOOD, 10, 10, 3, 3, 0), "面积 9 < 12");
        assert!(bodies.spawn_rect(&mut phys, &t, WOOD, 10, 10, 8, 4, 0));
        assert_eq!(bodies.rejected_total, 2);
        assert_eq!(bodies.list[0].id, 0);
        assert_eq!(bodies.next_id, 1);
    }

    /// 首次盖章：24×16 矩形落格数恰 384，全部带 body 标志与材质。
    #[test]
    fn first_stamp_covers_exact_rect() {
        let (mut w, t, mut phys, mut bodies) = setup();
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 30, 24, 16, 0);
        let mut spawns = Vec::new();
        bodies.stamp_all(&mut w, &t, &mut phys, 0, &mut spawns);
        let cells = stamped_cells(&w);
        assert_eq!(cells.len(), 24 * 16);
        assert!(cells.iter().all(|&(x, y)| (20..44).contains(&x) && (30..46).contains(&y)));
        assert_eq!(w.cell(20, 30).material(), WOOD);
        assert_eq!(bodies.list[0].stamped.len(), 384);
        assert!(spawns.is_empty());
    }

    /// 变换未变 ⇒ 第二次 stamp_all 零写入（chunk 的 next_dirty 保持为空）。
    #[test]
    fn unchanged_transform_writes_nothing() {
        let (mut w, t, mut phys, mut bodies) = setup();
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 30, 24, 16, 0);
        let mut spawns = Vec::new();
        bodies.stamp_all(&mut w, &t, &mut phys, 0, &mut spawns);
        for c in w.chunks.iter_mut() {
            c.dirty = c.next_dirty.take();
        }
        // 不 step 引擎，变换不变
        bodies.stamp_all(&mut w, &t, &mut phys, 1, &mut spawns);
        for (ci, c) in w.chunks.iter().enumerate() {
            assert!(c.next_dirty.snapshot().is_empty(), "chunk {ci} 有写入");
        }
    }

    /// 旋转 45° 后实心光栅化无洞：盖章格集合 4 连通为单一分量，且数量≈面积。
    #[test]
    fn rotated_stamp_has_no_holes() {
        let (mut w, t, mut phys, mut bodies) = setup();
        bodies.spawn_rect(&mut phys, &t, WOOD, 40, 40, 24, 16, 0);
        // 让引擎把它转 45°：直接改角速度并步进若干步（无重力干扰：把箱子放远处不落地也无妨）
        let h = bodies.list[0].handle;
        phys.set_velocity_for_test(h, (0.0, 0.0), std::f32::consts::FRAC_PI_4 * 60.0);
        phys.step();
        let mut spawns = Vec::new();
        bodies.stamp_all(&mut w, &t, &mut phys, 0, &mut spawns);
        let cells = stamped_cells(&w);
        let n = cells.len();
        assert!((n as i32 - 384).abs() < 60, "盖章格数 {n} 应接近面积 384");
        // 4 连通：从第一格 BFS 应覆盖全部
        let set: std::collections::BTreeSet<(i32, i32)> = cells.iter().copied().collect();
        let mut seen = std::collections::BTreeSet::new();
        let mut stack = vec![cells[0]];
        while let Some((x, y)) = stack.pop() {
            if !seen.insert((x, y)) {
                continue;
            }
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                if set.contains(&(x + dx, y + dy)) {
                    stack.push((x + dx, y + dy));
                }
            }
        }
        assert_eq!(seen.len(), n, "旋转后盖章格必须 4 连通（无洞、无裂缝）");
    }

    /// 反盖章读回 counter、盖章写回：燃烧进度随刚体走（spec §3）。
    #[test]
    fn counter_roundtrips_through_unstamp_and_stamp() {
        let (mut w, t, mut phys, mut bodies) = setup();
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 30, 24, 16, 0);
        let mut spawns = Vec::new();
        bodies.stamp_all(&mut w, &t, &mut phys, 0, &mut spawns);
        // 模拟 CA 点燃了 (20,30)：写 counter=7
        let c = w.cell(20, 30).with_counter(7);
        w.set_cell_raw(20, 30, c);
        // 让刚体动一下（下落一步），触发反盖章+盖章
        phys.step();
        bodies.stamp_all(&mut w, &t, &mut phys, 1, &mut spawns);
        let b = &bodies.list[0];
        assert_eq!(b.counter[0], 7, "反盖章必须读回 counter");
        // 新脚印里局部像素 0 对应的格 counter 仍是 7
        let (x, y, _) = b.stamped.iter().copied().find(|&(_, _, idx)| idx == 0).unwrap();
        assert_eq!(w.cell(x, y).counter(), 7, "盖章必须写回 counter");
        assert!(w.cell(x, y).is_body());
    }

    /// 被盖住的液体格脱格成粒子（质量守恒，溢出的根据）。
    #[test]
    fn stamping_over_liquid_ejects_particles() {
        let (mut w, t, mut phys, mut bodies) = setup();
        for y in 30..46 {
            for x in 20..44 {
                w.set_cell_stamped(&t, x, y, WATER, 0);
            }
        }
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 30, 24, 16, 0);
        let mut spawns = Vec::new();
        bodies.stamp_all(&mut w, &t, &mut phys, 0, &mut spawns);
        assert_eq!(spawns.len(), 384, "每个被盖住的水格一颗粒子");
        assert!(spawns.iter().all(|s| s.material == WATER));
        assert_eq!(w.count_material(WATER), 0);
    }

    /// 地形格（Static 非刚体）不被覆盖，也不进清单。
    #[test]
    fn stamping_skips_terrain_cells() {
        let (mut w, t, mut phys, mut bodies) = setup();
        w.set_cell_stamped(&t, 25, 35, WALL, 0);
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 30, 24, 16, 0);
        let mut spawns = Vec::new();
        bodies.stamp_all(&mut w, &t, &mut phys, 0, &mut spawns);
        assert_eq!(w.cell(25, 35).material(), WALL);
        assert!(!w.cell(25, 35).is_body());
        assert_eq!(bodies.list[0].stamped.len(), 383);
    }

    /// 硬格掩码排除刚体自身格与 body_passable 材质；液体/气体不算硬格。
    #[test]
    fn terrain_mask_excludes_body_and_passable() {
        let (mut w, t, mut phys, mut bodies) = setup();
        w.set_cell_stamped(&t, 5, 5, WALL, 0);
        w.set_cell_stamped(&t, 6, 5, WATER, 0);
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 20, 8, 4, 0);
        let mut spawns = Vec::new();
        bodies.stamp_all(&mut w, &t, &mut phys, 0, &mut spawns);
        let rects = terrain_rects(&w, &t, 0, 0);
        assert_eq!(rects, vec![Rect { x0: 5, y0: 5, x1: 5, y1: 5 }], "只有 wall 是硬格：{rects:?}");
    }

    /// 缓存：干净 chunk 不重建；dirty 非空才重建；离开范围即清除。
    #[test]
    fn terrain_cache_rebuilds_only_dirty_chunks() {
        let (mut w, t, mut phys, mut bodies) = setup();
        w.set_cell_stamped(&t, 30, 60, WALL, 0);
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 20, 8, 4, 0);
        for c in w.chunks.iter_mut() {
            c.dirty = c.next_dirty.take();
        }
        bodies.refresh_terrain(&w, &t, &mut phys);
        let first = bodies.terrain_rebuilds;
        assert!(first >= 1);
        // 封帧后 dirty 清空 ⇒ 第二次不重建
        for c in w.chunks.iter_mut() {
            c.dirty = c.next_dirty.take();
        }
        bodies.refresh_terrain(&w, &t, &mut phys);
        assert_eq!(bodies.terrain_rebuilds, first, "干净 chunk 不得重建");
        // 该 chunk 有写入 ⇒ 重建
        w.set_cell_stamped(&t, 31, 60, WALL, 1);
        for c in w.chunks.iter_mut() {
            c.dirty = c.next_dirty.take();
        }
        bodies.refresh_terrain(&w, &t, &mut phys);
        assert_eq!(bodies.terrain_rebuilds, first + 1);
    }

    /// 水面线：左邻列水面 40、右邻列 42 ⇒ 取最低者 h = 42；半浸箱子的淹没数 = 位图下半。
    #[test]
    fn surface_line_and_submerged_count() {
        let (mut w, t, mut phys, mut bodies) = setup();
        // 箱子 8×4 放在 (20,40)..(27,43)；紧贴的左列 19 填水到 40、右列 28 填水到 42
        for y in 40..60 {
            w.set_cell_stamped(&t, 19, y, WATER, 0);
        }
        for y in 42..60 {
            w.set_cell_stamped(&t, 28, y, WATER, 0);
        }
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 40, 8, 4, 0);
        let mut spawns = Vec::new();
        bodies.stamp_all(&mut w, &t, &mut phys, 0, &mut spawns);
        let (h, rho, _) = surface_line(&w, &t, &bodies.list[0], aabb_cells(&bodies.list[0], &phys)).unwrap();
        assert_eq!(h, 42, "两列 40/42 ⇒ 取最低者 42");
        assert_eq!(rho, 16);
        let sub = bodies.list[0].stamped.iter().filter(|&&(_, y, _)| y >= 42).count();
        assert_eq!(sub, 8 * 2, "y ≥ 42 的像素 = 下半两行");
    }

    /// 接触门控：隔着一格墙的水不算（方案 0 会穿墙采到）；只堆在箱顶/垫在箱底的水也不算。
    #[test]
    fn surface_line_requires_side_contact() {
        let (mut w, t, mut phys, mut bodies) = setup();
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 40, 8, 4, 0);
        let mut spawns = Vec::new();
        bodies.stamp_all(&mut w, &t, &mut phys, 0, &mut spawns);
        // 右侧 x=28 是墙，x=29.. 是水
        for y in 30..60 {
            w.set_cell_stamped(&t, 28, y, WALL, 0);
            w.set_cell_stamped(&t, 29, y, WATER, 0);
        }
        assert!(surface_line(&w, &t, &bodies.list[0], aabb_cells(&bodies.list[0], &phys)).is_none(), "隔墙的水不得当水面");
        // 箱顶堆水、箱底垫水：都不是侧面接触
        for x in 20..28 {
            w.set_cell_stamped(&t, x, 39, WATER, 0);
            w.set_cell_stamped(&t, x, 44, WATER, 0);
        }
        assert!(surface_line(&w, &t, &bodies.list[0], aabb_cells(&bodies.list[0], &phys)).is_none(), "顶/底邻格的水不得当水面");
        // 左侧贴着一列水（40..59）⇒ 接触成立，h = 40
        for y in 40..60 {
            w.set_cell_stamped(&t, 19, y, WATER, 0);
        }
        assert_eq!(surface_line(&w, &t, &bodies.list[0], aabb_cells(&bodies.list[0], &phys)).unwrap().0, 40);
    }

    /// 向上扫被刚体格挡住的列不算读到水面：左列上方压着另一刚体（挡在 42 之上）、右列通到
    /// 自由面 40 ⇒ 取 40，而不是把被挡的 42 当最低水面。
    #[test]
    fn surface_line_ignores_columns_blocked_by_bodies() {
        let (mut w, t, mut phys, mut bodies) = setup();
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 40, 8, 4, 0);
        bodies.spawn_rect(&mut phys, &t, WOOD, 12, 36, 8, 6, 0); // 覆盖 x 12..19、y 36..41
        let mut spawns = Vec::new();
        bodies.stamp_all(&mut w, &t, &mut phys, 0, &mut spawns);
        for y in 42..60 {
            w.set_cell_stamped(&t, 19, y, WATER, 0);
        }
        for y in 40..60 {
            w.set_cell_stamped(&t, 28, y, WATER, 0);
        }
        assert_eq!(surface_line(&w, &t, &bodies.list[0], aabb_cells(&bodies.list[0], &phys)).unwrap().0, 40);
        // 右列的水撤掉 ⇒ 只剩被挡住的左列，退到下界 42
        for y in 40..60 {
            w.set_cell_stamped(&t, 28, y, 0, 0);
        }
        assert_eq!(surface_line(&w, &t, &bodies.list[0], aabb_cells(&bodies.list[0], &phys)).unwrap().0, 42);
    }

    /// 自上而下读数对水里的气泡免疫：列 19 水 36..59、41 是气泡 ⇒ 读到 AABB 行带顶 38。
    #[test]
    fn surface_line_ignores_bubbles_below_surface() {
        let (mut w, t, mut phys, mut bodies) = setup();
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 40, 8, 4, 0);
        let mut spawns = Vec::new();
        bodies.stamp_all(&mut w, &t, &mut phys, 0, &mut spawns);
        for y in 36..60 {
            if y != 41 {
                w.set_cell_stamped(&t, 19, y, WATER, 0);
                w.set_cell_stamped(&t, 18, y, WATER, 0);
            }
        }
        let aabb = aabb_cells(&bodies.list[0], &phys);
        assert_eq!(aabb.1, 39, "AABB 顶 = 39，行带从 38 起");
        assert_eq!(surface_line(&w, &t, &bodies.list[0], aabb).unwrap().0, 38);
    }

    /// 竖直速度 ≥ 2 格/tick 的液体格（落水流）不算接触；1 格/tick（入水扰动）仍算。
    #[test]
    fn falling_liquid_is_not_a_contact() {
        let (mut w, t, mut phys, mut bodies) = setup();
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 40, 8, 4, 0);
        let mut spawns = Vec::new();
        bodies.stamp_all(&mut w, &t, &mut phys, 0, &mut spawns);
        for y in 30..60 {
            w.set_cell_stamped(&t, 19, y, WATER, 0);
            w.set_cell_vel(19, y, SETTLED_VEL_MAX);
        }
        let aabb = aabb_cells(&bodies.list[0], &phys);
        assert!(surface_line(&w, &t, &bodies.list[0], aabb).is_none(), "下落中的水不是水面");
        for y in 30..60 {
            w.set_cell_vel(19, y, SETTLED_VEL_MAX / 2);
        }
        assert!(surface_line(&w, &t, &bodies.list[0], aabb).is_some(), "被扰动的池水仍算");
    }

    /// 顶面载荷：箱顶 3 行水 ⇒ 无采样列全计 24 格；周围水面到 39 ⇒ 只计高于它的 2 行 16 格。
    #[test]
    fn top_load_counts_only_above_waterline() {
        let (mut w, t, mut phys, mut bodies) = setup();
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 40, 8, 4, 0);
        let mut spawns = Vec::new();
        bodies.stamp_all(&mut w, &t, &mut phys, 0, &mut spawns);
        for y in 37..40 {
            for x in 20..28 {
                w.set_cell_stamped(&t, x, y, WATER, 0);
            }
        }
        let b = &bodies.list[0];
        let (n, (cx, cy), rho) = top_load(&w, &t, b, &[]).unwrap();
        assert_eq!((n, rho), (24.0, 16));
        assert!((cx - 24.0).abs() < 1e-3 && (cy - 38.5).abs() < 1e-3, "中心 ({cx},{cy})");
        // 采样列 30 在 y=39 有水（周围水面到 39）⇒ 只有 37、38 两行算水丘
        w.set_cell_stamped(&t, 30, 39, WATER, 0);
        assert_eq!(top_load(&w, &t, b, &[30]).unwrap().0, 16.0);
        for y in 37..39 {
            w.set_cell_stamped(&t, 30, y, WATER, 0);
        }
        assert!(top_load(&w, &t, b, &[30]).is_none(), "周围水面同高 ⇒ 不是水丘");
    }

    /// 密封支撑：两侧贴墙、底下有沉降水 ⇒ Some(底面接触数)；侧墙开一格空气 ⇒ None。
    #[test]
    fn sealed_bottom_requires_hard_sides_and_liquid_below() {
        let (mut w, t, mut phys, mut bodies) = setup();
        for y in 36..52 {
            w.set_cell_stamped(&t, 19, y, WALL, 0);
            w.set_cell_stamped(&t, 28, y, WALL, 0);
        }
        for y in 44..52 {
            for x in 20..28 {
                w.set_cell_stamped(&t, x, y, WATER, 0);
            }
        }
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 40, 8, 4, 0);
        let mut spawns = Vec::new();
        bodies.stamp_all(&mut w, &t, &mut phys, 0, &mut spawns);
        assert_eq!(sealed_bottom(&w, &t, &bodies.list[0]), Some(8.0));
        w.set_cell_stamped(&t, 19, 41, 0, 0);
        assert_eq!(sealed_bottom(&w, &t, &bodies.list[0]), None, "侧面漏气 ⇒ 不密封");
    }

    /// 爆炸冲量：爆心在箱子左侧 ⇒ 向右推；离得远推得轻；出了半径不推。
    #[test]
    fn blast_pushes_away_and_falls_off_with_distance() {
        fn kick(dist: i32, r: i32) -> (f32, f32) {
            let (mut w, t, mut phys, mut bodies) = setup();
            bodies.spawn_rect(&mut phys, &t, WOOD, 40, 40, 8, 4, 0);
            let mut spawns = Vec::new();
            bodies.stamp_all(&mut w, &t, &mut phys, 0, &mut spawns);
            bodies.apply_blast(&mut phys, 40 - dist, 42, r);
            phys.velocity(bodies.list[0].handle).0
        }
        let (vx_near, vy_near) = kick(4, 20);
        let (vx_far, _) = kick(12, 20);
        let (vx_out, _) = kick(30, 20);
        assert!(vx_near > 0.0, "爆心在左 ⇒ 向右推：vx = {vx_near}");
        assert!(vy_near.abs() < vx_near * 0.2, "同一行的爆心几乎不给竖直分量：vy = {vy_near}");
        assert!(vx_far > 0.0 && vx_far < vx_near, "远处推得轻：{vx_far} < {vx_near}");
        assert_eq!(vx_out, 0.0, "出了半径不推");
    }

    fn stamp_once(bodies: &mut Bodies, w: &mut World, t: &MaterialTable, phys: &mut PhysicsWorld) {
        let mut spawns = Vec::new();
        bodies.stamp_all(w, t, phys, 0, &mut spawns);
    }

    /// 炸掉中间一列 ⇒ 对账清位图、重提取拆成两个新 body（新 id、父移除、速度继承）。
    #[test]
    fn cut_line_splits_into_two_bodies() {
        let (mut w, t, mut phys, mut bodies) = setup();
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 30, 24, 16, 0);
        stamp_once(&mut bodies, &mut w, &t, &mut phys);
        for y in 30..46 {
            w.set_cell_stamped(&t, 32, y, 0, 1); // 中线被"炸"成 air
        }
        bodies.reconcile(&w);
        assert!(bodies.list[0].dirty);
        assert_eq!(bodies.reextract_queue, vec![0]);
        let mut spawns = Vec::new();
        bodies.reextract(&mut w, &t, &mut phys, 1, &mut spawns);
        assert_eq!(bodies.list.len(), 2, "应拆成两块");
        assert_eq!(bodies.list.iter().map(|b| b.id).collect::<Vec<_>>(), vec![1, 2], "新 id、父 id 0 移除");
        let pix: Vec<usize> = bodies.list.iter().map(|b| b.mask.iter().filter(|&&m| m).count()).collect();
        assert_eq!(pix, vec![12 * 16, 11 * 16]);
        assert!(spawns.is_empty());
        assert!(bodies.reextract_queue.is_empty());
    }

    /// 掉一个角（单分量仍 ≥ 阈值）⇒ 就地换形、id 不变（滞回）。
    #[test]
    fn corner_loss_reshapes_in_place() {
        let (mut w, t, mut phys, mut bodies) = setup();
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 30, 24, 16, 0);
        stamp_once(&mut bodies, &mut w, &t, &mut phys);
        w.set_cell_stamped(&t, 20, 30, 0, 1);
        w.set_cell_stamped(&t, 21, 30, 0, 1);
        bodies.reconcile(&w);
        let mut spawns = Vec::new();
        bodies.reextract(&mut w, &t, &mut phys, 1, &mut spawns);
        assert_eq!(bodies.list.len(), 1);
        assert_eq!(bodies.list[0].id, 0, "单分量滞回：id 不变");
        assert_eq!(bodies.list[0].mask.iter().filter(|&&m| m).count(), 384 - 2);
        assert!(!bodies.list[0].dirty);
    }

    /// 小于阈值的碎片脱格成粒子、格置 air。
    #[test]
    fn small_fragment_is_ejected_as_particles() {
        let (mut w, t, mut phys, mut bodies) = setup();
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 30, 24, 16, 0);
        stamp_once(&mut bodies, &mut w, &t, &mut phys);
        // 切下左边 2 列（2×16=32 ≥ 12 会成 body）；改切 1 列外加割断成 1×3 的小块：
        // 把 x=21 整列炸掉，再把 x=20 列只留 y=30..32 三格
        for y in 30..46 {
            w.set_cell_stamped(&t, 21, y, 0, 1);
        }
        for y in 33..46 {
            w.set_cell_stamped(&t, 20, y, 0, 1);
        }
        bodies.reconcile(&w);
        let mut spawns = Vec::new();
        bodies.reextract(&mut w, &t, &mut phys, 1, &mut spawns);
        assert_eq!(spawns.len(), 3, "1×3 碎片三颗粒子");
        assert!(spawns.iter().all(|s| s.material == WOOD));
        assert_eq!(w.cell(20, 30).material(), 0);
        assert_eq!(bodies.list.len(), 1, "大块留下");
        assert_eq!(bodies.list[0].mask.iter().filter(|&&m| m).count(), 22 * 16);
    }

    /// 限额：3 个 dirty 同 tick 只处理 2 个，第 3 个顺延。
    #[test]
    fn reextract_respects_per_tick_budget() {
        let (mut w, t, mut phys, mut bodies) = setup();
        for i in 0..3 {
            bodies.spawn_rect(&mut phys, &t, WOOD, 10 + i * 30, 30, 8, 4, 0);
        }
        stamp_once(&mut bodies, &mut w, &t, &mut phys);
        for i in 0..3 {
            w.set_cell_stamped(&t, 10 + i * 30, 30, 0, 1);
        }
        bodies.reconcile(&w);
        assert_eq!(bodies.reextract_queue, vec![0, 1, 2]);
        let mut spawns = Vec::new();
        bodies.reextract(&mut w, &t, &mut phys, 1, &mut spawns);
        assert_eq!(bodies.reextract_queue, vec![2], "第 3 个顺延到下一 tick");
        bodies.reextract(&mut w, &t, &mut phys, 2, &mut spawns);
        assert!(bodies.reextract_queue.is_empty());
    }

    /// 哈希：同序操作同值；刚体动了值变。
    #[test]
    fn hash_is_pure_and_sensitive() {
        let (mut w, t, mut phys, mut bodies) = setup();
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 30, 24, 16, 0);
        let mut spawns = Vec::new();
        bodies.stamp_all(&mut w, &t, &mut phys, 0, &mut spawns);
        let h1 = bodies.hash_into(&phys);
        assert_eq!(h1, bodies.hash_into(&phys));
        phys.step();
        assert_ne!(h1, bodies.hash_into(&phys));
    }
}
