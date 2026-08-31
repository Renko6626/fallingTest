# Layer G 运动语义重做：液体色散 + 重力速度积分 + 撞击溅射脱格

> 文档路径：`docs/superpowers/specs/2026-08-31-layer-g-velocity-design.md`
> 运行时版本：Rust（内核）+ Godot 4 + gdext（表现层）
> 最近更新：2026-08-31 (UTC+8)
> **Status**: Proposed（Task 1 已落地；三 Task 全完才转 Implemented，见 §0）

---

## 实施进度

| Task | 状态 | 落账 |
|---|---|---|
| 1 · 液体色散 ≤8 | ✅ **完成（2026-08-31）**，GIF 目检已过用户确认 | CHANGELOG 2026-08-31 块；perf `docs/perf/2026-08-31-layer-g-task1-dispersion.md` |
| 2 · 重力速度积分 | 未开始 | — |
| 3 · 撞击溅射脱格 | 未开始 | — |

**Task 1 实测 vs 预期**：§3.5 的 golden 预期全部兑现（`sand_pile` 逐 tick 哈希逐位不变、仅 `materials_fp` 行变；三个含水场景状态哈希全变）。摊平速度 254 → 96 tick（2.6×）。两处与本文原计划的偏离，均已记 CHANGELOG：

1. **§3.3 说 Task 1「不碰 `window.rs`」，实际动了**——`window.rs:14` 的"M0 实际移动半径 = 1"注释被色散写失效，顺手把 §5 的 r 契约做了色散那一半（`MAX_WRITE_RADIUS = DISPERSION_MAX + 1 <= HALO`，现值 9 ≤ 16）。Task 2 按 §5 扩写为完整不等式。零行为变更。
2. **§3.4 的"缺省行为逐位相同"拿到了比计划更强的证据**：代码改完、`materials.ron` 未动时，四个 golden **原样全绿**——证明 `side()` 重构在 `dispersion=1` 时逐位等价，不依赖任何新写的测试。

**Task 2 开工前须知**：§4.2④（斜滑是否清零速度）与 §6.1①（`MovedSide` 是否触发溅射）两个待定子裁决仍未决，分别在 Task 2 / Task 3 目检后终裁。

---

## 0. 验收标准

三个 Task 各自独立落地、各自一轮验收。全部完成才算本提案 Implemented。

| # | 项 | 判据 |
|---|---|---|
| 1 | `cargo test --workspace` 全绿、`cargo clippy --workspace --all-targets` 零警告 | 每个 Task 结束时各一次 |
| 2 | **零加速旁路取证**（Task 2 专项） | `G_ACCEL = 0` 时 `hashrun --grid-only` 逐 tick 哈希序列与 Task 2 之前**逐位相同** |
| 3 | **线程数不变性**（Task 3 专项） | 同场景 1 / 8 / 16 线程跑出的粒子 id 序列与 `state_hash` 序列逐位相同 |
| 4 | **休眠不变量**（Task 2 专项） | 静止沙堆场景跑 N tick 后所有 chunk 的 `next_dirty` 恒为空 |
| 5 | golden 重录 | 每 Task 一轮，重录前须给出"哪些场景哈希该变、哪些不该变"的预期并实测对照 |
| 6 | SyncTest 零分叉 | 每 Task 一轮，2 万 tick 六配置（总纲 §11 翻案记录第 5 条口径） |
| 7 | GIF 目检 | 每 Task 一轮，结论留用户 |
| 8 | bench 无回退 | 对照 `docs/perf/2026-08-30-m0-rust-baseline.md` 与 `docs/perf/2026-08-30-m1-particle-baseline.md`，超预算须如实记录 |

---

## 1. 背景与范围

### 1.1 这笔债是什么

总纲 `docs/overview/kernel-charter.md:56` 写着 Layer G 的影响半径预算是"**格内移速 ≤ 4 + 液体色散 ≤ 8 + 余量**"，`HALO = 16`（`crates/sand-core/src/window.rs:16`）就是按这个预算取的。实际实现是：

- 移速恒 1——`rules.rs:92` / `rules.rs:100` 的 `displace` 每 tick 至多挪一格；
- 色散恒 1——`rules.rs:113` 的 `side()` 只探一格。

即**总纲的 r 预算用掉了 1/16**。这条偏离在 M1 立项时被显式记账后置（`docs/superpowers/specs/2026-08-30-m1-particle-layer-design.md:305`，"M1 后独立立项"），本提案就是还这笔账。

### 1.2 四项目标（用户裁决 2026-08-31）

1. **视觉手感**：加速下落、砸落感（Noita 调研 `docs/reference/noita-deep-dive.md:168` 称之为"最大的单笔视觉提升"）；
2. **打通 G→P 自然脱格通路**：网格 cell 被高速撞击后自动成为 Layer P 粒子；
3. **还账**：实现追上总纲 §4 措辞；
4. **液体色散 ≤8**：M0 记录的"水面锯齿 / 摊平极慢"顽疾（`docs/sessions/2026-08-30-m0-implementation.md`）的根因。

### 1.3 不做（Non-goals）

- **O3 粉末惯性 / free-falling 标记**：位段预留（§2），语义留 M2 裁决（M1 spec §12）。
- **`Op::Impulse` 之类的外部冲量 Op**：M3 刚体撞击、M4 法术冲量都还没有真实调用方，现在定接口是猜。本提案只抽 `rules` 层内部的 `eject_cell`（§4.4），届时加一个 Op 分支调它即可。
- **粉末色散**：`Category::Powder` 不走 `side()`，本提案不改这一点。
- **密度置换的速率控制**（Noita 调研 §3.5）：与本提案正交，不入范围。
- **二维网格速度**：只存竖直速度。水平运动仍由规则（斜下滑落 + 色散）决定；真正的二维弹道是 Layer P 的职责。

### 1.4 方案选型（已裁决）

| 方案 | 内容 | 裁决 |
|---|---|---|
| **A** | 逐步 step 循环 + Cell 位段存竖直速度（jason.today / Noita §3.1 路线） | ✅ **采纳** |
| B | 竖直冲刺（复用 `dda.rs`）+ 落点单步判定 | ❌ 丢掉"下落途中斜滑"，沙堆形状与安息角会变，而堆体行为是已调好且被 golden 锁住的 |
| C | 材质常量下落速度（无加速） | ❌ 拿不到加速感，直接违背目标 1 |

**采纳 A 的核心理由是行为连续性**：A 在 `v = 0` 时退化为今天的语义（§3.2 ①），因此 Task 2 有一个免费的强回归取证手段（验收 §0 第 2 项）。B 与 C 都是一次性替换现有移动语义，没有这个可退化性。

### 1.5 分期

三个语义变更**分三个 Task 独立落地**，各自重录 golden、各自跑 SyncTest。依据是**可诊断性**：golden 的哈希 diff 只有在单一语义变更下才读得懂（M1 那次正是靠"三个无爆炸场景仅 `materials_fp` 行变、tick 哈希逐位不变"才证明改动隔离干净的，见 CHANGELOG 2026-08-30 块）。三个一起上，一旦 SyncTest 分叉，面对的是三维搜索空间——这正是总纲 §11 记录的 M0 tick-583 教训（"一次只动一个语义层"）。

排序：**色散打头**。它最独立（不碰 Cell 位段、不碰粒子层、不碰调度器）、最便宜、视觉收益立刻可见，且先落地能让 Task 2 的 golden diff 干净。

---

## 2. Cell 位段总规划

`crates/sand-core/src/cell.rs:2` 现状：`bits 0–7 material / 8–15 stamp / 16 dir / 17–31 aux（M0 恒 0）`，剩 15 位。本提案一次性定死后续布局，避免每加一个字段抢一次：

| bits | 宽 | 字段 | 本提案 |
|---|---|---|---|
| 0–7 | 8 | `material` | 不变 |
| 8–15 | 8 | 世代戳 `stamp` | 不变 |
| 16 | 1 | 横向方向记忆 `dir` | 不变 |
| **17–21** | **5** | **`vy` 竖直速度，Q3.2 无符号，单位 ¼ 格/tick** | **Task 2 启用** |
| 22 | 1 | `free_falling`（O3 粉末惯性） | 预留，恒 0，不读不写 |
| 23–31 | 9 | 留白（durability / 染色 / 温度句柄候选） | 未分配 |

**为什么 5 位**：上限 `V_MAX_CELL = 4.0 格/tick = 16 单位`，需表示 `0..=16` → 5 位是硬下限。余量到 7.75，但 §5 的 HALO 不等式会把实际可用上限卡在 7 以内。

**为什么无符号**：粉末与液体只向下运动；向上运动的气体走 Layer F，撞击反弹走 Layer P（脱格成粒子）。省一位。

**常量**（全部落 `cell.rs`，`materials.ron` 不参与——现实重力与密度无关；若日后要做"羽毛飘"，那是空气阻力，属材质字段，不在这里）：

```rust
pub const VEL_SHIFT: u32 = 17;
pub const VEL_BITS: u32 = 5;
pub const VEL_ONE: u8 = 4;              // 1.0 格/tick = 4 个 ¼ 格单位
pub const V_MAX_CELL: u8 = 4 * VEL_ONE; // 16 单位 = 4.0 格/tick
pub const G_ACCEL: u8 = 1;              // 0.25 格/tick²
```

`G_ACCEL = 1` ⇒ 16 tick（267ms）达终端速度，曲线与 jason.today 的 `accel 0.4 / maxSpeed 8`（`docs/reference/noita-deep-dive.md:174`）同构。

**API**：`Cell::vel()` / `Cell::with_vel()` 照 `stamp` / `dir` 现有体例加（`cell.rs:22`–`cell.rs:37`）。`Cell::AIR` 仍是 `Cell(0)`，`cell.rs:57` 的 `air_is_zero` 测试不动。

---

## 3. Task 1 — 液体色散 ≤8

### 3.1 数据

`data/materials.ron` 每材质加 `dispersion: u8`，缺省 1。加载期在 `crates/sand-harness/src/scenario.rs` 校验 `1 <= d <= DISPERSION_MAX(8)`，越界 `Err`——体例仿既有 `quantize_vaporize_threshold` 与 `MAX_EMIT_JITTER_RAW` 校验。

**但 core 侧不能像 `blast_cost` 那样完全免校验**（2026-08-31 评审修订）：`blast_cost` 配错只是爆炸手感不对，`dispersion` 配错（绕过 harness 直接构表塞超界值）会让 `side()` 写出 WriteWindow——debug 构建撞 `window.rs:105` 窗口断言 panic，release 构建变成同相邻 chunk 的数据竞争 → SyncTest 分叉。这是破坏 P4 写域论证的字段，不是手感旋钮，§5 的编译期断言只锁常量、锁不住数据。故 `side()` 的循环上界取 `dispersion.min(DISPERSION_MAX)`（`DISPERSION_MAX` 落 core 侧，与 §5 断言共用同一常量）——把契约从"数据必须合法"降为"代码自证半径 ≤ DISPERSION_MAX"。harness 校验照做（用户可见的报错仍在 I/O 层），clamp 是 core 的最后防线。

初值：`water = 5`（Noita 调研 `docs/reference/noita-deep-dive.md:242` 的参考值：水 ≈5、油更低、岩浆 1–2），`sand` / `wall` / `air` 吃缺省 1。

### 3.2 逻辑

`rules.rs:113` 的 `side()` 从"探 1 格"改为"沿方向 `d` 探 `1..=dispersion`，遇非 AIR 即停，移到**最远可达空格**"：

```
fn side(x, y, c, d) -> bool:
    far = x
    for i in 1..=min(dispersion(c.material()), DISPERSION_MAX):   // clamp = P4 最后防线（§3.1）
        if win.get(x + d*i, y).material() != MAT_AIR: break
        far = x + d*i
    if far == x: return false
    win.set(far, y, c.with_dir(d > 0).with_stamp(stamp))
    win.set(x,   y, AIR.with_stamp(stamp))
    true
```

**方向记忆不变量一字不动**：成功后记忆 = 实际移动方向（2026-06-14 液面冻结修复的 Rust 版语义，M0 spec §4.3）。失败仍翻向试 `-d`（`rules.rs:110`）。

**仍然只入 AIR**，不做密度置换——保持现状，密度置换的速率控制不在本提案范围（§1.3）。

**脏矩形**：`win.set` 只对源格与目标格各调一次 `mark_dirty_around`（`window.rs:107`），中途掠过的格子未写入、不标记。这是正确的——它们的内容没变。

### 3.3 改动面

`crates/sand-core/src/rules.rs`（一个函数）、`crates/sand-core/src/material.rs`（一个字段 + 访问器）、`crates/sand-harness/src/scenario.rs`（加载与校验）、`data/materials.ron`。**不碰** `cell.rs` / `scheduler.rs` / `particle.rs` / `window.rs`。

### 3.4 测试

- 最远可达空格：一行连续 8 格空气，`dispersion = 5` 的水必须落在第 5 格而非第 1 或第 8；
- 遇阻即停：路径第 3 格是 wall 时必须落在第 2 格；
- 方向记忆不变量：移动后 `dir()` == 实际移动方向；翻向路径同样成立；
- 加载期校验：`dispersion = 0` 与 `dispersion = 9` 各一条拒绝测试；
- core 侧 clamp（§3.1 修订）：绕过 harness 直接构造 `dispersion = 20` 的表，断言水的单 tick 水平位移仍 ≤ `DISPERSION_MAX`；
- 缺省行为：未声明 `dispersion` 的材质行为与改动前逐位相同（sand 场景 golden 应零扰动）。

### 3.5 golden 预期

四个 golden 中只有 `sand_pile.golden` 不含 water（实测：`data/scenarios/` 下含 `water` 的场景为 `waterfall{,_ci}` / `explosion{_ci,_splash}` / `mixed` / `particle_stress` / `sparse` / `acceptance`）。故预期是：

- `sand_pile`：**tick 哈希序列逐位不变**，仅 `materials_fp` 行因 `materials.ron` 内容哈希变化而变；
- `mixed` / `waterfall_ci` / `explosion_ci`：状态哈希全变（预期内）。

重录前先跑 `--grid-only` diff 取证，把上述预期与实测对照写进 CHANGELOG——M1 那次正是靠这一步证明改动隔离干净的。

---

## 4. Task 2 — 重力速度积分

### 4.1 主循环

`rules.rs:61` 的 `Ctx::eval` 在通过静止/世代戳判定（`rules.rs:64`）后，外层包一个子步循环：

```
v0 = c.vel()                                    // Q3.2 单位（¼ 格/tick）
v1 = min(v0 + G_ACCEL, V_MAX_CELL)
n  = max(1, v1 / VEL_ONE + frac_roll(v1, x, y)) // 概率取整 = 零存储子像素精度
cur = (x, y); stalled = false
for k in 0..n:
    match one_step(cur, k, c.with_vel(v1)):     // 现有 powder_step / liquid_step 体
        Moved(next)     => cur = next
        MovedSide(next) => { cur = next; stalled = true; break }   // 色散 = 撞停
        Blocked         => { stalled = true; break }
v_final = if stalled { 0 } else { v1 }
if v_final != cell_at(cur).vel():
    set_vel(cur, v_final)                       // ← 唯一写回路径
```

`one_step` 是把现有 `powder_step`（`rules.rs:91`）/ `liquid_step`（`rules.rs:99`）的函数体原样搬进来，返回值从 `()` 改为三态枚举——**判定逻辑一行不改**，只是把"成功/失败"的信息回传给外层循环。

`frac_roll` 必须**纯整数**（总纲 §6 数值红线：网格逻辑禁浮点）。`VEL_ONE = 4` 恰是 2 的幂，取模即可：

```
frac_roll(v1, x, y) -> u8:
    frac = v1 % VEL_ONE                                    // 0..=3
    if rng_u32(fseed, STREAM_FALLSTEP, x, y, 0, 0) % VEL_ONE < frac { 1 } else { 0 }
```

`VEL_ONE` 是 2 的幂 ⇒ `% VEL_ONE` 是无偏取位，不存在取模偏置。

### 4.2 四个要点

**① `n = max(1, ...)` 是可退化性的来源。** `v = 0` 时 `n = 1`，走的就是今天那条路径。把 `G_ACCEL` 设成 0，整个 sim 必须与 Task 2 之前**逐位相同**（验收 §0 第 2 项）——这与 M1 Task 3 的 `--grid-only` 两步取证是同一招（CHANGELOG 2026-08-30 块）。它同时保证"加速是严格叠加在现有语义之上"，而非替换。

**② 写回纪律 = 休眠的生命线。** 只在 `v_final ≠ 已存值` 时写。

这条不是优化而是**正确性红线**：现在静止堆体不写任何 cell → `next_dirty` 为空 → chunk 入睡（`scheduler.rs:74` 的 `dirty ∪ next_dirty` 判空）。若照 jason.today 原样"每 tick `v += accel`"无条件写回，静止沙子的速度会从 0 涨起来 → 每 tick 一次 `set()` → `mark_dirty_around`（`window.rs:107`）→ **整张图永不入睡**，M0 建立的稀疏性能（sparse 场景 LiveRect 1 线程 0.228ms，`docs/perf/2026-08-30-m0-rust-baseline.md`）当场退回全量扫描。

按上面的写法：静止堆体 `v0 = 0` → 第 0 子步即 `Blocked` → `v_final = 0` → 与已存值相同 → **零写入 → 照旧入睡**。快速下落的 cell 落地时写一次 0，之后永远不再写。验收 §0 第 4 项就是这条的执法。

**③ RNG 维度。** 两处：

- `frac_roll` 用**新流** `STREAM_FALLSTEP = 3`（`rng.rs:23` 起的注册表追加，禁止复用既有流），key 取 cell 的**起始坐标**。每 tick 每 cell 只掷一次，而扫描开始时每个网格位置至多一个 cell，故起始坐标天然唯一，不需要 salt/attempt 维度。
- `diag_side`（`rules.rs:87`）的 `attempt` 形参（`rng.rs:63`，现在恒传 0）改传**子步序号 `k`**。这是总纲 §11 翻案记录第 4 条点名要求保留的维度（"同帧同格多次掷骰返回同值的偏置隐患是外部评审确认过的真问题"）。`k = 0` 时值与今天相同，故不破坏 ① 的逐位退化性。

**④ 待目检的子裁决：斜滑要不要清零速度。** 沙在 `v = 4` 时下方被挡、斜下可走，按上面的写法算 `Moved`，循环继续 → 单 tick 斜滑 4 格，沙堆坍塌明显变快。jason.today / Noita 就是这个行为，**默认取它**。若目检认为塌得太夸张，改成"斜滑即 `stalled`"是一行常量的事，水平 r 还会从 4 降到 1（§5 的余量只会变大）。此项列入 Task 2 目检清单，结论落 §7 决策记录。

### 4.3 改动面

`crates/sand-core/src/cell.rs`（位段 API + 常量）、`crates/sand-core/src/rules.rs`（`eval` 外层循环 + `powder_step` / `liquid_step` 改三态返回）、`crates/sand-core/src/rng.rs`（新流常量）、`crates/sand-core/src/window.rs`（§5 的编译期不等式断言 + 文档注释）。**不碰** `particle.rs` / `scheduler.rs` / `material.rs`。

### 4.4 测试

- **零加速旁路取证**（验收 §0 第 2 项）：`G_ACCEL = 0` 重编译 → `hashrun --grid-only` 逐 tick diff 为空。取证文件存 `.superpowers/` 任务目录，路径写进 CHANGELOG。
- **休眠不变量**（验收 §0 第 4 项）：静止沙堆场景跑 N tick，断言所有 chunk 的 `next_dirty` 恒空。
- 终端速度金值：自由落体 20 tick 后 `vel()` 必须 `== V_MAX_CELL`，且第 16 tick 首次达到。
- 撞停清零：高速下落撞 wall 后 `vel() == 0`。
- 子像素概率取整：`v1 = 1.5` 时 `n` 在 1 与 2 之间按 hash 分布，且同 `(tick, x, y)` 复现同值。
- `Cell` 位段往返：`with_vel` / `vel` 往返 + 与 `material` / `stamp` / `dir` 互不干扰（扩写 `cell.rs:45` 的 `bitfield_roundtrip`）。

---

## 5. r ≤ 16 论证（Task 1 + Task 2 合并复审）

单个 cell 单 tick 的最大读写半径，逐条算：

1. 子步循环最多 `n ≤ V_MAX_CELL = 4` 步。世代戳（`rules.rs:64`）保证每 cell 每 tick 只被 `eval` 一次，不会级联叠加。
2. `side()`（色散）一旦走到就 `break`（§4.1 的 `MovedSide`）⇒ **每 tick 至多一次色散**。
3. 故最坏水平位移路径 = `(n − 1)` 次同向斜下 + 1 次色散 = `3 + 8 = 11`；另一条候选是 4 次全斜下 = 4。取 **11**。
4. 最坏竖直位移 = 4。
5. `displace` 探测读半径 = 写半径；`side` 的探测路径 ≤ `dispersion`，同样 ≤ 写半径。
6. `mark_dirty_around`（`window.rs:111`）再 ±1。

⇒ **最大读写半径 `max(11, 4) + 1 = 12 ≤ HALO 16`，余量 4。**

固化为编译期契约，落 `window.rs` 紧邻 `HALO` 定义处：

```rust
const _: () = assert!(
    (V_MAX_CELL / VEL_ONE) as i32 - 1 + DISPERSION_MAX as i32 + 1 <= HALO,
    "r<=16 契约破裂：(V_MAX_CELL-1) + DISPERSION_MAX + 1 必须 <= HALO"
);
```

以后谁把 `V_MAX_CELL` 提到 8 或色散上限提到 12，**编译不过**——不再依赖"新增移动规则必须自证半径"这句人肉纪律（`window.rs:15` 现有注释）。该注释同步改写为指向本断言。

脱格粒子的初速**不受此约束**：粒子层不经 `WriteWindow`，走自己的 DDA 与串行按 id 落格（总纲 §4 Layer P）。

`scheduler.rs:116` 的 `phase_windows_disjoint_and_phases_cover_all` 不需要改——窗口几何没变，变的只是"实际用掉多少窗口"。另加一个 `rules` 层测试：debug 构建下记录每 cell 的起点与终点，断言 `|Δx| <= 11 && |Δy| <= 4`。

---

## 6. Task 3 — 撞击溅射脱格

### 6.1 触发

三条全中才脱格：

1. `stalled == true`（§4.1，本 tick 撞停）——**`Blocked` 与 `MovedSide` 均触发，是有意为之**（2026-08-31 评审修订）：瀑布砸进水面走的正是"下方被挡 → 色散走开"路径（`MovedSide`），这恰是目标 4 要的水花来源。副作用是高速水贴地横流也可能冒向上的水花——此项列入 Task 3 目检清单（§7.1），若目检认为不对，改成"仅 `Blocked` 触发"是一行判别的事；
2. `v1 >= SPLASH_MIN_SPEED`（初值 `2 * VEL_ONE` = 2.0 格/tick）；
3. `rng_u32(fseed, STREAM_SPLASH, sx, sy, 0, 0)` 的量化值 `< splash_chance[material]`，**key 用该 cell 本 tick 的起始坐标 `(sx, sy)`，不是撞停坐标**（2026-08-31 评审修订）。撞停坐标同 tick 内不唯一：cell A 撞停脱格后原格变 AIR（盖戳不阻止它被 `displace` 当目标），上方 cell B 同 tick 落入同格再撞停，若 key 用撞停坐标则掷出同一值——同材质则 A 溅 B 必溅，整列连锁全脱或全停，正是总纲 §11 翻案 4 点名的"同帧同格多骰同值"偏置。起始坐标每 tick 每 cell 唯一（§4.2③ frac_roll 的同一论证），撞停位置只决定粒子出生点，不进 RNG key。

### 6.2 数据

`data/materials.ron` 加 `splash_chance`，RON 写 `0.0..=1.0` 十进制，加载期经 `quantize_splash_chance` 一次性 `×255 round` 量化为 `u8`（负值/超界报错），缺省 `0.0` = 永不溅射——**完全照 `vaporize_threshold` 的体例**（`material.rs:37`，CHANGELOG 2026-08-30 块）。

初值：`water = 0.6`、`sand = 0.1`、其余缺省 0。

### 6.3 效果

网格原位置 `AIR`（盖当前戳），生成一颗粒子：

- 位置 = 撞停格中心；
- `vy` = `−v1 × SPLASH_RESTITUTION`（初值 0.5，向上反弹）；
- `vx` = 水平抖动，直接复用 `emit.rs:92` 的 `emit_jitter`（已是 `pub(crate)`，爆炸路径已复用同一套数学）；
- 抖动掷骰的 `attempt` 用 `SPLASH_ROLL_VX` / `SPLASH_ROLL_VY` 常量区分两骰，体例同 `EXPLODE_ROLL_VX` / `EXPLODE_ROLL_VY`（`rng.rs:32` 注释）；坐标 key 与 §6.1 触发骰同口径——**起始坐标 `(sx, sy)`**，否则同 tick 同撞停点的两颗溅射粒子抖动完全重合。

### 6.4 核心工程：并行 pass 的确定性生成序

现在 `SpawnRequest`（`world.rs:41`）只在**串行的 ops 阶段**产生（`scheduler.rs:51`–`scheduler.rs:53`），入队序 = id 序天然确定（`particle.rs:108` 的 `spawn` 顺序即 id 序）。溅射发生在**并行的四相 pass 里**（`scheduler.rs:84` 的 `par_iter`），直接 push 全局队列必然破坏确定性。

方案：

1. `Chunk`（`chunk.rs:108`）加 `spawn_buf: Vec<SpawnRequest>`。每个 chunk 只写自己那份——安全论证与 `cells` 完全同构（写域互斥由 `WriteWindow::own_ci` 保证，`window.rs:62`）。缓冲跨 tick 复用（`clear()` 而非重新分配）。
2. **每个相位屏障之后**（`scheduler.rs:99` 注释标注的位置）立刻按 **chunk index 升序** drain 该相位所有 chunk 的 `spawn_buf`，追加进 `scheduler::step` 的 `spawns` 出参。
3. 最终 id 序 = `(相位序, chunk index, chunk 内扫描序)`。相位序是 `phase_order(tick)`（`scheduler.rs:16`）的轮换，而 `tick` 是状态的一部分 ⇒ 全链确定，与线程数无关。

`Sim::step`（`lib.rs:122`）的 drain 逻辑一行不改：ops 阶段的请求在前、网格阶段的在后，都在粒子相之前入队。

### 6.5 限流：两道防线，都确定性

| 防线 | 机制 | 为什么确定 |
|---|---|---|
| 本地 | `MAX_SPLASH_PER_CHUNK = 64` / tick / chunk，超出即不溅射（照旧停住） | 本地计数不依赖全局状态，与线程调度无关 |
| 全局 | M1 既有的 `MAX_PARTICLES = 65536` 确定性拒绝（`particle.rs:109`） | 串行 drain 阶段判定 |

640×384 图 60 个 chunk ⇒ 本地防线把最坏情况钉在 3840 粒子/tick。

**质量守恒缺口**：脱格已把网格置 air，若随后被全局容量拒绝，该质量凭空消失。这与 M1 爆炸路径是同一个已知权衡（`world.rs` 的 `Op::Explode` 分支已有就地注释，commit `098fe23`），Task 3 沿用同一注释纪律，就地写明。

### 6.6 `eject_cell`

抽一个 `rules` 层内部函数 `eject_cell(win, x, y, cell, vx, vy, buf)`：置 air + 盖戳 + 往 `spawn_buf` 追加请求。溅射路径走它。**这就是"G→P 通路"本身**——M3 刚体撞击、M4 法术冲量届时加一个 Op 分支调它即可，不预建接口（§1.3）。

### 6.7 测试

- 三条触发条件各一条测试（`stalled` 为假不溅射 / 速度不足不溅射 / 概率为 0 不溅射）；
- RNG key 用起始坐标（§6.1 修订）：构造"A 撞停脱格、B 同 tick 落入同格再撞停"的连锁场景，断言两次触发骰值不同（各自独立），且骰值与撞停坐标无关；
- **线程数不变性**（验收 §0 第 3 项）：同场景 1 / 8 / 16 线程，粒子 id 序列与 `state_hash` 序列逐位相同；
- per-chunk 限流：构造单 chunk 内 100 次同 tick 撞击，断言恰好 64 颗脱格、其余照旧停住；
- 脱格后网格为 air 且粒子数 +1（质量账对齐）；
- 全局容量拒绝路径下 `rejected_total` 递增（复用 `particle.rs:84`）。

### 6.8 改动面

`crates/sand-core/src/{rules,chunk,window,scheduler,material,cell}.rs`、`crates/sand-harness/src/scenario.rs`、`data/materials.ron`。

⚠️ **本 Task 让并行网格 pass 成为粒子生成队列的新写入源。** 外部可观测的 tick 阶段顺序不变（仍是 ops → 网格四相 → 粒子相 → 封帧，`lib.rs:112`），但数据流变了。按 M1 粒子相入管线的先例（总纲 §11 实施期决策第 1 条），这仍须落 §11 留痕。

---

## 7. 验收程序与文档落点

### 7.1 每 Task 一轮

| | Task 1 色散 | Task 2 速度积分 | Task 3 溅射脱格 |
|---|---|---|---|
| 单测 | §3.4 | §4.4 | §6.7 |
| golden | 重录①（预期见 §3.5） | 重录② | 重录③ |
| SyncTest | waterfall + mixed 2 万 tick 六配置 | 同上 | + explosion_splash |
| 目检 | 水面锯齿是否消失 | 加速下落手感 + §4.2④ 子裁决 | 瀑布砸地水花 + §6.1① 子裁决（横流水花是否过量） |
| bench | 对照 M0/M1 基线，记录活跃 cell 更新次数涨幅 | 同左（Task 2 是涨幅主要来源） | 同左 |

**性能预期**：子步循环只对**正在下落**的 cell 生效（静止堆体恒 `n = 1`），故 dense 场景实际涨幅应远小于最坏 ×4。这是预期不是结论——按总纲纪律，须跑 harness-bench 实测并落 `docs/perf/`，超预算如实记录。

### 7.2 文档落点

- 本 spec 落 `docs/superpowers/specs/2026-08-31-layer-g-velocity-design.md`；
- 总纲 `docs/overview/kernel-charter.md` §11 加一条实施期决策：**Cell 位段总规划 + HALO 编译期不等式契约 + 网格 pass 新增 `spawns` 写入源**；
- 总纲 §4 Layer G 的"格内移速 ≤ 4"从预算措辞改为实现描述；
- `docs/CHANGELOG.md` 每 Task 落账（含 golden 预期 vs 实测对照）；
- `docs/README.md` 优先队列同步。

---

## 8. 决策记录

| # | 决策 | 依据 |
|---|---|---|
| 1 | 方案 A（逐步 step 循环 + Cell 位段速度） | §1.4；用户裁决 2026-08-31 |
| 2 | 脱格触发 = 外部冲量 + 高速撞击溅射（重力 clamp 4，自由落体本身不脱格） | 粒子数正比于撞击面宽度而非下落体积，量级可控；用户裁决 2026-08-31 |
| 3 | 分三 Task 独立落地，色散打头 | §1.5；总纲 §11 tick-583 教训 |
| 4 | 竖直速度只存 5 位无符号 Q3.2 | §2 |
| 5 | 速度写回只在值变化时发生 | §4.2②；不这样做会毁掉 chunk 休眠 |
| 6 | 色散走到即 `break`（每 tick 至多一次） | §5 的 r 上界依赖此性质 |
| 7 | r 契约固化为编译期断言而非人肉纪律 | §5 |
| 8 | 不预建 `Op::Impulse` | §1.3；无真实调用方 |
| 9 | 斜滑是否清零速度 | **待定**，Task 2 目检后裁决（§4.2④） |
| 10 | 溅射两骰（触发 + 抖动）的 RNG key 用起始坐标而非撞停坐标 | §6.1③/§6.3；撞停坐标同 tick 不唯一（脱格后同格可被二次占据），撞车即翻案 4 偏置。2026-08-31 评审修订 |
| 11 | `side()` 循环上界 core 侧 clamp 到 `DISPERSION_MAX` | §3.1；`dispersion` 越界破坏 P4 写域论证（release 数据竞争），与 `blast_cost` 手感旋钮不同类，不能只靠 I/O 校验。2026-08-31 评审修订 |
| 12 | `MovedSide` 撞停也触发溅射（暂定） | §6.1①；瀑布入水的水花正走此路径。横流水花是否过量**待 Task 3 目检**后终裁 |
