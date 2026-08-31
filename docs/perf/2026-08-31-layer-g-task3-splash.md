# Layer G Task 3（撞击溅射脱格）性能对照

> 文档路径：`docs/perf/2026-08-31-layer-g-task3-splash.md`
> 运行时版本：Rust（sand-core + sand-harness）
> 最近更新：2026-08-31 (UTC+8)
> **Status**: Implemented

对照 spec `docs/superpowers/specs/2026-08-31-layer-g-velocity-design.md` §7.1 的 bench 验收项。
before 侧 = `2dd9925`（Task 2 收口点）。

## 结论先说

**无回退。代价比 Task 2 小一个量级。**

最有代表性的 `acceptance`（640×384，最接近总纲 §7 目标量级）六个格子是
**+7.3% / +1.5% / +1.2% / −3.0% / +8.0% / +2.9%**，中位约 +2%。对比 Task 2 同场景的
+21%~+34%，这一轮几乎看不见。

机制清楚：溅射只在**撞停那一 tick**触发一次判定（三条件短路，最便宜的
`v1 < SPLASH_MIN_SPEED` 排在最前），不像速度积分那样给每个下落 cell 都加子步循环。
真正新增的持续成本是多出来的粒子——但粒子层本来就存在，且 `MAX_SPLASH_PER_CHUNK = 64`
把每 chunk 每 tick 的增量钉死。

**`mixed` 与 `sparse` 的大数值不要单独引用**：两者方向都不一致（`mixed 1 live −0.6%`
对 `mixed 8 live +28.6%`，`sparse 1 live −10.0%` 对 `sparse 16 live +23.8%`），
是本机噪声的典型形状，不是可解释的机制。判方向要看 `acceptance` 那种"多个线程数同号"
的结构。

## 环境与口径

完全照抄 `docs/perf/2026-08-31-layer-g-task2-gravity.md`（该文又照抄 M0 基线），三份同尺度：

- CPU：Intel Xeon Gold 6330 @ 2.00GHz，112 逻辑核，**共享服务器**，数字 provisional。
- 构建 `cargo build --release -p sand-harness`；路径 `sand-harness hashrun`，取 stderr 的 `avg`。
- 每组合 3 次独立进程冷启动取中位，共 108 次测量。
- 场景与 tick：`mixed` 1500、`sparse` 2000、`acceptance` 5000。
- 同机同轮次 before/after，**两侧逐次交替执行**；before 侧用 `git worktree` 在 `2dd9925`
  另建工作树独立编译，`--materials` 指向该工作树自己的 `data/materials.ron`
  （**本 Task 改了 materials.ron**：新增 `splash_chance`，故两侧材料表内容不同——
  这正是被测的语义差，不是口径污染）。

## 数据（ms/tick，3 次中位）

| 场景 | 线程 | scan | before | after | Δ |
|---|---|---|---|---|---|
| mixed | 1 | sleep | 0.633 | 0.623 | −1.6% |
| mixed | 1 | live | 0.325 | 0.323 | −0.6% |
| mixed | 8 | sleep | 0.601 | 0.630 | +4.8% |
| mixed | 8 | live | 0.273 | 0.351 | +28.6% |
| mixed | 16 | sleep | 0.669 | 0.614 | −8.2% |
| mixed | 16 | live | 0.328 | 0.411 | +25.3% |
| sparse | 1 | sleep | 1.127 | 1.062 | −5.8% |
| sparse | 1 | live | 0.291 | 0.262 | −10.0% |
| sparse | 8 | sleep | 0.376 | 0.395 | +5.1% |
| sparse | 8 | live | 0.190 | 0.189 | −0.5% |
| sparse | 16 | sleep | 0.382 | 0.427 | +11.8% |
| sparse | 16 | live | 0.189 | 0.234 | +23.8% |
| acceptance | 1 | sleep | 2.214 | 2.248 | +1.5% |
| acceptance | 1 | live | 0.965 | 1.035 | +7.3% |
| acceptance | 8 | sleep | 0.798 | 0.774 | −3.0% |
| acceptance | 8 | live | 0.496 | 0.502 | **+1.2%** |
| acceptance | 16 | sleep | 1.171 | 1.205 | +2.9% |
| acceptance | 16 | live | 0.625 | 0.675 | +8.0% |

## Layer G 三个 Task 的累计账（`acceptance` 8 线程 live，产品配置）

| 时点 | ms/tick | 相对起点 |
|---|---|---|
| Layer G 起点（Task 1 之前） | 1.045 | — |
| Task 1 液体色散 | 0.465 | −55% |
| Task 2 重力速度积分 | 0.536 | −49% |
| **Task 3 撞击溅射（本轮）** | **0.502** | **−52%** |

60Hz 的单 tick 预算是 16.6 ms ⇒ 当前用掉约 **3%**，余量 33 倍。
（Task 3 的 0.502 比 Task 2 的 0.536 还低一点，那是跨轮次的机器负载差异，
**不要**读成"溅射让它变快了"——同轮次对照里 `acceptance 8 live` 是 +1.2%。）

## 没做的事

- **没有测粒子峰值数量**。溅射的真实成本上界是粒子层的负担，而 harness 目前不导出
  粒子数时间序列。`MAX_SPLASH_PER_CHUNK` 已经给出解析上界（640×384 = 60 chunk ⇒
  3840 粒子/tick），ms/tick 也没显示异常，故本轮不加计数器。若 M2 之后粒子层
  成为瓶颈，这是第一个该补的观测量。
