# 会话总账：2026-08-30 · M0 骨架与执法实施

> 文档路径：`docs/sessions/2026-08-30-m0-implementation.md`
> 最近更新：2026-08-30 (UTC+8)
> 上一篇：`2026-08-29-rust-pivot.md`（同一连续会话的前半：大转向 + 文档整合 + M0 spec）

## 做了什么

用户裁决跳过 writing-plans，按 spec（`docs/superpowers/specs/2026-08-29-m0-skeleton-design.md`）
一口气实施 M0。产出：

- **sand-core**（commit `44a70cb`）：cell / chunk / world / rng / window / rules / scheduler / hash / Sim。
  RNG 金值与 `archive/prototype-python/core/rng.py` 实跑值交叉锚定。
- **sand-harness**（commit `d3d140e`）：scenario（RON+指纹）/ runner / render；synctest、replay
  （--golden / --write-golden）、hashrun、render 四子命令；golden ×2 入库；`data/` 材料表 + 三场景。
- 测试 22 项全绿（单测 14 + 行为 5 + CI SyncTest 1 + golden 2）。

## 关键事件：SyncTest 首战开张

初版按 spec 的"cell 级冻结脏矩形"实现，CI SyncTest 在 tick 583 抓到
「跳过开 vs 关」分叉。定位：一段静止水行右端让位后，全扫模式下右到左扫描令整段
在**单 tick 内链式平移**（每 cell 看到刚腾空的格子），冻结矩形只覆盖右端 ±1，链被切断。
根因 = 单缓冲"tick 内链式移动"语义与任何 tick 起点冻结的 cell 级扫描域不相容（链长无上界）。
修复：**休眠粒度提升为 chunk 级 + 相位边界唤醒**（dirty ∪ next_dirty 重查；屏障后原子
合并结果与调度无关），活跃 chunk 全量扫描。等价论证与修订记录在 spec §1.4。
Python 当年没踩坑是因为 M0.5 决策①直接删了 static 跳过。

另：spec 自审阶段已把总纲的 1-bit 奇偶戳升级为 8 位世代戳（陈旧位撞车，1/256 稀释）。

## 验收状态（spec §0）

| # | 项 | 状态 |
|---|---|---|
| 1 | cargo test 全绿 | ✅ 22 项 |
| 2 | synctest 10 万 tick 零分叉 | ✅ release 版 acceptance 场景（640×384 四配置，实跑 2993s，scenario_fp `f5a093e75d6e67c1`） |
| 3 | 双机 hashrun 逐字一致 | ⬜ **待用户执行**：两台机器各跑 `./target/release/sand-harness hashrun data/scenarios/acceptance.ron --ticks 100000 > hashes.txt` 后 diff |
| 4 | render GIF 目检 | ✅ `out/mixed.gif`：安息角、沙沉水、液面摊平、三路浇注 |

## 留给后续

- M1 粒子层（脱格/落格、DDA、容量限流）——动手前过 brainstorming 出 spec。
- 正式 bench 场景 + `docs/perf/` Rust 基线（M1 前后）。
- 简版横流的水面有颗粒感缝隙（1 格横移语义所致），dispersion 留 M2 前评估。
- `sand-session`（GGRS）/ `sand-bridge`（gdext）在各自里程碑建 crate。
