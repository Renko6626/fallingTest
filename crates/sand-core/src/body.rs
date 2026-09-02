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
use crate::material::{Category, MaterialTable, MAT_AIR};
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
/// 线阻力系数（spec §5）：`F = −K_DRAG × n_sub × v`。取 200：对 16×12 木箱
/// （浮力"弹簧"ω ≈ 10 rad/s）阻尼比 ≈ 0.8，近临界——入水后一两个来回即静止。目检可调。
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

    /// 生成矩形刚体（spec §8）：`(x, y)` 左上角格坐标，`w×h` 格。契约：材质 Static、
    /// 面积 ≥ `MIN_BODY_PIXELS`、数量 < `MAX_BODIES`；违反即确定性拒绝并计数。
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
        let handle = phys.insert_body(&rects, pivot, table.density(material) as f32, pos, 0.0, (0.0, 0.0), 0.0);
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

    /// 第 3 步前半之二（spec §5，采样式阿基米德）：对每个清醒刚体，水面线采样得
    /// `h`，淹没像素 = 上次盖章脚印中 `y ≥ h` 者；`F_浮 = n_sub × ρ_liq × g` 逆重力施于
    /// 淹没质心，阻力 `−K_DRAG × n_sub × v`。全整数计数，进引擎前才转 f32。
    pub(crate) fn apply_buoyancy(&mut self, world: &World, table: &MaterialTable, phys: &mut PhysicsWorld) {
        for body in &self.list {
            if body.stamped.is_empty() || phys.is_sleeping(body.handle) {
                continue;
            }
            let (x0, y0, x1, y1) = aabb_cells(body, phys);
            let Some((h, rho)) = surface_line(world, table, (x0, y0, x1, y1)) else {
                continue;
            };
            let (mut n, mut sx, mut sy) = (0i64, 0i64, 0i64);
            for &(x, y, _) in &body.stamped {
                if y >= h {
                    n += 1;
                    sx += x as i64;
                    sy += y as i64;
                }
            }
            if n == 0 {
                continue;
            }
            let centroid = ((sx as f32 + 0.5 * n as f32) / n as f32, (sy as f32 + 0.5 * n as f32) / n as f32);
            let f_up = n as f32 * rho as f32 * GRAVITY_CELLS_PER_S2;

            phys.apply_force_at(body.handle, (0.0, -f_up), centroid);
            phys.apply_drag(body.handle, K_DRAG * n as f32);
        }
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
                    });
                } else {
                    for &i in &comp {
                        if let Some(&(x, y)) = by_idx.get(&(i as u32)) {
                            spawns.push(SpawnRequest {
                                material: parent.material,
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
        }
        h.update(&self.next_id.to_le_bytes());
        for q in &self.reextract_queue {
            h.update(&q.to_le_bytes());
        }
        h.digest()
    }
}

/// 硬格判定（spec §4，B′）：非 air、非 Gas、非 Liquid、非刚体格、材质非 `body_passable`。
fn is_hard(cell: Cell, table: &MaterialTable) -> bool {
    let m = cell.material();
    m != MAT_AIR
        && !cell.is_body()
        && !matches!(table.category(m), Category::Gas | Category::Liquid)
        && !table.body_passable(m)
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

/// 有序样本的"偶数个取较高者"中位数（y 向下为正 ⇒ 较高 = 较小）。
fn median_high(sorted: &[i32]) -> i32 {
    debug_assert!(!sorted.is_empty());
    sorted[(sorted.len() - 1) / 2]
}

/// 水面线采样（spec §5）：只采箱子**两侧各紧邻 2 列**（脚印之外），在 AABB 行范围
/// 内自上而下找首个 Liquid 格；各列水面 y 取中位数（偶数个取较高者），`ρ_liq` 取样本
/// 里出现最多的液体材质（并列取 id 小者）的 `density`。一列都没有 ⇒ `None`。
///
/// 不采 AABB 内部的列：被排开、溅到**箱子顶上**的水会被误认成水面，产生"越浮越高"
/// 的正反馈（Task 3 实测把箱子弹出水面）。
fn surface_line(world: &World, table: &MaterialTable, (x0, y0, x1, y1): (i32, i32, i32, i32)) -> Option<(i32, u16)> {
    let mut ys: Vec<i32> = Vec::new();
    let mut mats: Vec<u8> = Vec::new();
    let (ya, yb) = ((y0 - 1).max(0), (y1 + 1).min(world.height() - 1));
    // `aabb_cells` 已各向外扩 1 格：x0/x1 本身就是紧邻箱子的第一列，再各取一列。
    let cols = [x0 - 1, x0, x1, x1 + 1];
    for x in cols {
        if x < 0 || x >= world.width() {
            continue;
        }
        for y in ya..=yb {
            let c = world.cell(x, y);
            if c.is_body() {
                continue;
            }
            if table.category(c.material()) == Category::Liquid {
                ys.push(y);
                mats.push(c.material());
                break;
            }
        }
    }
    if ys.is_empty() {
        return None;
    }
    ys.sort_unstable();
    let h = median_high(&ys);
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
    Some((h, table.density(best.0)))
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
                    // 排开：脱格成粒子，质量守恒 + 溅射（Noita 语义的一半，spec §1.2 第 4 条）
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
        assert!(!bodies.spawn_rect(&mut phys, &t, WATER, 10, 10, 8, 8), "液体不能当刚体");
        assert!(!bodies.spawn_rect(&mut phys, &t, WOOD, 10, 10, 3, 3), "面积 9 < 12");
        assert!(bodies.spawn_rect(&mut phys, &t, WOOD, 10, 10, 8, 4));
        assert_eq!(bodies.rejected_total, 2);
        assert_eq!(bodies.list[0].id, 0);
        assert_eq!(bodies.next_id, 1);
    }

    /// 首次盖章：24×16 矩形落格数恰 384，全部带 body 标志与材质。
    #[test]
    fn first_stamp_covers_exact_rect() {
        let (mut w, t, mut phys, mut bodies) = setup();
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 30, 24, 16);
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
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 30, 24, 16);
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
        bodies.spawn_rect(&mut phys, &t, WOOD, 40, 40, 24, 16);
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
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 30, 24, 16);
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
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 30, 24, 16);
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
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 30, 24, 16);
        let mut spawns = Vec::new();
        bodies.stamp_all(&mut w, &t, &mut phys, 0, &mut spawns);
        assert_eq!(w.cell(25, 35).material(), WALL);
        assert!(!w.cell(25, 35).is_body());
        assert_eq!(bodies.list[0].stamped.len(), 383);
    }

    #[test]
    fn median_high_takes_higher_of_even_pair() {
        assert_eq!(median_high(&[10, 20]), 10, "偶数个取较高者（y 小）");
        assert_eq!(median_high(&[10, 20, 30]), 20);
        assert_eq!(median_high(&[5]), 5);
    }

    /// 硬格掩码排除刚体自身格与 body_passable 材质；液体/气体不算硬格。
    #[test]
    fn terrain_mask_excludes_body_and_passable() {
        let (mut w, t, mut phys, mut bodies) = setup();
        w.set_cell_stamped(&t, 5, 5, WALL, 0);
        w.set_cell_stamped(&t, 6, 5, WATER, 0);
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 20, 8, 4);
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
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 20, 8, 4);
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

    /// 水面线：三列水面 y = 40/42/44 ⇒ h = 42；半浸箱子的淹没数 = 位图下半。
    #[test]
    fn surface_line_and_submerged_count() {
        let (mut w, t, mut phys, mut bodies) = setup();
        // 箱子 8×4 放在 (20,40)..(27,43)，两侧列各填水到不同高度
        for y in 40..60 {
            w.set_cell_stamped(&t, 19, y, WATER, 0);
        }
        for y in 42..60 {
            w.set_cell_stamped(&t, 28, y, WATER, 0);
        }
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 40, 8, 4);
        let mut spawns = Vec::new();
        bodies.stamp_all(&mut w, &t, &mut phys, 0, &mut spawns);
        let (x0, y0, x1, y1) = aabb_cells(&bodies.list[0], &phys);
        let (h, rho) = surface_line(&w, &t, (x0, y0, x1, y1)).unwrap();
        assert_eq!(h, 40, "两列 40/42 ⇒ 取较高者 40");
        assert_eq!(rho, 16);
        let sub = bodies.list[0].stamped.iter().filter(|&&(_, y, _)| y >= 42).count();
        assert_eq!(sub, 8 * 2, "y ≥ 42 的像素 = 下半两行");
    }

    fn stamp_once(bodies: &mut Bodies, w: &mut World, t: &MaterialTable, phys: &mut PhysicsWorld) {
        let mut spawns = Vec::new();
        bodies.stamp_all(w, t, phys, 0, &mut spawns);
    }

    /// 炸掉中间一列 ⇒ 对账清位图、重提取拆成两个新 body（新 id、父移除、速度继承）。
    #[test]
    fn cut_line_splits_into_two_bodies() {
        let (mut w, t, mut phys, mut bodies) = setup();
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 30, 24, 16);
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
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 30, 24, 16);
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
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 30, 24, 16);
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
            bodies.spawn_rect(&mut phys, &t, WOOD, 10 + i * 30, 30, 8, 4);
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
        bodies.spawn_rect(&mut phys, &t, WOOD, 20, 30, 24, 16);
        let mut spawns = Vec::new();
        bodies.stamp_all(&mut w, &t, &mut phys, 0, &mut spawns);
        let h1 = bodies.hash_into(&phys);
        assert_eq!(h1, bodies.hash_into(&phys));
        phys.step();
        assert_ne!(h1, bodies.hash_into(&phys));
    }
}
