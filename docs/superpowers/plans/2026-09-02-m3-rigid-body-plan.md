# M3 刚体实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> 文档路径：`docs/superpowers/plans/2026-09-02-m3-rigid-body-plan.md`
> 最近更新：2026-09-02 (UTC+8)
> **Status**: Proposed

**Goal:** 像素刚体全链路——`Op::SpawnBody` 生成、实心盖章/反盖章、B′ 地形碰撞、采样式阿基米德浮沉、爆炸/燃烧破坏对账与限额重提取，SyncTest 绿、快照往返恒等。

**Architecture:** `sand-core::physics`（Rapier2D 薄封装，core 之外不见 rapier 类型）+ `sand-core::body`（刚体同步态本体，唯一同时读写 grid 与 physics 的模块）+ `sand-core::geom`（marching squares / DP / 耳切，纯函数）。tick 管线第 3 步（四相前）与第 7 步（粒子相后）从占位变生效，编号不改。

**Tech Stack:** Rust；`rapier2d = "0.35.3"`，features `["enhanced-determinism", "serde-serialize"]`，**不开** `parallel`/`simd*`；xxh3 折叠刚体层入 `state_hash`。

**Spec:** `docs/superpowers/specs/2026-09-02-m3-rigid-body-design.md`

## Global Constraints

- 确定性红线（CLAUDE.md §5 / 总纲 §2、§6）：浮点只许关在 `physics` 模块内；边界 = `transform()` 出 f32 → 逆映射布尔，整数计数 × 常量 → f32 进；一切建/删/施力/查询按 body id 序；单线程步进、`dt = 1/60`、无子步。
- `Cell` bit 23 = `BODY_FLAG`，只做所有权标记，不豁免任何 CA 规则（spec §3）。
- 写域：第 3/7 步是**串行**阶段（在 `Sim::step` 里、四相之外），直接操作 `World`，不经 `WriteWindow`；写入统一走 `World::set_cell_stamped`（盖戳 + 脏矩形 + dir/lifetime 装填）。
- 休眠/静止刚体零写入（spec §3 防抖 + 睡眠）——执法测试 `resting_body_lets_chunk_sleep`。
- 常量集中在 `body.rs` 顶部 `pub const`：`MIN_BODY_PIXELS = 12`、`MAX_REEXTRACT_PER_TICK = 2`、`DP_EPSILON = 0.5`、`TERRAIN_MARGIN = 1`、`K_DRAG`、`MAX_BODIES = 256`。
- golden：`state_hash` 加入刚体层后**全部重录一次**，重录前 `hashrun --grid-only` 证明网格哈希流逐位不变（M1 粒子层先例）。
- 每 Task 收尾：`cargo test --workspace` + `cargo clippy --workspace --all-targets` 绿、spec 进度表打勾、CHANGELOG 落账、单独 commit 合回 master 推送。

---

### Task 1: `physics` 适配层 + `geom` 几何工具

**Files:**
- Modify: `crates/sand-core/Cargo.toml`（加 `rapier2d`、`serde`）；`Cargo.lock` 提交
- Create: `crates/sand-core/src/physics.rs`、`crates/sand-core/src/geom.rs`
- Modify: `crates/sand-core/src/lib.rs`（`mod physics; mod geom;`，不 pub 导出 rapier 类型）

**Interfaces（后续 Task 依赖）:**
```rust
// physics.rs —— trait 隔离；PhysicsWorld 是唯一实现，rapier 类型不出模块
pub(crate) struct BodyHandle(u32);          // 引擎句柄的不透明包装
pub(crate) struct PhysicsWorld { /* RigidBodySet, ColliderSet, IslandManager, BroadPhase, NarrowPhase, ImpulseJointSet, MultibodyJointSet, CCDSolver, IntegrationParameters, PhysicsPipeline, gravity */ }
impl PhysicsWorld {
    pub(crate) fn new() -> Self;                                   // gravity = (0, +G_PIX) 向下为正 y；dt 1/60
    pub(crate) fn insert_body(&mut self, tris: &[[(f32, f32); 3]], density: f32, pos: (f32, f32), vel: (f32, f32), angvel: f32) -> BodyHandle;
    pub(crate) fn remove_body(&mut self, h: BodyHandle);
    pub(crate) fn set_terrain(&mut self, key: (u32, u32), polylines: &[Vec<(f32, f32)>]);   // 覆盖式：同 key 先删后建
    pub(crate) fn clear_terrain(&mut self, key: (u32, u32));
    pub(crate) fn apply_force_at(&mut self, h: BodyHandle, f: (f32, f32), at: (f32, f32));
    pub(crate) fn apply_drag(&mut self, h: BodyHandle, k: f32);      // 线速度 × (−k)
    pub(crate) fn step(&mut self);
    pub(crate) fn transform(&self, h: BodyHandle) -> (f32, f32, f32);   // x, y, angle
    pub(crate) fn velocity(&self, h: BodyHandle) -> ((f32, f32), f32);
    pub(crate) fn is_sleeping(&self, h: BodyHandle) -> bool;
    pub(crate) fn snapshot(&self) -> Vec<u8>;                        // bincode/serde；用作 SyncTest checksum 与 M6 快照
    pub(crate) fn restore(&mut self, bytes: &[u8]) -> Result<(), String>;
}
// geom.rs —— 纯函数，整数/布尔进，顶点 f32 出（唯一浮点产出点，格角坐标精确可表示）
pub(crate) fn marching_squares(mask: &[bool], w: usize, h: usize) -> Vec<Vec<(i32, i32)>>;  // 多轮廓含洞，外轮廓逆时针、洞顺时针，按起点 (y,x) 序
pub(crate) fn douglas_peucker(poly: &[(i32, i32)], eps: f32) -> Vec<(f32, f32)>;
pub(crate) fn ear_clip(poly: &[(f32, f32)], holes: &[Vec<(f32, f32)>]) -> Vec<[(f32, f32); 3]>;  // 遍历序固定
pub(crate) fn components4(mask: &[bool], w: usize, h: usize) -> Vec<Vec<usize>>;  // 4 连通分量，按最小索引序
```

**要点：**
- [ ] Cargo：`rapier2d = { version = "0.35.3", default-features = false, features = ["dim2", "f32", "std", "block-solver", "enhanced-determinism", "serde-serialize"] }`；`serde = { version = "1", features = ["derive"] }`；`bincode = "1"`（快照编码）。首次编译 rapier 预计 2–3 分钟，先预告。Ring 0 铁律注释：rapier 是纯计算库，允许清单同 rayon/xxhash。
- [ ] `physics.rs` 文档头写红线（spec §7）：单线程、固定 dt、按 id 序、handle 迭代序禁止驱动写入、`enhanced-determinism` 不可关。`snapshot` 序列化 `RigidBodySet/ColliderSet/IslandManager/ImpulseJointSet/MultibodyJointSet/BroadPhase/NarrowPhase/CCDSolver/IntegrationParameters`（PhysicsPipeline 无状态，重建）。
- [ ] `physics` 单测（TDD）：① 两个世界同序插入 3 个箱子、各 `step` 600 次 → `snapshot()` 字节逐位相同；② `snapshot → restore → step 300` 与不恢复连续 `step 900` 的 `transform` bits 相同（验收 4 的引擎侧）；③ 箱子落到 polyline 地板上静止后 `is_sleeping` 为真。
- [ ] `geom` 单测（TDD）：marching squares 对 3×3 实心块给 1 条外轮廓、对带 1 格洞的 5×5 给 1 外 + 1 洞；DP 对直线只留端点、对 L 形保留拐点；耳切三角形数 = n−2 且面积和 = 多边形面积（整数网格上精确）；`components4` 对两块不相连区域返回 2 个、按最小索引序。
- [ ] 收尾：spec 进度 Task 1 ✅、CHANGELOG、commit `feat(core): M3 Task 1——physics 适配层（Rapier2D）+ geom 几何工具`。

---

### Task 2: `body` 本体——位图、盖章/反盖章、`Op::SpawnBody`、哈希

**Files:**
- Create: `crates/sand-core/src/body.rs`
- Modify: `cell.rs`（`BODY_FLAG` bit 23 + `is_body()/with_body(bool)`）、`world.rs`（`Op::SpawnBody`）、`lib.rs`（`Sim` 持 `Bodies` + `PhysicsWorld`；`step` 加第 3/7 步骨架；`state_hash` 折入刚体层）、`hash.rs`（`combine3`）、`material.rs`（`body_passable` 字段）、`sand-harness/scenario.rs`（`SpawnBody` RON + `body_passable`）、`data/materials.ron`（wood density 12、`stone` id 8、`body_passable`）
- Test: `body.rs` 单测 + `tests/body_behavior.rs`

**Interfaces:**
```rust
pub const BODY_FLAG: CellRepr = 1 << 23;   // cell.rs；with_stamp/with_vel/with_counter 均不动它（单测钉死）
pub struct Body { pub id: u16, pub material: u8, pub w: u16, pub h: u16, pub occ: Vec<u8>, pub stamped: Vec<(i32, i32)>, pub dirty: bool, handle: BodyHandle, last_xf: (u32, u32, u32) /* to_bits */ }
pub struct Bodies { pub list: Vec<Body>, pub next_id: u16, pub reextract_queue: Vec<u16>, pub rejected_total: u64 }
impl Bodies {
    pub fn spawn_rect(&mut self, phys: &mut PhysicsWorld, table: &MaterialTable, material: u8, x: i32, y: i32, w: u16, h: u16) -> bool; // MAX_BODIES 拒绝
    pub(crate) fn unstamp_all(&mut self, world: &mut World, phys: &PhysicsWorld);   // 清醒且变换变化的 body：写回 air + 读回 counter
    pub(crate) fn stamp_all(&mut self, world: &mut World, table: &MaterialTable, phys: &PhysicsWorld, stamp: u8, spawns: &mut Vec<SpawnRequest>); // 逆映射实心光栅化；被盖液体/粉末 → spawns
    pub fn hash_into(&self, phys: &PhysicsWorld) -> u64;
}
pub enum Op { …, SpawnBody { material: u8, x: i32, y: i32, w: u16, h: u16 } }
```
`Sim::step` 顺序：ops（含 SpawnBody → `spawn_rect`）→ **第 3 步** `unstamp_all → (Task 3 的浮力/地形) → phys.step → stamp_all` → 网格四相（`scheduler::step` 现有）→ 粒子相 → **第 7 步**（Task 4）。注意 `scheduler::step` 目前把 ops 与四相绑在一起：把 ops 循环提到 `Sim::step` 里、`scheduler::step` 只做四相——这是**纯搬移**（M1 收口拆 world.rs 同款），外部可观测顺序不变。

**要点：**
- [ ] `cell.rs`：`BODY_FLAG` + 访问器 + 单测"`with_stamp/with_vel/with_counter/with_dir` 不清 body 位、`pack` 产物为 0"。
- [ ] `body.rs`（TDD 单测）：① 逆映射盖章：24×16 矩形旋转 45° 后盖章格数 = 位图占位数 ± 边缘（每个局部像素至少被一格覆盖 ⇒ **无洞**：对盖章格集合做 4 连通检查为 1 个分量）；② 反盖章读回 counter：手工给某盖章格 `with_counter(7)` → `unstamp` 后 `occ` 对应位 = 8；再 `stamp` → 该格 counter = 7；③ 变换未变 ⇒ 两次 `stamp_all` 之间零写入（用 `chunks[].next_dirty` 为空断言）；④ 被盖液体 → `spawns` 长度 = 覆盖到的液体格数。
- [ ] 哈希：`hash::combine3(grid, particles, bodies)`；`Bodies::hash_into` 按 id 序折 `(id, material, w, h, occ 字节, transform bits, linvel bits, angvel bits, sleeping as u8)` + `next_id` + `reextract_queue`；**无刚体时的刚体层哈希是常量**，但 `combine3` 使既有 golden 全变——按 Global Constraints 的取证程序重录。
- [ ] harness：`OpSpec::SpawnBody`（加载期校验：材质 Static、`w*h >= MIN_BODY_PIXELS`、矩形在世界内）；`MatSpec.body_passable`；materials.ron 改 wood density 12、加 stone。
- [ ] `tests/body_behavior.rs`：`crate_falls_and_rests_on_wall`（落到墙上、`transform` 稳定、全图入睡 = `resting_body_lets_chunk_sleep`）；`crate_rests_on_sand`（B′——本 Task 只有 wall 地形，沙的部分在 Task 3 补）。
- [ ] 收尾：spec 进度 Task 2 ✅、CHANGELOG、golden 重录（取证）、commit `feat(core): M3 Task 2——body 本体、盖章/反盖章、SpawnBody、刚体层入哈希`。

---

### Task 3: 地形碰撞（B′）+ 浮沉

**Files:**
- Modify: `body.rs`（`refresh_terrain`、`apply_buoyancy`）、`chunk.rs`（若需暴露上一 tick `dirty` 快照）
- Test: `body.rs` 单测 + `tests/body_behavior.rs`

**Interfaces:**
```rust
impl Bodies {
    pub(crate) fn refresh_terrain(&mut self, world: &World, table: &MaterialTable, phys: &mut PhysicsWorld); // 刚体 AABB 外扩 TERRAIN_MARGIN chunk：硬格掩码 → marching_squares → DP → set_terrain；按 chunk 缓存（HashSet<key> 用 BTreeSet，禁默认 hasher）；失效 = chunk.dirty 非空 或 本 tick 盖章/反盖章触及；离开范围 clear_terrain
    pub(crate) fn apply_buoyancy(&mut self, world: &World, table: &MaterialTable, phys: &mut PhysicsWorld); // 水面线采样（spec §5）：AABB±1 列扫脚印外首个 Liquid → 中位数 h → n_sub、质心、ρ_liq → apply_force_at + apply_drag
}
```
硬格判定：`m != AIR && category ∉ {Gas, Liquid} && !cell.is_body() && !table.body_passable(m)`。

**要点：**
- [ ] 单测（TDD）：① 硬格掩码排除自身盖章格与 `body_passable` 材质；② 地形缓存：chunk 无写入时第二 tick 不调用 `set_terrain`（用计数器 mock 或 `PhysicsWorld` 内部计数）；③ 水面线：给定一列列液体高度，中位数与偶数取高规则；④ `n_sub` 对半浸箱子 = 位图下半像素数。
- [ ] 行为测试：`crate_rests_on_sand_pile`（沙堆上箱子 y 稳定不下陷）；`wood_crate_floats_stone_crate_sinks`（500 tick 后木箱质心 y < 水面、石箱质心 y > 水面 − 若干）；`full_pool_overflows_when_crate_drops`（池外出现液体格且 水格数 + 液体粒子数 守恒）；`floating_crate_lets_chunk_sleep`（漂浮静止后零写入）。
- [ ] `K_DRAG` 初值目检定（起始 0.05）；记进 `docs/tuning-knobs.md`（顺手把 M2 那批字段也补上——该表已过期）。
- [ ] 收尾：spec 进度 Task 3 ✅、CHANGELOG、commit `feat(core): M3 Task 3——B′ 地形碰撞缓存 + 水面线阿基米德浮沉`。

---

### Task 4: 破坏对账 + 限额重提取 + 燃烧散架

**Files:**
- Modify: `body.rs`（`reconcile`、`reextract`）、`lib.rs`（第 7 步接线）
- Test: 单测 + `tests/body_behavior.rs`

**Interfaces:**
```rust
impl Bodies {
    pub(crate) fn reconcile(&mut self, world: &World);            // 含睡眠 body：stamped 格不再是 material|BODY_FLAG ⇒ 清 occ、dirty、入队
    pub(crate) fn reextract(&mut self, world: &mut World, table: &MaterialTable, phys: &mut PhysicsWorld, stamp: u8, spawns: &mut Vec<SpawnRequest>); // 每 tick ≤ MAX_REEXTRACT_PER_TICK：components4 → ≥ MIN_BODY_PIXELS 成新 body（继承速度、材质；位置按分量质心）/ < 阈值 eject 成粒子；单分量滞回就地换形
}
```

**要点：**
- [ ] 单测（TDD）：① 位图去掉一条中线 → 两个分量、两个新 id、父移除、速度继承；② 单分量掉角 → id 不变、`occ` 更新；③ 3 像素碎片 → `spawns` 3 条、格置 air；④ 队列限额：3 个 dirty 同 tick 只处理 2 个，第 3 个下一 tick。
- [ ] 行为测试：`explosion_splits_crate_in_two`（`Op::Explode` 打中箱子中部，body 数 1 → ≥2）；`burning_crate_shrinks_and_collapses`（火埋进箱子表面，2000 tick 内 `occ` 占位数单调不增、最终 body 消失或碎成粒子——验收 2）；`stamped_cells_burn_like_material`（盖章格能被点燃且 counter 随刚体移动保持）。
- [ ] 收尾：spec 进度 Task 4 ✅、CHANGELOG、commit `feat(core): M3 Task 4——破坏对账、限额重提取、碎片脱格、燃烧散架`。

---

### Task 5: 收口——`crate_yard`、快照往返、bench、总纲、目检

**Files:**
- Create: `data/scenarios/crate_yard.ron`、`crates/sand-harness/tests/golden/crate_yard.golden`、`docs/perf/2026-09-0X-m3-rigid-body.md`
- Modify: `sand-harness/tests/{golden,synctest}.rs`、`runner.rs`（synctest 每 256 tick 比对 `snapshot()` checksum——需 `Sim::physics_checksum()` 公开一个 u64）、`kernel-charter.md` §11（实施期决策第 8 条：第 3/7 步生效、Rapier 选型、B′、刚体可燃、阿基米德、哈希结构变更）、`program-architecture.md` §3（`physics-adapter` 待决项 → 已定 Rapier2D）、`README.md`、spec Status

**要点：**
- [ ] `crate_yard`（256×192，2 万 tick）：木箱×3（一个在沙堆上、一个空中落到墙、一个落进满池）、石箱×1 落池、定时 `Explode` 打中箱子、定时点火烧一个箱子。
- [ ] SyncTest 六配置 2 万 tick 零分叉 + 引擎 checksum 巡检；golden 新录；快照往返测试（`Sim::snapshot_physics/restore_physics` 走一遍后 `state_hash` 序列相同）。
- [ ] bench：三场景（mixed/sparse/acceptance）无刚体对照 `402322e` 前后（第 3/7 步空转成本必须近零）+ `crate_yard` 入档。
- [ ] 文档：总纲 §11、架构 §3、README 优先队列、spec Implemented、CHANGELOG；GIF 目检五项交用户。
- [ ] commit `chore+docs: M3 收口——crate_yard golden/SyncTest、快照往返、bench、总纲 §11`。

## Self-Review

- spec 覆盖：§2 架构→T1/T2；§3 盖章→T2；§4 碰撞→T1 geom + T3 terrain；§5 浮沉→T3；§6 对账→T4；§7 确定性/哈希/快照→T1/T2/T5；§8 op/数据/常量→T2；§9 测试→各 Task + T5；§10 non-goals 未越界。
- 一致性：`PhysicsWorld` 方法名在 T1 定义、T2–T4 使用一致；`Bodies` 四个阶段方法（`unstamp_all/stamp_all/refresh_terrain/apply_buoyancy/reconcile/reextract`）在 `Sim::step` 的调用顺序 = spec §2 第 3/7 步。
- 风险预告：rapier 首次编译慢；`enhanced-determinism` 关掉 SIMD 后性能未知——bench 在 T5 兜底；水面线采样对"箱子横着漂"场景足够，竖直细长物体的中位数估计可能偏——记为目检项。
