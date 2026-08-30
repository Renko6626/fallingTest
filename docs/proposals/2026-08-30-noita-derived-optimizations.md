# Noita 对照得出的四项优化（活矩形 / 场降本 / 粉末惯性 / 哈希口径）

> 文档路径：`docs/proposals/2026-08-30-noita-derived-optimizations.md`
> 运行时版本：Rust（sand-core M0+）
> 最近更新：2026-08-30 (UTC+8)
> **Status**: O1 Implemented（2026-08-30，spec `docs/superpowers/specs/2026-08-30-o1-live-rect-design.md`，稀疏 2.7×/worst 1.2×/睡眠持平）；O2–O4 Proposed（各项时点见表）
> 依据：`docs/perf/2026-08-30-m0-rust-informal.md`（M0 实测）、`docs/reference/noita-deep-dive.md`
> （Noita 调研）、80.lv / GDC / macuyiko 复查（2026-08-30 会话）

## 总判断

M0 与 Noita 的调度架构同构，性能差距不是方向性的，而是：①少一层 Noita 有的
sub-chunk 细粒度跳过（O1 可回收）；②多背一个 Noita 没有的全图场 pass
（Layer F，总纲翻案 2 的代价——O2 可降本）。O3/O4 为顺手项。

| # | 项 | 时点 | 预期收益 |
|---|---|---|---|
| O1 | chunk 内活矩形扫描 | **M1 门口**（粒子层动工前） | 活跃 chunk 内静区扫描归零；稀疏活跃场景数倍 |
| O2 | Layer F 低分辨率场格 + 半频更新 | **M2 设计期**（场层 spec 里裁决） | 场 pass 成本 ÷16（4×4 格）再 ÷2（半频） |
| O3 | 粉末 `is_free_falling` + `inertial_resistance` | M1 可选（顺手） | 静止堆免斜下判定 + 湿沙陡壁手感 |
| O4 | 运行时周期哈希口径 | M5（session 层） | 对局内不付每 tick 全图哈希 |

## O1 chunk 内活矩形（sub-chunk dirty rect 的确定性版）

**动机**：Noita 官方核心跳过手段是 chunk 内脏矩形；M0 因 tick 583 分叉
（spec §1.4 修订）退到 chunk 级全扫——活跃 chunk 哪怕 10 个 cell 在动也扫 4096 格。

**方案**：每 chunk 扫描仍单线程单遍（自下而上、行内交替），起始矩形 = 上 tick 标记的
本地矩形；扫描过程中发生的写入把矩形**向扫描前方扩张**（已扫过的方向不回访）。

**确定性等价论证**（须入 spec 并由 SyncTest「活矩形 vs 全扫」配置执法）：
全扫语义本身是单遍的——cell 被评估一次，评估后邻域再变化也不回访。因此全扫中
"实际会动"的 cell 集合 = 扫描到达时可动的 cell。链式移动（tick 583 型）只沿扫描方向
传播，扩张恰好覆盖；逆扫描方向的连锁在全扫里同样要等下一 tick。故活矩形扫描与
全扫**逐位等价**。与被否决的"tick 起点冻结矩形"的本质区别：冻结矩形不随本遍写入生长。

**验收**：SyncTest 新增第 5 配置（活矩形开）与既有四配置比对零分叉；
worst/稀疏两类 bench 前后对照落 `docs/perf/`。

## O2 Layer F 降本（低分辨率场格 + 半频）

**动机**：Layer F pull 双缓冲是每 tick O(全图)（总纲 §4），会吃掉"睡眠让静止世界免费"
的红利（perf 文档前视 caveat）；Noita 的对照做法是压根不设全图场。

**方案**（M2 场层 spec 里正式裁决，此处仅立约束）：
1. 场分辨率与 cell 解耦——场格 N×N cell（首选 4×4，成本 ÷16），网格↔场的采样/源项
   注入走固定整数映射；
2. 半频更新——场 pass 只在偶数 tick 执行（tick 奇偶确定，不破 P1；G↔F 本就是
   一 tick 延迟的单向边，延迟变为 1–2 tick，语义仍确定）；
3. 两项都必须是**数据/常量可调**而非硬编码，且写入 tick 管线协议（architecture §4）。

**判定条件**：M2 场层原型 bench——若全分辨率全频已在预算内，可只留缝不启用。

## O3 粉末惯性（FSS 社区方案，材质字段驱动）

静止粉末堆不参与斜下滑动判定（`is_free_falling` 位，Cell aux 有空位）；被扰动时按材质
`inertial_resistance ∈ [0,255]`（整数化）经 counter RNG 判定是否唤醒左右邻居。
省 CPU 之外还带玩法收益（沙 ≈易塌 / 湿沙 ≈立陡壁）。反应表禁令不涉——这是运动规则
字段，参数进 materials.ron。需要新 RNG stream（注册表追加，禁复用）。

## O4 哈希口径分层

- **执法口径**（SyncTest / golden / CI）：每 tick 全图哈希——维持现状。
- **运行口径**（对局内 lockstep，M5 落地）：周期哈希（如每 32–64 tick）+ chunk 哈希树
  按需下钻定位。720p 全图哈希实测量级 <1ms，周期化后运行时成本可忽略。

## 明确不采纳（记录以免重开）

- **Reality bubble / 相机附近模拟**：模拟范围依赖本地相机违反 P1，与 lockstep 不相容。
  小图全模拟是架构费，实测（睡眠 0.066ms）便宜，不回收。
- **非确定性随机跳过**（如"每帧只更新随机 1/N 液体"之类墙钟/顺序流技巧）：违反 P1/D2。
