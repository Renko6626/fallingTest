# Layer G Task 2（重力速度积分）性能对照

> 文档路径：`docs/perf/2026-08-31-layer-g-task2-gravity.md`
> 运行时版本：Rust（sand-core + sand-harness）
> 最近更新：2026-08-31 (UTC+8)
> **Status**: Implemented

对照 spec `docs/superpowers/specs/2026-08-31-layer-g-velocity-design.md` §7.1 的 bench 验收项。

## 结论先说

**一致变慢 5%–34%，方向统一、机制清楚，属预期内的语义成本，不是回退性缺陷。**

18 个 {场景 × 线程 × scan} 格子里 15 个变慢、3 个小幅变快（−1.3%、−3.6%、−7.2%，都在本机噪声带内）。
最有代表性的 `acceptance`（640×384，最接近总纲 §7 的目标量级）在 LiveRect 下
**+21.2% / +27.3% / +34.0%**（1/8/16 线程），三个线程数方向一致 ⇒ 不是噪声。

机制不需要猜：下落中的 cell 每 tick 现在最多走 4 个子步，每个子步都是一次完整的
`displace` 探测 + 写入；`acceptance` 是双水源+沙源的持续浇注场景，下落体积占比最高，
所以吃满了涨幅。静止堆体不受影响（`v = 0` ⇒ `n = 1` ⇒ 与 Task 2 之前同一条路径），
这就是 `sparse` / `mixed` 涨幅明显更小的原因。

**绝对量级仍远在预算内**：最慢的一格是 `acceptance 16 线程 sleep` 的 1.238 ms/tick，
60Hz 的单 tick 预算是 16.6 ms。产品实际使用的 `acceptance 8 线程 live` 是 **0.536 ms/tick**。

**与 Task 1 的关系**：Task 1（色散）在同一场景同一配置上把 1.045 ms 降到 0.465 ms，
Task 2 把它推回 0.536 ms —— 两个 Task 合计相对 Layer G 起点仍是**净变快约一半**。
Task 1 的性能文档当时已写明"下一个 Task 预期是净增成本，别把本文的数字当成可继承的余量"，
本次实测兑现了那句话。

## 环境与口径

口径完全照抄 `docs/perf/2026-08-31-layer-g-task1-dispersion.md`（该文又照抄
`docs/perf/2026-08-30-m0-rust-baseline.md`），以便三份文档同尺度阅读：

- CPU：Intel Xeon Gold 6330 @ 2.00GHz，112 逻辑核（2×28C56T，NUMA 双节点），**共享服务器**，数字 provisional。
- 构建：`cargo build --release -p sand-harness`。
- 路径：`sand-harness hashrun`，取 stderr 尾行 `tick 耗时 avg`。
- 每个 {场景 × 线程 × scan} 组合 **3 次独立进程冷启动取中位**，共 108 次测量。
- 场景与 tick：`mixed` 1500、`sparse` 2000、`acceptance` 5000（沿用 M0 基线的 tick 数）。
- **同机同轮次 before/after 对照，两侧逐次交替执行**：before 侧用 `git worktree` 在
  `b17fdc9`（Task 2 之前）另建工作树独立编译，并显式 `--materials` 指向该工作树的
  `data/materials.ron`。本 Task 未改任何数据文件，两侧材料表逐字相同。

脚本与原始输出：本次会话 scratchpad（`bench.sh` + `bench.tsv`，108 行）。

## 数据（ms/tick，3 次中位）

| 场景 | 线程 | scan | before | after | Δ |
|---|---|---|---|---|---|
| mixed | 1 | sleep | 0.561 | 0.610 | +8.7% |
| mixed | 1 | live | 0.279 | 0.312 | +11.8% |
| mixed | 8 | sleep | 0.601 | 0.663 | +10.3% |
| mixed | 8 | live | 0.301 | 0.312 | +3.7% |
| mixed | 16 | sleep | 0.551 | 0.609 | +10.5% |
| mixed | 16 | live | 0.362 | 0.336 | −7.2% |
| sparse | 1 | sleep | 1.021 | 1.119 | +9.6% |
| sparse | 1 | live | 0.227 | 0.264 | +16.3% |
| sparse | 8 | sleep | 0.380 | 0.375 | −1.3% |
| sparse | 8 | live | 0.145 | 0.164 | +13.1% |
| sparse | 16 | sleep | 0.467 | 0.450 | −3.6% |
| sparse | 16 | live | 0.172 | 0.196 | +14.0% |
| acceptance | 1 | sleep | 1.883 | 1.995 | +5.9% |
| acceptance | 1 | live | 0.789 | 0.956 | **+21.2%** |
| acceptance | 8 | sleep | 0.657 | 0.757 | +15.2% |
| acceptance | 8 | live | 0.421 | 0.536 | **+27.3%** |
| acceptance | 16 | sleep | 1.046 | 1.238 | +18.4% |
| acceptance | 16 | live | 0.477 | 0.639 | **+34.0%** |

## 怎么读这张表

- **别把 3 个负值当成"部分场景变快了"。** 它们分散在三个不同配置、幅度都 ≤ 7.2%，
  而本机同格自身抖动实测可达 45%（见 `docs/proposals/2026-08-31-powder-scan-direction-bias.md`
  §6 记录的 `sparse 1T live` 教训：初测 +42.2%，重测 7 次后为 +3.9%）。**单点数据不能
  证明方向**——能证明方向的是 `acceptance` 三个线程数一致为正这种结构。
- **16 线程列信噪比最差**（M0 基线文档已记录本机 16 线程常态倒挂：活跃工作量太小、
  112 核 NUMA 共享机跨节点调度）。`acceptance 16 live +34.0%` 是本表最大值，但它同时
  是噪声最大的格子，别单独引用它。
- **`sleep` 列涨幅普遍小于 `live` 列**，符合机制预期：ChunkSleep 是活跃 chunk 全量扫描，
  分母里本来就有大量"扫到但 `v=0` 不动"的 cell，子步循环加的那点成本被摊薄了；
  LiveRect 的分母更接近纯活跃 cell，涨幅更贴近真实边际成本。

## 没做的事

- **没有做子步循环的性能优化**。当前实现每个子步都重走一遍 `displace` 的完整探测，
  连续下落时"下一格是空气"这个信息没有跨子步复用。若日后 `acceptance` 级场景的
  绝对耗时逼近预算，这是第一个该动的地方——但现在离预算还有 30 倍余量，
  过早优化会给 Task 3 的溅射逻辑增加不必要的耦合。
- **没有测活跃 cell 更新次数**（spec §7.1 提到的口径）。harness 目前没有该计数器，
  加它属于工具改动；ms/tick 已经足够支撑"涨幅可控"这个结论，故本轮不加。
