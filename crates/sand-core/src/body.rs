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
use crate::geom::{rect_cover, Rect};
use crate::material::{Category, MaterialTable, MAT_AIR};
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

    /// 当前位图的矩形覆盖（Task 4 重提取重建碰撞形状用）。
    #[allow(dead_code)]
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
        let handle = phys.insert_body(&rects, pivot, table.density(material) as f32, pos, (0.0, 0.0), 0.0);
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
            world.set_cell_stamped(table, x, y, body.material, stamp);
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
