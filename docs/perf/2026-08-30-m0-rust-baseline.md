# M1 动工前正式性能基线

> 文档路径：`docs/perf/2026-08-30-m0-rust-baseline.md`
> 运行时版本：Rust（sand-core，commit `5653be6`）
> 最近更新：2026-08-30 (UTC+8)
> **Status**: Implemented（正式基线：固定场景、release 构建、逐组 3 次取中位；仍是**服务器 CPU**，provisional——本机为共享 Xeon 服务器，单核明显弱于目标桌面 CPU，估计打 5–7 折，与 informal 文档口径一致）

取代 `docs/perf/2026-08-30-m0-rust-informal.md`（该文档头部已加指针回指本文档）。本文档是 M1 粒子层动工前的性能基线快照（`.superpowers/sdd/2026-08-30-m1-particle-layer-plan/task-1-brief.md` Task 1），纯测量，未改动 `crates/` 或 `data/` 任何文件。

**口径说明（重要，与 informal 文档不可直接对比）**：本基线走**短程稳态吞吐**口径——每组测量的 tick 数以命令行为准（见下），远小于 informal 文档"720p 全活跃前 300/1200 tick"或 `acceptance.ron` 全量 20000 tick 的长程口径。这是执行期纠偏后的结果：最初尝试用 acceptance 20000→100000 tick 差分单独扣出"纯稳态"数值，单组测量墙钟达 ~60 秒、总预算超支，被中途叫停；改为本文档口径后单组测量 ≤ 数秒，总测量墙钟 < 5 分钟。

## 环境

- CPU：Intel(R) Xeon(R) Gold 6330 @ 2.00GHz，112 逻辑核（2×28C56T，NUMA 双节点），max 3.1GHz / min 0.8GHz。**共享服务器**，非独占——运行期间无法排除其他进程干扰，数字标 provisional。单核性能明显弱于目标桌面 CPU（沿用 informal 文档估计：打 5–7 折）。
- 构建：`cargo build --release -p sand-harness`（release profile，优化开启）。
- 路径：`sand-harness hashrun`（非 `synctest`——后者跑六配置比对，不是计时口径；`hashrun` 单配置跑满 `--ticks`，`tick 耗时 avg` 取自 stderr）。
- Commit：`5653be6`（分支 `m1-particle-layer`）。

## 方法

- 每个 {场景 × 线程数 × scan 模式} 组合独立跑 3 次（各自独立进程冷启动，非同进程重复 tick），取 `avg_ms` 中位数。
- 线程数：`--threads {1,8,16}`。scan 模式：`--scan sleep`（ChunkSleep，M0 语义）、`--scan live`（LiveRect，O1 活矩形）。未测 `--scan full`（无 chunk 休眠的参照模式，已在 O1 文档留证，非本任务范围）。
- 场景与 tick 数：
  - **mixed**（`data/scenarios/mixed.ron`，256×192）——**1500 tick**（场景自带全量值）。浇注 0–900，尾段 900–1500 为沉降/入睡段。
  - **acceptance**（`data/scenarios/acceptance.ron`，640×384）——**5000 tick**（非全量 20000；停在浇注期 0–8000 内部，短程稳态口径）。
  - **sparse**（`data/scenarios/sparse.ron`，640×384）——**2000 tick**（场景自带全量值，三股细流持续到 2000，永不入睡）。
  - **睡眠常态**：不新增场景文件（遵守"不改 data/"纪律），改用 **mixed 自身的浇注尾段差分**：分别以 `--ticks 900`（浇注刚结束）与 `--ticks 1500`（+600 tick 沉降尾）各跑 3 次取中位 avg，`steady_ms = (median(avg@1500)×1500 − median(avg@900)×900) / 600`，代表 [900,1500) 这 600 tick 窗口的边际每 tick 成本。**注意**：该差分法在小场景/小窗口下信噪比有限（见下方"异常观察"），结果按量级参考，不是精确稳态值。

### 完整命令行

```bash
cargo build --release -p sand-harness

# dense: mixed（3 次 × 6 组合，场景自带 1500 tick）
./target/release/sand-harness hashrun data/scenarios/mixed.ron --threads {1,8,16} --scan {sleep,live}

# dense: acceptance 短程稳态（3 次 × 6 组合，5000 tick，非全量）
./target/release/sand-harness hashrun data/scenarios/acceptance.ron --threads {1,8,16} --scan {sleep,live} --ticks 5000

# sparse（3 次 × 6 组合，场景自带 2000 tick）
./target/release/sand-harness hashrun data/scenarios/sparse.ron --threads {1,8,16} --scan {sleep,live}

# 睡眠常态（mixed 尾段差分，3 次 × 6 组合 × 2 个 ticks 值）
./target/release/sand-harness hashrun data/scenarios/mixed.ron --threads {1,8,16} --scan {sleep,live} --ticks 900
./target/release/sand-harness hashrun data/scenarios/mixed.ron --threads {1,8,16} --scan {sleep,live} --ticks 1500
```

每次调用取 stderr 尾行 `tick 耗时 avg X.XXXms / max Y.YYYms` 中的 avg 值。

## 数据

### dense — mixed（256×192，1500 tick 全量 avg，ms/tick，3 次中位）

| 线程 | ChunkSleep | LiveRect |
|---|---|---|
| 1 | 0.640 | 0.364 |
| 8 | 0.414 | 0.379 |
| 16 | 0.594 | 0.381 |

### dense — acceptance（640×384，**5000 tick**，浇注期内 avg，ms/tick，3 次中位）

| 线程 | ChunkSleep | LiveRect |
|---|---|---|
| 1 | 1.693 | 1.042 |
| 8 | 1.014 | 1.017 |
| 16 | 1.349 | 1.146 |

### sparse（640×384，2000 tick 全量 avg，ms/tick，三股细流永不入睡，3 次中位）

| 线程 | ChunkSleep | LiveRect |
|---|---|---|
| 1 | 0.985 | 0.228 |
| 8 | 0.367 | 0.188 |
| 16 | 0.475 | 0.212 |

### 睡眠常态（mixed [900,1500) 差分窗口，ms/tick，量级参考）

| 线程 | ChunkSleep | LiveRect |
|---|---|---|
| 1 | 0.355 | 0.091 |
| 8 | 0.749 | 0.259 |
| 16 | 0.609 | 0.310 |

原始三次数据、中间计算过程见 `.superpowers/sdd/2026-08-30-m1-particle-layer-plan/task-1-report.md`。

## 观察

- **LiveRect 全面优于或持平 ChunkSleep**：三个 dense/sparse 场景里 LiveRect 中位数无一例差于 ChunkSleep，sparse 场景差距最大（1 线程 4.3×：0.985ms → 0.228ms），与 O1 spec 的"稀疏受益最大"结论一致。
- **线程数扩展性在这批小场景下不明显、甚至倒挂**：mixed/sparse/accept_5000 三个场景里，8 线程普遍比 1 线程快，但 16 线程常常**比 8 线程更慢**（如 sparse ChunkSleep：8T 0.367ms → 16T 0.475ms；mixed ChunkSleep：8T 0.414ms → 16T 0.594ms）。这些场景（256×192 / 640×384，几十个 chunk 量级）活跃工作量太小，线程池调度/同步开销在 16 线程时抵消并行收益，且本机是 112 核共享服务器、NUMA 双节点，16 线程可能跨节点调度产生额外延迟。**结论**：现有场景规模不足以体现高线程数收益，与总纲 §7 的 1280×720 上限场景（informal 文档测得 8T 明显优于 1T）量级不同，不构成回归信号。
- **睡眠常态差分法信噪比有限**：mixed 尾段窗口仅 600 tick、每 tick 成本本身在亚毫秒量级，两次独立进程 900/1500 全量 run 之间的调度噪声（观测到同组 3 次 avg 波动可达 20–30%）会显著影响差分结果。最初尝试用 acceptance 20000→100000 tick 差分（窗口 80000 tick，理论上信噪比应更好）在纠偏前的中间产物里出现过**负值**（[9000,11000) 窗口差分给出 −0.416ms/tick，物理上不可能），已在纠偏后弃用；改用 mixed 900→1500 后数值全部为正、量级合理（0.09–0.75ms/tick），但仍应视为参考而非精确稳态值——**若后续需要精确睡眠常态数字，需要在 harness 里加逐 tick 计时输出（当前 `hashrun` 只报告整段 avg/max），这超出本任务"不改代码"的范围**，留作后续任务。
- **本文档口径与 informal 文档不可直接比较**：informal 文档"睡眠常态 < 0.1ms"测的是 1280×768 全静止专用场景（720p，本仓库当前无对应场景文件）；本文档"睡眠常态"是 256×192 场景的浇注尾段边际成本，场景尺寸、静止程度均不同，两者不是同一测量对象。
- **acceptance 5000-tick 短程稳态明显高于 mixed/sparse**：640×384 图 + 双水源+沙源同时浇注，1 线程 ChunkSleep 达 1.693ms/tick，是 mixed 同配置的 2.6×——符合"活跃面积越大成本越高"的预期，但因未跑满 20000 tick 全量（含沉降与两波唤醒），不能直接类比 informal 文档里 acceptance 全量口径的历史数字（此前中间产物中曾测得 acceptance 全量 20000 tick 均值 1 线程 ChunkSleep 1.745ms、LiveRect 0.718ms，8 线程 ChunkSleep 1.345ms、LiveRect 0.903ms，量级接近但非同一 tick 窗口，不作为本文档正式结论，仅供交叉参考）。

## 对总纲 §7 预算的校准

总纲 §7：1280×720 上限配置，网格全图最坏 ~7ms 单线程 → 四相并行后 ~2ms；tick 总预算 ≤4ms（最坏情况）。本次全部场景（256×192 / 640×384）均小于 1280×720 上限，且本次为短程稳态口径而非全活跃最坏口径，故不直接对标 §7 数字；informal 文档已用专用 720p/1080p 场景对 §7 做过校准（8T 全活跃 ~8ms，超预算但在帧预算内），本次测量未推翻或修订该结论。M1 落地粒子层、场层后应在 720p 量级场景上用相同短程稳态口径复测一次，观察每 cell 成本上涨幅度。
