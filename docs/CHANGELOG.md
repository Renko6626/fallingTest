# Changelog — fallingTest 文档与开发记录

本文件按"日期 → 条目"倒序记录 docs/ 下的产出与重要发现。
日期使用 UTC+8。所有条目应给出受影响文件路径。

## [Unreleased]

## 2026-06-06

### Added
- `docs/reference/noita-deep-dive.md`：Noita 深度调研报告（4 路并行网络调研 + prototype 现状对照）。覆盖：目标效果全景（材质规模/染色 stains/打击感构成）、核心算法确证（单缓冲循环与我们一致）、超越朴素 CA 的运动学扩展（速度/重力积分、CA↔粒子双轨、dispersion rate、粉末 inertia）、刚体桥接与多线程核验、Phase 1 行动队列（§6）。
- `docs/reference/noita-multiplayer-and-determinism.md`：联机专题调研——Noita 多线程公开细节"挖尽"声明、模拟确定性证据链（世界生成确定、模拟大概率不确定）、四个联机模组架构对比（NT / NoitaMP / Entangled Worlds / Arena，含 NEW 同步协议源码级细节）、同类先例（Factorio lockstep / Teardown 确定性命令流 / Terraria diff / rollback 不可行性）。
- `docs/CHANGELOG.md`、`docs/sessions/`：按 CLAUDE.md §4 / §3.1 首次建账。

### Proposed
- `docs/proposals/2026-06-06-deterministic-parallel-and-netcode.md`：①确定性棋盘格论证——写域互斥 + 读域夹断 + counter-based RNG 三条件 ⇒ 任意线程数位级一致；②确定性工程契约 D1–D10；③联机推荐"地形 lockstep + 实体状态同步"双层架构（NEW 式 chunk RLE 快照做修复/late-join 兜底）；④分阶段 M0–M3。后应用户质询补强 §2.3"顺序账本"——逐项论证 7 个顺序来源如何钉死到数据（核心：同 pass chunk 间因 footprint 不相交而可交换，"顺序不存在"；并显式声明棋盘格语义 ≠ 串行全网格语义，迁移时一次性接受）。待裁决：M0 入 Phase 1 队列、联机目标形态确认。

### Changed
- `docs/algorithms/parallel-update-strategies.md`：按已核验事实精化——十字写域精确表述（含 Petri 原话）、Margolus 标注"非 Noita 方案 + 天然确定性"、补 64/512 双层 chunk 结构与确定性 caveat。
- **四项用户裁决落账**：①fire spec 走 Noita 式（温度场降级实验分支，spec 头部加裁决横幅）；②M0 确定性地基批准、排 Phase 1 队首；③联机目标形态定为 coop + 小规模 PvP（M2 需加对称竞技场景）；④旧反应表火焰调参留档于 commit `b99b2ec`。提案 Status: Proposed → Trial。涉及：`docs/superpowers/specs/2026-05-26-fire-system-design.md`、`docs/proposals/2026-06-06-deterministic-parallel-and-netcode.md` §7。
- **采纳外部评审（用户提供的 GPT 审阅，6/6 成立）**：①RNG key 升级为完整 7 元组 `(seed, tick, pass_id, x, y, salt, attempt)`——修复"确定但强相关"隐患（同帧同格多次掷骰返回同值、子像素概率取整被偏置）；②staged plan 插入 **M0.5**（Python 单线程 4-pass 语义原型，避免 Phase 2 同时换语言+调度+并行）；③D1 补整数化细则（density 整数等级、概率 u32 阈值 + 2 的幂量化加载）；④实体连续占位升级为 §4.3 一等规则（量化实体快照 = 地形 tick 输入，量化边界 = 确定性边界）。涉及 `docs/proposals/2026-06-06-deterministic-parallel-and-netcode.md`。
- `docs/superpowers/specs/2026-05-26-fire-system-design.md`：**全文重写为 v2**——主线 Noita 式（fire_hp / 静态温度比较 / requires_oxygen / counter RNG 完整 key），新增**延迟点燃队列**设计（防帧内沿扫描方向的连锁偏置）；蔓延行为显式由数值编码（wood 仅经火苗+氧气表面蔓延、oil 相邻直燃含水下、水蒸发复用燃烧机制）；v1 温度场整章降级为附录 A（实验分支，3 项开启前置条件）。消除"裁决横幅 vs 正文温度场"的自相矛盾。

### Fixed
- `docs/reference/noita-deep-dive.md`：应用户质询，对 5 组承重结论做一手来源逐字抽查（80.lv / macuyiko / jason.today / FSS issues #3 #4 / materials.xml dump 直查），4 组全部逐字命中；删除 1 条伪引语（"temperature is not part of this simulation" 不存在于其声称出处），"Noita 无温度场"结论改由数据文件结构证据支撑（报告 §2.3 + §7 抽查记录）。

### Investigating
- **重大发现：Noita 没有温度场/热传导**（开发者直述，80.lv）。火 = 材质静态常量比较（`temperature_of_fire` vs `autoignition_temperature`）+ 随机方向概率点燃 + `fire_hp` 消耗；连 lava 点火/固化都走反应表。`docs/superpowers/specs/2026-05-26-fire-system-design.md` 的"每像素温度场 + 传导 pass"属自创设计，与 chunk 休眠优化正面冲突——待裁决，建议先实现 Noita 式、传导降级为实验分支（报告 §5.3）。
- Noita 的 `cell_type` 只有 solid/liquid/fire/gas 四种，粉末 = liquid + `liquid_sand="1"`；玩家/敌人不是 Box2D 刚体而是逐像素碰撞的 kinematic entity——均与我们原先的直觉假设不同（报告 §2.2、§4.3）。
