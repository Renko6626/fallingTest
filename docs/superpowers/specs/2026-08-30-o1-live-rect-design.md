# O1 chunk 内活矩形 · 实现级设计

> 文档路径：`docs/superpowers/specs/2026-08-30-o1-live-rect-design.md`
> 运行时版本：Rust（sand-core M0+）
> 最近更新：2026-08-30 (UTC+8)
> **Status**: Implemented（2026-08-30 当日实施完成。等价性双证：golden 用 LiveRect 重放
> 哈希一字不变 + 六配置 SyncTest 绿。收益实测：稀疏 2.7×、worst 1.2×、睡眠持平——
> 见 `docs/perf/2026-08-30-m0-rust-informal.md` O1 节）

## 0. 目标

回收 Noita 式 sub-chunk 跳过：活跃 chunk 不再全扫 4096 格，只扫「上 tick 标记区 +
本遍扫描中动态生长的区域」。**语义与全扫逐位等价**（golden 不变 + SyncTest 执法），
稀疏活跃场景（细水流、局部连锁——最常见的玩法区间）预期数倍收益。

## 1. 等价性论证（本设计的承重墙）

设全扫访问序 V = 行自下而上、行内按 `(y+tick)` 奇偶定向，每 cell 恰访问一次、不回访。

1. **跳过安全条件**：活矩形扫描按同一 V 序访问其覆盖集的 cell。等价 ⟺ 每个被跳过的
   cell 在全扫中于其访问时刻不可动（评估 = no-op；评估不改状态——只有移动才盖戳，
   spec M0 §1.1 性质继续承重）。
2. **过度包含永远安全**：全扫评估整个 chunk，任何 ⊇「访问时刻可动集」的覆盖集都等价；
   多扫到的不可动 cell 是 no-op，且有效评估的相对序不变（同一 V 序的子序列）。
   → 矩形做粗包络合法，无需位掩码级精度。
3. **跳过者不可动的归纳**：cell c 被跳过 ⇒ c 不在起始矩形（上 tick 末五邻域无变化，
   不可动）且本遍到 c 的访问时刻前无「使其入矩形」的写入。任何能让 c 变可动的写入
   都在 c±1；若该写入发生在 c 的 V 序访问时刻**之前**，扩张规则（§2.3）会把 c 纳入；
   若在**之后**，则全扫访问 c 时写入尚未发生，c 彼时不可动 = no-op ✓。
4. tick 583 反例的覆盖：链式移动沿 V 序传播 → 每步写入都在下一个链节的访问时刻
   之前 → 逐节被扩张纳入。与被否决的「冻结矩形」的本质区别 = 矩形随本遍写入生长。

## 2. 语义定义

### 2.1 扫描模式（InitConfig）

`ScanMode { Full, ChunkSleep, LiveRect }` 替换原 `sleep_skip: bool`：

| 模式 | chunk 调度 | 起始扫描矩形 |
|---|---|---|
| Full | 全部 chunk | FULL |
| ChunkSleep | dirty 非空 ∨ next_dirty 已标记（相位边界重查，同 M0） | FULL |
| LiveRect | 同 ChunkSleep | `dirty ∪ next_dirty 快照`（本地矩形） |

三种模式**语义等价**；LiveRect 为运行默认。

### 2.2 单代码路径

只有一个扫描循环：动态边界 + 起始矩形参数。Full/ChunkSleep 传 FULL——扩张天然
无效（已达上界），行为与 M0 逐位相同。**禁止**为 LiveRect 另写第二套循环。

### 2.3 扩张规则（以 V 序定义"前方"）

本 chunk 扫描任务对**自己 chunk 内**的每次 cell 写入 (wx,wy)，取 ±1 邻域与 chunk 的交：

- **上方行**（ny < 当前行 y）：并入活矩形（扩 y0、x0、x1）。
- **本行、方向前方**（ny == y 且未访问侧）：扩 x 边界；行内循环边界每步重读。
- **本行方向后方 / 下方行**：不纳入本 tick（全扫在写入前已访问，彼时 no-op，§1.3）；
  照常进 next_dirty 等下 tick。

底边 y1 在扫描开始时固定（首行即最底行，向下写入必属已访问区）。
邻 chunk 的写入照常走 next_dirty 原子合并（唤醒语义不变）；活矩形是**任务本地**
变量（`Cell<DirtyRect>`），无并发面。异相写入隔相位屏障，同相邻居写域互斥，
均不触碰本 chunk cell——本 chunk 的活矩形只由自己的写入驱动。

### 2.4 追踪成本

WriteWindow 恒常追踪 own-chunk 写入（几次比较/写），三模式共用；是否消费由起始
矩形决定。不为省这几条指令引入模式分支。

## 3. 执法与验证

1. **golden 不变（最硬证据）**：replay/render 切到 LiveRect 后，既有 golden 哈希
   必须一字不变。变了 = 等价性破产，直接修，禁止重生成 golden。
2. **SyncTest 六配置**：{1, N 线程} × {Full, ChunkSleep, LiveRect}，CI 与验收场景
   照跑。
3. **tick-583 型回归**：CI 场景已含静止液面 + 局部扰动几何，六配置覆盖即为回归。
4. **稀疏 bench**：新增细水流场景（大图低活跃占比），ChunkSleep vs LiveRect 前后
   对照落 `docs/perf/`；worst 场景回归确认无倒退（矩形≈FULL 时追踪开销应可忽略）。

## 4. 波及面

- `sand-core`：`InitConfig.scan`（破坏性改名）、scheduler 起始矩形逻辑、
  window 本地追踪、rules 动态边界循环。
- `sand-harness`：runner/synctest 配置集、main 默认模式。
- 测试：core tests `common::sim` 签名、synctest_ci 六配置、新增稀疏等价测试。
- **M1 前瞻**：粒子落格写入走同一 window 路径，自动参与追踪与 next_dirty——
  但粒子写入发生在网格 pass 之后（管线第 5 步），只影响下 tick 矩形，无本遍扩张
  语义问题。M1 spec 复核此段。

## 5. 风险

| 风险 | 缓解 |
|---|---|
| 行内"前方"判定 off-by-one | 边界每步重读 + 六配置 SyncTest；这是测试集火点 |
| 起始矩形漏合 next_dirty 快照（唤醒 chunk 扫空矩形） | 唤醒判定与起始矩形取同一快照；synctest 长睡唤醒波覆盖 |
| 追踪开销拖慢 worst 场景 | bench 回归口径写入 §3.4 |
