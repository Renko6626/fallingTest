> 文档路径：`docs/perf/baseline.md`
> 运行时版本：Python 3.x（CPython，本机）
> 最近更新：2026-06-06 (UTC+8)

# 性能基线

按 CLAUDE.md §5.3 约定记录。格式：`{width}x{height}, {active_ratio}% active, {fps} FPS`。
**正式基准 = `prototype/benchmark.py`**（128×128、底墙+大沙块+水层 ≈34% 非空、200 帧、seed 42）——此后所有对比一律用它。

## 2026-06-07 — M0 前后对比（正式基准脚本，同机同场景）

| 版本 | 实测 | 备注 |
|---|---|---|
| M0 前（commit 061ede8） | `128x128, 34% active, 27.6 FPS`（36.2 ms/帧） | 全局 random + float 密度 |
| **M0 后（counter RNG + crc32 hash + 整数化）** | `128x128, 34% active, 23.0 FPS`（43.5 ms/帧） | **回退 -17%（+7.3 ms/帧）**，在 20% 预算内 ✓（评审 m1 预测的量级一致；纯 Python 下 keyed 哈希贵于 C 实现的 random，C# 期反转） |

注：下方 06-06 的 42 FPS provisional 来自评审的另一套即兴场景，活跃构成更轻，**不再作为对比基线**，仅留档。

## 2026-06-06 — M0 前基线（评审实测，provisional）

> 来源：opus 独立评审 subagent 在本机的只读验算（确定性提案评审 M6/m1）。**M0 开工时用正式 benchmark 脚本复测并替换本节数字**。

| 项 | 实测 | 备注 |
|---|---|---|
| `CellGrid.update()` 整帧 | **128x128, 30% active, ~42 FPS**（≈23.7 ms/帧） | 现行三次全网格遍历（`prototype/core/grid.py:57-93`） |
| 空网格底价 | 128x128, 0% active, ≈7.2 ms/帧 | 三次全网格遍历的固定开销 |
| counter RNG（纯 Python SquirrelNoise5 式） | ≈1828 ns/次 | vs `random.random()` ≈100 ns、`random.shuffle`(2 元素) ≈823 ns——**慢约一个数量级，Phase 1 接受**；C# 内联后预期反超 |

### 预测（待 M0/火系统实测验证）

- M0（counter RNG 替换）后：128×128 30% active 预计 **28–33 FPS**
- 火系统（新增全网格 burn pass）后：预计 **~25 FPS**

### 约定

- 每完成一个里程碑（M0 / M0.5 / dispersion / velocity / fire / 粒子 / dirty rect）在本文件追加一节对比基线；回退超 20% 须在 CHANGELOG 说明原因。
- 确定性回归测试规模控制：64×64×1000 帧 或 128×128×200 帧（单测 <30s，评审 m8）。
