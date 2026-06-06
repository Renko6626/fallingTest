# Changelog — fallingTest 文档与开发记录

本文件按"日期 → 条目"倒序记录 docs/ 下的产出与重要发现。
日期使用 UTC+8。所有条目应给出受影响文件路径。

## [Unreleased]

## 2026-06-07

### Added
- **M0 确定性地基完成**（提案 §5 M0 行，验收四件套全过——fresh evidence：56 passed / 污染测试过 / replay CLI 两遍逐字一致 / benchmark 入档）：
  - `prototype/core/rng.py`：SquirrelNoise5 counter RNG，完整 7 元 key（素数折叠 + 每帧预计算 frame_seed），金值锚定 + "sim 模块禁 import random" 防回归断言。
  - `prototype/core/ops.py` + `prototype/replay.py`：apply_brush 共用写入路径；JSONL demo 录制/headless 回放（header 嵌 materials.toml sha256，不匹配拒绝）；`main.py --seed/--record`。
  - `prototype/benchmark.py` + `docs/perf/baseline.md`：正式基准定版——M0 前 27.6 FPS → M0 后 23.0 FPS（同机同场景，**-17%，在 20% 预算内**；42 FPS provisional 降级为留档）。
  - `prototype/tests/test_rng.py` / `test_determinism.py` / `test_ops.py`：金值、key 独立性、同 seed 等价、**污染测试**（帧间扰动全局 random，hash 不变）、录放等价、错误 TOML 拒绝。
- `docs/superpowers/specs/2026-06-07-m0-determinism-design.md`、`docs/superpowers/plans/2026-06-07-m0-determinism-plan.md`：M0 实现级设计（用户批准）与 9-task TDD 计划（superpowers 流程：brainstorming → writing-plans → executing-plans → verification）。

### Changed
- `prototype/core/{grid,rules,material,reaction}.py`：6 处全局 `random.*` 全部替换为 keyed RNG（D2）；density 全线整数化、reaction probability → u32 threshold（D1）；CellGrid 增加 seed/_fseed/state_hash(crc32)（D5）。`data/materials.toml` 与全部测试 fixture 密度 ×10。
- 既有测试重钉：删除全部 `random.seed()`，seed 扫描循环 collapse 为确定性单断言。

## 2026-06-06

### Added
- `docs/reference/noita-deep-dive.md`：Noita 深度调研报告（4 路并行网络调研 + prototype 现状对照）。覆盖：目标效果全景（材质规模/染色 stains/打击感构成）、核心算法确证（单缓冲循环与我们一致）、超越朴素 CA 的运动学扩展（速度/重力积分、CA↔粒子双轨、dispersion rate、粉末 inertia）、刚体桥接与多线程核验、Phase 1 行动队列（§6）。
- `docs/reference/noita-multiplayer-and-determinism.md`：联机专题调研——Noita 多线程公开细节"挖尽"声明、模拟确定性证据链（世界生成确定、模拟大概率不确定）、四个联机模组架构对比（NT / NoitaMP / Entangled Worlds / Arena，含 NEW 同步协议源码级细节）、同类先例（Factorio lockstep / Teardown 确定性命令流 / Terraria diff / rollback 不可行性）。
- `docs/CHANGELOG.md`、`docs/sessions/`：按 CLAUDE.md §4 / §3.1 首次建账。
- `docs/perf/baseline.md`：M0 前性能基线（opus 评审实测，provisional）——`128x128, 30% active, ~42 FPS`、空网格底价 7.2ms、counter RNG 纯 Python ≈1.8μs/次（慢 `random.random()` 一个数量级，Phase 1 接受）；含里程碑对比与回退预算约定（落实 CLAUDE.md §5.3，评审 M6）。

### Proposed
- `docs/proposals/2026-06-06-deterministic-parallel-and-netcode.md`：①确定性棋盘格论证——写域互斥 + 读域夹断 + counter-based RNG 三条件 ⇒ 任意线程数位级一致；②确定性工程契约 D1–D10；③联机推荐"地形 lockstep + 实体状态同步"双层架构（NEW 式 chunk RLE 快照做修复/late-join 兜底）；④分阶段 M0–M3。后应用户质询补强 §2.3"顺序账本"——逐项论证 7 个顺序来源如何钉死到数据（核心：同 pass chunk 间因 footprint 不相交而可交换，"顺序不存在"；并显式声明棋盘格语义 ≠ 串行全网格语义，迁移时一次性接受）。待裁决：M0 入 Phase 1 队列、联机目标形态确认。

### Changed
- `docs/algorithms/parallel-update-strategies.md`：按已核验事实精化——十字写域精确表述（含 Petri 原话）、Margolus 标注"非 Noita 方案 + 天然确定性"、补 64/512 双层 chunk 结构与确定性 caveat。
- **四项用户裁决落账**：①fire spec 走 Noita 式（温度场降级实验分支，spec 头部加裁决横幅）；②M0 确定性地基批准、排 Phase 1 队首；③联机目标形态定为 coop + 小规模 PvP（M2 需加对称竞技场景）；④旧反应表火焰调参留档于 commit `b99b2ec`。提案 Status: Proposed → Trial。涉及：`docs/superpowers/specs/2026-05-26-fire-system-design.md`、`docs/proposals/2026-06-06-deterministic-parallel-and-netcode.md` §7。
- **采纳外部评审（用户提供的 GPT 审阅，6/6 成立）**：①RNG key 升级为完整 7 元组 `(seed, tick, pass_id, x, y, salt, attempt)`——修复"确定但强相关"隐患（同帧同格多次掷骰返回同值、子像素概率取整被偏置）；②staged plan 插入 **M0.5**（Python 单线程 4-pass 语义原型，避免 Phase 2 同时换语言+调度+并行）；③D1 补整数化细则（density 整数等级、概率 u32 阈值 + 2 的幂量化加载）；④实体连续占位升级为 §4.3 一等规则（量化实体快照 = 地形 tick 输入，量化边界 = 确定性边界）。涉及 `docs/proposals/2026-06-06-deterministic-parallel-and-netcode.md`。
- **采纳 opus 独立评审（第二轮，2 blocker / 6 major / 10 minor / 5 nit 全部成立并落账）**：除 Fixed 节的修复外——M3 M0 验收谓词重写（污染测试 + RNG 金值 + 回放等价，原"同 seed 同 hash"在 CPython 上空洞）；M6/m1/m8 性能账（基线入档、RNG 成本断言修正、回归测试规模上限）；m3 RNG key 取数时点钉死；m4 M0.5 测试网格 ≥192×192；m5 补 `lava+[flammable]→lava+fire` 反应（浸没木头）；m7 demo 录制头嵌 toml 哈希；m9 文档同步欠账（parallel 文档 Phase 1 行、CLAUDE.md §5.1/§5.2 过时表述、deep-dive §6 过时标注）；m10/n4 测试几何与调参注记；n2 Margolus 条件修正；n3 PvP 公平性入 M2/风险。工作量复核：M0 ~3 天、M0.5 ~2.5–3 天。**评审总判断：M0 可以开工**。
- `CLAUDE.md`：§5.1 velocity 行更新（8.8 定点目标）、§5.2 追加"2026-06-06 更新"块（燃烧不走反应表的豁免说明、旧示例反应过时标注、密度整数化、指向确定性契约与性能基线）。
- `docs/superpowers/specs/2026-05-26-fire-system-design.md`：**全文重写为 v2**——主线 Noita 式（fire_hp / 静态温度比较 / requires_oxygen / counter RNG 完整 key），新增**延迟点燃队列**设计（防帧内沿扫描方向的连锁偏置）；蔓延行为显式由数值编码（wood 仅经火苗+氧气表面蔓延、oil 相邻直燃含水下、水蒸发复用燃烧机制）；v1 温度场整章降级为附录 A（实验分支，3 项开启前置条件）。消除"裁决横幅 vs 正文温度场"的自相矛盾。

### Fixed
- **opus 独立评审揪出 2 个设计 blocker，已修复**：①B1 冷油自燃——fire spec 热源定义缺 `fire_hp==0` 门控，未燃烧的油（temperature_of_fire=120 > 自身阈值 100）会无火自爆并蒸干邻水；②B2 十字写域角落死锁——所有权扫描语义下 (63,63)→(64,64) 对角移动永久不可达、每 pass 25% 面积无人可写，**写域改为正方形 `[chunk−32, chunk+96)²`**（穷举验证：同 pass 两两不相交且恰好密铺，交换律保持）。另修复 M1（ignite_queue 陈旧条目湮灭 ash，apply 时复检）、M2（缝隙延迟"≤3 pass 帧内"为假，正确口径=最坏下一帧对应 pass）、M4（实体占位快照必须走 reliable 命令流，通道按"是否进入地形 tick"划分）、M5（burn pass 的 pass_id=4 约定 + M1 分块化预告）。涉及：fire spec、提案、parallel-update-strategies.md。
- `docs/reference/noita-deep-dive.md`：应用户质询，对 5 组承重结论做一手来源逐字抽查（80.lv / macuyiko / jason.today / FSS issues #3 #4 / materials.xml dump 直查），4 组全部逐字命中；删除 1 条伪引语（"temperature is not part of this simulation" 不存在于其声称出处），"Noita 无温度场"结论改由数据文件结构证据支撑（报告 §2.3 + §7 抽查记录）。

### Investigating
- **重大发现：Noita 没有温度场/热传导**（开发者直述，80.lv）。火 = 材质静态常量比较（`temperature_of_fire` vs `autoignition_temperature`）+ 随机方向概率点燃 + `fire_hp` 消耗；连 lava 点火/固化都走反应表。`docs/superpowers/specs/2026-05-26-fire-system-design.md` 的"每像素温度场 + 传导 pass"属自创设计，与 chunk 休眠优化正面冲突——待裁决，建议先实现 Noita 式、传导降级为实验分支（报告 §5.3）。
- Noita 的 `cell_type` 只有 solid/liquid/fire/gas 四种，粉末 = liquid + `liquid_sand="1"`；玩家/敌人不是 Box2D 刚体而是逐像素碰撞的 kinematic entity——均与我们原先的直觉假设不同（报告 §2.2、§4.3）。
