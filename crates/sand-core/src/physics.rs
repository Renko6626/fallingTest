//! 物理引擎适配层（M3 spec §2 `physics-adapter`、§7 确定性红线）。
//!
//! Rapier2D 的薄封装。**rapier 类型不出本模块**——`body.rs` 只见 [`BodyHandle`]、
//! f32 元组与整数矩形。
//!
//! # 红线（violation = 破坏 P1，评审一票否决）
//! - features 固定 `enhanced-determinism` + `serde-serialize`，**不开** `parallel` /
//!   `simd*`：前者是 IEEE-754 平台上位级确定的前提，后两者会引入求和序差异。
//! - **单线程**步进，固定 `dt = 1/60`、无子步（`IntegrationParameters.dt` 只在
//!   `new` 里设一次）。
//! - 建/删/施力/查询**全部由调用方按 body id 序驱动**；本模块绝不迭代
//!   `RigidBodySet` 来产生任何写入——handle 迭代序不是状态的纯函数。
//! - 浮点只在此模块内活动：进来的是格坐标/整数计数 × 常量（精确可表示），
//!   出去的是 `transform()` 的三个 f32，由 `body.rs` 逆映射成布尔。
//! - 引擎版本锁 `Cargo.lock`（lockstep 本就要求同二进制互联）。
//!
//! # 快照
//! [`PhysicsWorld::snapshot`] 序列化 rapier 内置 `PhysicsWorld`（bodies / colliders /
//! islands / broad phase / narrow phase / joints / 参数；pipeline 与 CCD solver 无状态
//! 不含）。两端同序操作 ⇒ 字节逐位相同（单测钉死）；M6 rollback 决策门据此判定。

use rapier2d::prelude::*;

use crate::geom::Rect;

/// 引擎句柄的不透明包装：`body.rs` 拿着它但看不见 rapier。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BodyHandle(RigidBodyHandle);

/// 每格 = 1 物理单位；重力 = 每 tick² 多少格——与网格 `G_ACCEL = 0.25 格/tick²`
/// 同量级（Layer G Task 2），单位换算：dt = 1/60 s ⇒ g = 0.25 × 60² = 900 格/s²。
pub(crate) const GRAVITY_CELLS_PER_S2: f32 = 900.0;
/// 固定步长（60 Hz tick 同步，无子步）。
pub(crate) const DT: f32 = 1.0 / 60.0;
/// 入睡的线速度阈值（格/s；rapier 的 normalized 口径 = 速度 / 长度尺度，本项目长度尺度 = 1 格）。
/// 带碰撞体的刚体其角阈值由本值 ÷ 尺寸推导：取 1.0 ⇒ 24 格箱子约 0.08 rad/s——真在
/// 翻倒的箱子（α ≈ 26 rad/s²）远超此值，不会被冻在半途；浮体靠分数淹没平滑后能压到此下。
pub(crate) const SLEEP_LINEAR_THRESHOLD: f32 = 1.0;
/// 入睡的角速度阈值（rad/s）。
pub(crate) const SLEEP_ANGULAR_THRESHOLD: f32 = 0.3;

pub(crate) struct PhysicsWorld {
    inner: rapier2d::pipeline::PhysicsWorld,
    /// 地形碰撞体按 chunk 键索引（覆盖式重建），`BTreeMap` 定序（禁默认 hasher）。
    terrain: std::collections::BTreeMap<(u32, u32), Vec<ColliderHandle>>,
}

impl PhysicsWorld {
    pub(crate) fn new() -> Self {
        let mut inner = rapier2d::pipeline::PhysicsWorld::new();
        inner.gravity = Vector::new(0.0, GRAVITY_CELLS_PER_S2); // y 向下为正（网格约定）
        inner.integration_parameters.dt = DT;
        PhysicsWorld { inner, terrain: std::collections::BTreeMap::new() }
    }

    /// 插入动态刚体：`rects` 是**局部**格坐标的闭区间矩形（`geom::rect_cover` 产物，
    /// 相对位图左上角），`pivot` 是局部坐标里的旋转中心（位图中心），`pos` 是
    /// pivot 的世界坐标。每个矩形 → 一个 cuboid 子形状，compound 的局部原点 = pivot。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn insert_body(
        &mut self,
        rects: &[Rect],
        pivot: (f32, f32),
        density: f32,
        pos: (f32, f32),
        angle: f32,
        vel: (f32, f32),
        angvel: f32,
    ) -> BodyHandle {
        let mut rb = RigidBodyBuilder::dynamic()
            .translation(Vector::new(pos.0, pos.1))
            .rotation(angle)
            .linvel(Vector::new(vel.0, vel.1))
            .angvel(angvel)
            .build();
        // 睡眠阈值按本项目单位（格/s）放宽：rapier 缺省 0.05 × length_unit 是米制
        // 口径，在 g = 900 格/s² 下漂浮箱子的力量化极限环（1–5 格/s）永远睡不着。
        rb.activation_mut().normalized_linear_threshold = SLEEP_LINEAR_THRESHOLD;
        rb.activation_mut().angular_threshold = SLEEP_ANGULAR_THRESHOLD;
        let h = self.inner.bodies.insert(rb);
        let coll = ColliderBuilder::compound(Self::compound_shapes(rects, pivot)).density(density).friction(0.5).build();
        self.inner.colliders.insert_with_parent(coll, h, &mut self.inner.bodies);
        BodyHandle(h)
    }

    fn compound_shapes(rects: &[Rect], pivot: (f32, f32)) -> Vec<(Pose, SharedShape)> {
        rects
            .iter()
            .map(|r| {
                let (hx, hy) = (((r.x1 - r.x0 + 1) as f32) * 0.5, ((r.y1 - r.y0 + 1) as f32) * 0.5);
                (Pose::translation(r.x0 as f32 + hx - pivot.0, r.y0 as f32 + hy - pivot.1), SharedShape::cuboid(hx, hy))
            })
            .collect()
    }

    /// 就地换形状（重提取的单分量滞回路径，spec §6）：删旧碰撞体、装新 compound；
    /// 变换与速度不动，质量按新面积重算。
    pub(crate) fn replace_shape(&mut self, h: BodyHandle, rects: &[Rect], pivot: (f32, f32), density: f32) {
        let old: Vec<ColliderHandle> = match self.inner.bodies.get(h.0) {
            Some(rb) => rb.colliders().to_vec(),
            None => return,
        };
        for ch in old {
            self.inner.colliders.remove(ch, &mut self.inner.islands, &mut self.inner.bodies, true);
        }
        let coll = ColliderBuilder::compound(Self::compound_shapes(rects, pivot)).density(density).friction(0.5).build();
        self.inner.colliders.insert_with_parent(coll, h.0, &mut self.inner.bodies);
    }

    pub(crate) fn remove_body(&mut self, h: BodyHandle) {
        self.inner.bodies.remove(
            h.0,
            &mut self.inner.islands,
            &mut self.inner.colliders,
            &mut self.inner.impulse_joints,
            &mut self.inner.multibody_joints,
            true,
        );
    }

    /// 覆盖式设置某 chunk 的静态地形：先删同键旧碰撞体，再按 `rects`（**世界**格坐标）
    /// 建固定 cuboid。空 `rects` 等价于 [`Self::clear_terrain`]。
    pub(crate) fn set_terrain(&mut self, key: (u32, u32), rects: &[Rect]) {
        self.clear_terrain(key);
        let mut handles = Vec::with_capacity(rects.len());
        for r in rects {
            let (hx, hy) = (((r.x1 - r.x0 + 1) as f32) * 0.5, ((r.y1 - r.y0 + 1) as f32) * 0.5);
            let coll = ColliderBuilder::cuboid(hx, hy)
                .translation(Vector::new(r.x0 as f32 + hx, r.y0 as f32 + hy))
                .friction(0.6)
                .build();
            handles.push(self.inner.colliders.insert(coll));
        }
        if !handles.is_empty() {
            self.terrain.insert(key, handles);
        }
    }

    pub(crate) fn clear_terrain(&mut self, key: (u32, u32)) {
        if let Some(hs) = self.terrain.remove(&key) {
            for h in hs {
                self.inner.colliders.remove(h, &mut self.inner.islands, &mut self.inner.bodies, false);
            }
        }
    }

    /// 在世界点 `at` 施加力（本 tick 生效，`step` 后由本模块清零——rapier 的
    /// `add_force` 是持久力，不清会累积）。**不唤醒**：调用方只对清醒刚体施力，
    /// 若这里传 `wake_up = true`，水里的刚体每 tick 被强制唤醒、永远睡不着
    /// （目检修订 2026-09-02 实测）。
    pub(crate) fn apply_force_at(&mut self, h: BodyHandle, f: (f32, f32), at: (f32, f32)) {
        if let Some(rb) = self.inner.bodies.get_mut(h.0) {
            rb.add_force_at_point(Vector::new(f.0, f.1), Vector::new(at.0, at.1), false);
        }
    }

    /// 线阻力：`F = −k · v`（质心处）。不唤醒，理由同上。
    pub(crate) fn apply_drag(&mut self, h: BodyHandle, k: f32) {
        if let Some(rb) = self.inner.bodies.get_mut(h.0) {
            let v = rb.linvel();
            rb.add_force(Vector::new(-k * v.x, -k * v.y), false);
        }
    }

    /// 显式唤醒（浮体的水位变了：睡眠刚体的淹没量与入睡时相差过大，`body.rs`）。
    pub(crate) fn wake(&mut self, h: BodyHandle) {
        if let Some(rb) = self.inner.bodies.get_mut(h.0) {
            rb.wake_up(true);
        }
    }

    /// 角阻力：`τ = −k · ω`。不唤醒。
    pub(crate) fn apply_angular_drag(&mut self, h: BodyHandle, k: f32) {
        if let Some(rb) = self.inner.bodies.get_mut(h.0) {
            let w = rb.angvel();
            rb.add_torque(-k * w, false);
        }
    }

    /// 一步（dt 固定）。步完清掉本 tick 的外力——调用方每 tick 重新施加。
    /// 清力按 handle 迭代：**只清零、不产生任何可观测差异**，允许。
    pub(crate) fn step(&mut self) {
        self.inner.step();
        for (_, rb) in self.inner.bodies.iter_mut() {
            rb.reset_forces(false);
            rb.reset_torques(false);
        }
    }

    /// `(x, y, angle)`，角度弧度、y 向下为正坐标系下逆时针为正（glam 约定）。
    pub(crate) fn transform(&self, h: BodyHandle) -> (f32, f32, f32) {
        let rb = &self.inner.bodies[h.0];
        let p = rb.position();
        (p.translation.x, p.translation.y, p.rotation.angle())
    }

    /// 世界点 → 刚体局部坐标（盖章逆映射用，spec §3）。
    pub(crate) fn world_to_local(&self, h: BodyHandle, p: (f32, f32)) -> (f32, f32) {
        let q = self.inner.bodies[h.0].position().inverse_transform_point(Vector::new(p.0, p.1));
        (q.x, q.y)
    }

    /// 刚体局部点 → 世界坐标（AABB 估算用）。
    pub(crate) fn local_to_world(&self, h: BodyHandle, p: (f32, f32)) -> (f32, f32) {
        let q = self.inner.bodies[h.0].position().transform_point(Vector::new(p.0, p.1));
        (q.x, q.y)
    }

    pub(crate) fn mass(&self, h: BodyHandle) -> f32 {
        self.inner.bodies[h.0].mass()
    }

    /// 截断向下速度（密封支撑：塞子压着不可压缩的封闭水柱，撞上即停）。不唤醒。
    pub(crate) fn stop_downward(&mut self, h: BodyHandle) {
        if let Some(rb) = self.inner.bodies.get_mut(h.0) {
            let v = rb.linvel();
            if v.y > 0.0 {
                rb.set_linvel(Vector::new(v.x, 0.0), false);
            }
        }
    }

    /// 质心处施力（密封支撑用）。不唤醒。
    pub(crate) fn apply_force(&mut self, h: BodyHandle, f: (f32, f32)) {
        if let Some(rb) = self.inner.bodies.get_mut(h.0) {
            rb.add_force(Vector::new(f.0, f.1), false);
        }
    }

    pub(crate) fn velocity(&self, h: BodyHandle) -> ((f32, f32), f32) {
        let rb = &self.inner.bodies[h.0];
        let v = rb.linvel();
        ((v.x, v.y), rb.angvel())
    }

    /// 测试专用：直接设速度（生产路径只经 `spawn_rect` 的初速与外力）。
    #[cfg(test)]
    pub(crate) fn set_velocity_for_test(&mut self, h: BodyHandle, v: (f32, f32), angvel: f32) {
        if let Some(rb) = self.inner.bodies.get_mut(h.0) {
            rb.set_linvel(Vector::new(v.0, v.1), true);
            rb.set_angvel(angvel, true);
        }
    }

    pub(crate) fn is_sleeping(&self, h: BodyHandle) -> bool {
        self.inner.bodies[h.0].is_sleeping()
    }

    /// 整体快照（serde/bincode）。
    pub(crate) fn snapshot(&self) -> Vec<u8> {
        bincode::serialize(&self.inner).expect("rapier PhysicsWorld 序列化不应失败")
    }

    pub(crate) fn restore(&mut self, bytes: &[u8]) -> Result<(), String> {
        let mut w: rapier2d::pipeline::PhysicsWorld =
            bincode::deserialize(bytes).map_err(|e| format!("物理快照反序列化失败：{e}"))?;
        // pipeline / ccd_solver 被 serde 跳过 ⇒ 反序列化后是默认值，正是"无状态"的含义。
        w.integration_parameters.dt = DT;
        self.inner = w;
        Ok(())
    }

    /// 快照 checksum（SyncTest 巡检用，spec §7）。
    pub(crate) fn checksum(&self) -> u64 {
        xxhash_rust::xxh3::xxh3_64(&self.snapshot())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floor() -> Vec<Rect> {
        vec![Rect { x0: 0, y0: 100, x1: 255, y1: 103 }]
    }

    fn crate_rects() -> Vec<Rect> {
        vec![Rect { x0: 0, y0: 0, x1: 23, y1: 15 }]
    }

    fn build_scene() -> (PhysicsWorld, Vec<BodyHandle>) {
        let mut w = PhysicsWorld::new();
        w.set_terrain((0, 1), &floor());
        let hs = (0..3)
            .map(|i| w.insert_body(&crate_rects(), (12.0, 8.0), 12.0, (40.0 + 30.0 * i as f32, 20.0), 0.0, (0.0, 0.0), 0.0))
            .collect();
        (w, hs)
    }

    /// spec §7：两个世界同序操作、同步数 ⇒ 快照字节逐位相同。
    #[test]
    fn same_ops_same_snapshot_bytes() {
        let (mut a, _) = build_scene();
        let (mut b, _) = build_scene();
        for _ in 0..600 {
            a.step();
            b.step();
        }
        assert_eq!(a.snapshot(), b.snapshot(), "同序操作的两世界快照必须逐位相同");
        assert_eq!(a.checksum(), b.checksum());
    }

    /// 验收 4 的引擎侧：快照 → 恢复 → 续跑 300 步，与不恢复连续跑 900 步逐位相同。
    #[test]
    fn snapshot_restore_continues_bit_identically() {
        let (mut a, ha) = build_scene();
        let (mut b, hb) = build_scene();
        for _ in 0..600 {
            a.step();
            b.step();
        }
        let snap = a.snapshot();
        let mut c = PhysicsWorld::new();
        c.restore(&snap).unwrap();
        for _ in 0..300 {
            a.step();
            b.step();
            c.step();
        }
        for (i, h) in ha.iter().enumerate() {
            let ta = a.transform(*h);
            let tb = b.transform(hb[i]);
            let tc = c.transform(*h); // 恢复后 handle 保持（RigidBodySet 序列化含 arena 索引）
            assert_eq!(ta.0.to_bits(), tb.0.to_bits());
            assert_eq!(ta.0.to_bits(), tc.0.to_bits(), "body {i} x 恢复后不一致");
            assert_eq!(ta.1.to_bits(), tc.1.to_bits(), "body {i} y 恢复后不一致");
            assert_eq!(ta.2.to_bits(), tc.2.to_bits(), "body {i} angle 恢复后不一致");
        }
        assert_eq!(a.snapshot(), c.snapshot(), "续跑后整体快照仍应逐位相同");
    }

    /// 箱子落到地板上静止后应入睡（spec §3 零写入的前提）。
    #[test]
    fn crate_on_floor_falls_asleep() {
        let (mut w, hs) = build_scene();
        for _ in 0..600 {
            w.step();
        }
        for h in hs {
            let (x, y, _) = w.transform(h);
            assert!(y < 100.0 && y > 80.0, "箱子应停在地板上方：y={y}");
            assert!((20.0..=260.0).contains(&x));
            assert!(w.is_sleeping(h), "静止的箱子应入睡");
        }
    }

    /// 地形覆盖式重建：同键 set 两次不会累积碰撞体。
    #[test]
    fn set_terrain_is_overwrite_not_append() {
        let mut w = PhysicsWorld::new();
        w.set_terrain((0, 0), &floor());
        w.set_terrain((0, 0), &floor());
        assert_eq!(w.inner.colliders.len(), 1);
        w.clear_terrain((0, 0));
        assert_eq!(w.inner.colliders.len(), 0);
        assert!(w.terrain.is_empty());
    }
}
