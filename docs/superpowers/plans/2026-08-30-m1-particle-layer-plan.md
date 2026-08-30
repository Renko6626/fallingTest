# M1 粒子层实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> 文档路径：`docs/superpowers/plans/2026-08-30-m1-particle-layer-plan.md`
> 最近更新：2026-08-30 (UTC+8)
> **粒度说明**：用户裁决本计划走粗粒度——任务级拆分 + 接口契约 + 验证命令；
> 实现细节以 spec 为准，执行者读两份文档。

**Goal:** 实现 Layer P 稀疏粒子层最小闭环：Op::Emit/Op::Explode 生成 → 并行积分 + DDA → 串行按 id 落格，过六配置 SyncTest 验收。

**Architecture:** SoA 粒子池挂在 Sim 上，粒子相插入 tick 管线第 5 步（网格四相之后）；积分/DDA 纯函数并行、提交串行定序；爆炸走 Noita 射线模型与 DDA 共用穿越基建。

**Tech Stack:** Rust（sand-core / sand-harness）、rayon、xxh3、RON。

**Spec:** `docs/superpowers/specs/2026-08-30-m1-particle-layer-design.md`（各任务引用其 § 号）

## Global Constraints

- 确定性红线全文见 `CLAUDE.md` §5 与总纲 §6：核心零 I/O / 禁 std HashMap / 禁浮点入核（harness 加载期量化除外，spec §7）/ RNG 一律 `rng_u32(fseed, stream, x, y, salt, attempt)` 显式 stream。
- tick 管线顺序 = 协议：本次为**新增**第 5 步粒子相，不重排既有阶段；落地时须在总纲 §11 决策日志加条目（Task 7）。
- 每个任务完成 = `cargo test` + `cargo clippy` 全绿 + commit；行为断言过 SyncTest 才算数。
- subagent 禁调 Godot CLI；只做静态写 + Edit + cargo 校验 + commit。
- 常量初值（spec §2）：`GRAVITY = Fx::from_ratio(1,4)`，`MAX_SPEED = 16`，`MAX_PARTICLES = 65536`。

---

### Task 1: 动工前 bench 基线

**Files:** Create `docs/perf/2026-08-30-m0-rust-baseline.md`；不改代码。

- [ ] release 构建后跑既有 harness bench/hashrun 路径：`data/scenarios/` 下 dense（mixed/acceptance）、`sparse.ron`、睡眠常态各测 3 次取中位，记录 {1, 8, 16} 线程 × {ChunkSleep, LiveRect}。
- [ ] 结果按 `docs/perf/2026-08-30-m0-rust-informal.md` 的表格式落正式基线文档（注明 CPU、commit hash、命令行），informal 文档头部加"已被正式基线取代"指针。
- [ ] CHANGELOG 落账 + commit。

### Task 2: 定点基建 `fixed.rs`

**Files:** Create `crates/sand-core/src/fixed.rs`；Modify `crates/sand-core/src/lib.rs`（挂模块）。

**Produces（后续任务全部依赖）:**
```rust
pub struct Fx(pub i32);            // Q16.16
impl Fx {
    pub const ZERO: Fx;
    pub fn from_int(v: i32) -> Fx;
    pub fn from_ratio(num: i32, den: i32) -> Fx;
    pub fn to_cell(self) -> i32;   // floor
    pub fn mul_int(self, k: i32) -> Fx;
    pub fn mul(self, o: Fx) -> Fx; // i64 中间量 >> 16
}
// + Add/Sub/Neg/PartialOrd derive 或手动 impl
pub fn isqrt(v: u64) -> u32;
```

- [ ] TDD：金值单测先行（`from_ratio(1,4)` 位模式、负数 floor、`mul` 舍入、`isqrt` 边界 0/1/完全平方/u32::MAX²）→ 实现 → 全绿。
- [ ] clippy 全绿 + commit。

### Task 3: 粒子池 + 哈希并入 + golden 重录

**Files:** Create `crates/sand-core/src/particle.rs`；Modify `lib.rs`（Sim 挂 `Particles` + 粒子相骨架：生成/压缩，暂无运动）、`hash.rs`（combine 网格根 + 粒子层）。

**Produces:**
```rust
pub struct Particles { /* SoA，见 spec §3 */ }
impl Particles {
    pub fn spawn(&mut self, material: u8, x: Fx, y: Fx, vx: Fx, vy: Fx) -> bool; // 容量拒绝返回 false
    pub fn len(&self) -> usize;
    pub fn hash_into(&self) -> u64;  // spec §9 折叠口径
}
```

- [ ] 单测：spawn 顺序即遍历序、第 65537 个被拒且重跑一致、空池哈希稳定。
- [ ] **golden 重录（spec §9 两步程序）**：新代码跑两个旧 golden 场景导出网格层逐 tick 哈希序列，与改动前（git stash 或上一 commit 构建）diff 一字不差 → 重录 golden 终态；diff 证据记入本任务 commit message。
- [ ] commit。

### Task 4: 积分 + DDA + 串行落格

**Files:** Create `crates/sand-core/src/dda.rs`；Modify `particle.rs`（积分/提交）、`lib.rs`（粒子相完整接线：并行积分 → 串行提交 → 压缩）。

**Interfaces:**
- Consumes: `Fx`、`Particles::spawn`、`World` 只读 cell 访问、`world.rs` 既有写入路径。
- Produces:
```rust
pub enum Outcome { Land { cx: i32, cy: i32, pos: (Fx, Fx) }, Fly { pos: (Fx, Fx) }, Gone }
pub fn integrate(p: /* 单粒子视图 */, grid: &WorldView) -> Outcome; // 纯函数：重力→clamp→DDA
```

- [ ] DDA 语义照 spec §5：i64 交叉相乘定边界序、首个非 air 阻挡、候选 = 最后 air 格、出界 Gone。单测：水平/垂直/对角穿越序、阻挡停点。
- [ ] 串行提交照 spec §5：id 序遍历 Outcome，冲突邻格序**上、左、右、左上、右上**，全占则悬浮（pos = L 中心、速度清零）。单测：双粒子同格 id 小者胜 + 邻格降级。
- [ ] 并行积分挂 rayon（与调度器同池）；行为测试：spawn N 粒子自由落体 → 全部落格成堆、格数守恒。
- [ ] 既有 CI SyncTest 全绿（粒子相并入后 {1,N 线程} 必须同哈希）+ commit。

### Task 5: `Op::Emit` + 瀑布场景

**Files:** Modify `crates/sand-core/src/world.rs`（Op 枚举 + 应用）、`rng.rs`（`STREAM_EMIT`）、`crates/sand-harness`（RON 浮点→Fx 量化、Emit 解析）；Create `data/scenarios/waterfall.ron`。

**Produces:**
```rust
Op::Emit { material: u8, x: Fx, y: Fx, vx: Fx, vy: Fx, count: u16, jitter: Fx }
```

- [ ] 抖动结算在 core：`rng_u32(fseed, STREAM_EMIT, x格, y格, salt = i, 0)` 映射到 `[-jitter, +jitter]`；单测：salt 独立性（不同 i 不同骰）。
- [ ] harness：RON 十进制 → Q16.16 round 量化，量化后值入场景指纹（spec §7）。
- [ ] `waterfall.ron`：高处 Emit 水 → 盆地几何，2 万 tick；先小图版并入 CI SyncTest 矩阵，golden 入库。
- [ ] commit。

### Task 6: `Op::Explode` 射线模型 + 爆炸场景

**Files:** Modify `world.rs`（Explode 应用，复用 `dda.rs` 穿越）、`material.rs` + `data/materials.ron`（`blast_cost` 字段：air 0 / water 1 / sand 2 / wall 哨兵 ∞，RON 缺省 1）、`rng.rs`（`STREAM_EXPLODE`）；Create `data/scenarios/explosion_splash.ron`、`data/scenarios/particle_stress.ron`。

**Produces:**
```rust
Op::Explode { x: i32, y: i32, r: i32, power: u32 }
```

- [ ] 算法照 spec §6：Bresenham 圆周定序发射线、逐格耗能、能量 ≥ cost 才摧毁、粒子速度 = 方向 × `MAX_SPEED × 剩余/power` + 抖动、炸过的格按 air 计费不重复生成。
- [ ] 行为测试：薄墙遮挡（墙后逐格完好）、挖坑守恒（炸掉格数 = 生成粒子数，容量内）、同 Op 重跑一致。
- [ ] `explosion_splash.ron`（沙墙 + 水池 + script 定期 Explode，2 万 tick）golden 入库；`particle_stress.ron`（持续高 rate Emit 顶满 64k）bench 专用。
- [ ] commit。

### Task 7: 验收与收尾

**Files:** Modify `docs/perf/`（对照数据）、`docs/overview/kernel-charter.md`（§11 决策日志：管线新增第 5 步粒子相）、`docs/CHANGELOG.md`、spec Status 行 → Implemented；Create `docs/sessions/` 本次条目、GIF 产物（不入库，路径记文档）。

- [ ] **预告后跑**（≥30 秒操作）：release synctest 六配置 × {waterfall, explosion_splash} 各 2 万 tick，零分叉。
- [ ] bench：Task 1 同口径复测网格路径无回退；`particle_stress` 对照总纲 0.8ms/2 万粒子预算，结果落 `docs/perf/`。
- [ ] render GIF 目检四项（spec §0.4），结论记 session 文档；**Godot 无关，仅 harness 渲染器**。
- [ ] 文档四件套（§11 条目、CHANGELOG、session、spec Status）+ commit。

---

## Self-review 备忘

spec §1–§13 逐节对照：§2→Task 2，§3/§9→Task 3，§4/§5→Task 4，§7/§8→Task 5/6，§6→Task 6，§0/§10/§11→各任务测试项 + Task 1/7，§12 记账项无需任务。无占位符；接口签名跨任务一致（Fx/Particles/Outcome/Op 变体均在 Produces 块定义）。
