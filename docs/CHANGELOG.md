# Changelog — fallingTest 文档与开发记录

本文件按"日期 → 条目"倒序记录 docs/ 下的产出与重要发现。
日期使用 UTC+8。所有条目应给出受影响文件路径。

## [Unreleased]

## 2026-08-30

### Added
- **M0 骨架与执法实施完成**（spec `2026-08-29-m0-skeleton-design.md` → Implemented）：
  - `crates/sand-core`：Cell u32 位段（`cell.rs`）、chunk/脏矩形原子合并（`chunk.rs`）、chunk 寻址世界 + brush/fill 共用写入路径（`world.rs`）、SquirrelNoise5 RNG **与 Python 版金值交叉锚定**（`rng.rs`）、WriteWindow unsafe 窗口 + debug 写域断言（`window.rs`）、数据驱动沙/水规则含方向承诺不变量（`rules.rs`）、四相 rayon 调度器（`scheduler.rs`）、xxh3 哈希树叶层（`hash.rs`）、Sim 门面（`lib.rs`）。
  - `crates/sand-harness`：RON 场景/材料加载 + xxh3 指纹（P5）、synctest/replay/hashrun/render 四子命令、golden 回归 ×2 入库、GIF 占位渲染器（消费 `Sim::world()` 只读视图 = Channel A 雏形）。
  - `data/materials.ron` + 三场景（sand_pile / mixed / acceptance 640×384）。
  - 测试 22 项全绿：单测（位段/RNG 金值/相几何互斥穷举/哈希/写域 should_panic）+ 行为 5 项（沙落/堆守恒/沉水/跨缝摊平/方向承诺）+ CI SyncTest（256×192×6000 tick×四配置）+ golden ×2。
  - 验收：release 版 640×384 × 10 万 tick × 四配置 SyncTest 零分叉；GIF 目检通过（安息角/沙沉水/液面摊平）。双机 hashrun 待用户执行。
  - 性能参考（release，640×384 活跃期）：见 synctest 日志；正式 bench 与 `docs/perf/` Rust 基线留 M1 后。

### Added
- `docs/perf/2026-08-30-m0-rust-informal.md`：M0 非正式性能测量（Xeon 6330 服务器 CPU，provisional）。要点：640×384 全活跃最坏 ~3ms@8T、1280×768 ~8ms@8T（>总纲 4ms 目标但在帧预算内；§7 单线程估算实测偏乐观 3×）、睡眠常态 0.066ms、1080p 全图崩塌 9.6ms@16T 仍可 60Hz；地图上限的真实约束 = 同时活跃面积（本机 ~百万 cell ≈ 8–10ms）而非驻留面积；与 Noita 对照表（调度同构、规则复杂度暂不可比、Layer F 落地后大图结论需重估）。

### Fixed
- **spec §1.4 实施期修订：cell 级冻结脏矩形被 SyncTest 当场击落**——单缓冲扫描允许 tick 内链式移动（整段静止水沿扫描方向一 tick 整体平移，链长无上界），tick 起点冻结的矩形切断链，与全扫语义分叉（实测 tick 583，256×192 场景）。修复：休眠粒度提升为 **chunk 级 + 相位边界唤醒**（重查 dirty ∪ next_dirty，屏障后原子合并结果调度无关），活跃 chunk 全量扫描；等价论证入 spec §1.4。`crates/sand-core/src/scheduler.rs`。这正是 SyncTest 作为常驻执法的第一次实战开张。

## 2026-08-29

### Changed
- **项目大转向（用户裁决）**：东方同人横版动作（Python 原型 → Godot C#）→ **1v1 落沙法术对战，Rust 内核 + Godot 4/gdext 表现层，全栈确定性 lockstep**。产品与内核约束由用户新写的两份文档定义（见 Added）。
- `CLAUDE.md` 全文重写：薄化为协作规范 + 确定性红线速查 §5，技术真源指向总纲/架构文档（消除旧版正文与补丁横幅漂移的模式）；命名约定换 Rust/RON/GDScript。
- **四项翻案落档**（总纲 §11 翻案记录）：①刚体入核心全 lockstep（推翻 R3-A 状态同步）；②温度场作为 Layer F 回归主线（推翻 2026-06-06 降级裁决）；③并行语义换四相棋盘 + r≤16（替代 M0.5 正方形写域）；④RNG key 收敛为 hash(tick,x,y,salt/stream)，pass_id/attempt 维度并入 stream 且实现时必须显式保留。对应旧文档加 Superseded 标注：`docs/overview/architecture.md`、`docs/proposals/2026-06-14-determinism-hardening-r1-r3.md`、`docs/proposals/2026-06-06-deterministic-parallel-and-netcode.md`、`docs/superpowers/specs/2026-05-26-fire-system-design.md`。

### Added
- `docs/overview/kernel-charter.md`：内核顶层设计总纲 v0.1（用户撰写，2026-08-29 采纳为项目宪法）——P1–P5 第一性原则、三层内核（四相 push 网格 + 稀疏粒子 + pull 场）、1v1 延迟制 lockstep、确定性纪律法典、里程碑 M0–M6、决策日志。
- `docs/overview/program-architecture.md`：程序架构文档 v0.1（用户撰写）——四环结构、crate 布局与单向依赖、Ring 0 子系统读写清单、规范 tick 管线（时序即契约）、跨层通信白名单。
- `docs/README.md`：docs 导航入口 + 当前优先队列（CLAUDE.md §3.1 规划已久，首次补建）。
- **Rust workspace 骨架**：`Cargo.toml`（workspace，edition 2024）+ `crates/sand-core`（Ring 0 纯库，暂只载铁律文档注释）+ `crates/sand-harness`（CLI stub）+ `clippy.toml` disallowed_types deny std HashMap/HashSet（charter §6 执法，**红绿验证**：临时违规代码确认 clippy 报错后移除）。`cargo clippy`/`cargo test` 全绿。commit `453eec0`。

### Proposed
- `docs/superpowers/specs/2026-08-29-m0-skeleton-design.md`：M0 实现级设计（用户批准六节设计 + 两裁决：M0 即上 rayon、水走简版横流；GIF 占位渲染器并入 M0）。要点：Cell u32（8 位世代戳替代总纲的 1-bit 奇偶位——自审抓到陈旧位撞车 bug，M0.5 决策①同款结论）、脏矩形原子 min/max 合并为相内唯一共享写、WriteWindow debug 写域断言、SyncTest 四配置（1/N 线程 × 跳过开关）、材料表走 RON + InitConfig 注入保持 Ring 0 零 I/O。

### Removed
- **Python 原型归档**：`prototype/` → `archive/prototype-python/`（只读史料 + README 定性：算法语义参考，不做一对一移植）。83 tests 与 M0/M0.5 成果封存，commit `f5f2371`。

## 2026-06-14

### Fixed
- **液体/气体"方向承诺"bug（day-one 缺陷，非 M0/M0.5 回归）**：`_move_liquid`/`_move_gas` 侧移走 `-vel` 方向时不翻转方向记忆，下帧先试 `+vel`（= 刚腾出的空格）→ 表面像素在两格间永久打乒乓，净输运为零，**液面冻结成沙堆形**（盆中水柱 6000 帧 profile 一字不变；单像素轨迹追踪铁证）。三版本（pre-M0 / M0 / M0.5）行为一致证明非回归。修复后水柱 600 帧摊平至 spread 1（修复前 13）。`prototype/core/rules.py`（侧移段方向承诺）；新增 3 测试（方向承诺 ×2 + 液面摊平守恒）`prototype/tests/test_rules.py`。72 passed；benchmark 27.2/13.9 FPS 无回退（基线 27.1/14.0）。注意：本修复改变模拟语义，既往 hash 序列作废（与 M0.5 同口径）。commit `fcc9312`。

### Fixed
- **R1 加载顺序确定性（A1 实施完成）**：`material.py` type_id 改按 `sorted(material names)` 分配（原按 toml 声明序），解耦 C# `Dictionary` 枚举序——消除跨平台 type_id 漂移 → state_hash 不一致的隐患。新增 `test_materials.py::test_type_id_assigned_by_sorted_name`（非字母序 fixture 红绿）+ `tests/test_load_order.py`（D3 capstone：真实 toml type_id 按 name 排序 + 双载 hash 一致）。83 passed。type_id 重排 → 既往 state_hash 序列作废（语义等价，录放/同 seed 等价测试不受影响）。A2（reaction 排序）经核对为非 live bug 已砍。commits `f3a9600`、`ef48c8d`。

### Added
- `docs/reference/ep01-sandsim-comparison.md`：外部参考实现对照（GameEngineering/EP01_SandSim，C+OpenGL 教学 demo，main.c 3215 行通读）。结论：算法/确定性/数据驱动/并行我们全面更强（EP01 硬编码材质 + `rand()` 非确定 + 无 chunk）；EP01 唯一不可替代价值是**实机渲染参考**——bloom 后处理、整纹理上传+NEAREST、velocity 多格手感、fire 视觉分层。可落地借鉴项已并入路线（velocity 队列 #2、bloom/fire 视觉留 Phase 2）。
- `docs/reference/2026-06-14-deterministic-physics-netcode-survey.md`：刚体/物理确定性联机方案专项调研（deep research，5 角度 / 23 源 / 25 条对抗式验证，23 confirmed / 2 killed）。结论：**Teardown 2026-03 混合架构（破坏走确定性命令流 + 刚体走状态同步）是与我们同构的直接商用先例**，"刚体走状态同步"是合理默认而非无奈；Box2D 3.1 默认已跨平台确定（无需定点，但需关 FMA + 确定接触顺序 + 无 rollback）、Quantum 全栈定点已出货 32 人物理。为 R3 三路线对比提供依据。
- `docs/reference/2026-06-14-tech-route-critical-review.md`：技术路线批判性复核（deep research，5 角度 / 21 源 / 25 条对抗式验证，15 confirmed / 10 killed）。结论：多数决策有一手先例支撑（定点+counter RNG、正方形写域、运动学、单缓冲布局），**联机 lockstep 是最高风险**。
- `docs/overview/architecture.md`：架构总览导航（三阶段 + M0–M3 里程碑 + Phase 1 玩法队列 + 最终架构两支柱 + 代码地图 + 文档指针 + 不变量速查）。新建 `docs/overview/` 目录。

### Proposed
- `docs/proposals/2026-06-14-determinism-hardening-r1-r3.md`：R1 迭代顺序 + R3 刚体桥接确定性加固方案。**R1**：审计确认 sim 热路径无顺序依赖，只加载期两处（`material.py:43` type_id 按 toml 序、`reaction.py:35` tag set 枚举序）→ 改 `sorted()` + 打乱顺序防回归测试（Phase 1 即做，~1h）。**R3**：刚体取 **R3-A（Teardown 式混合）**——刚体属实体层、不进地形 tick，走状态同步+客户端预测；R3-B（Box2D 3.1 lockstep）/R3-C（全栈定点）留作 M2 评估的升级路径。同步 architecture §5 加刚体归属句。

### Changed
- **采纳 deep research 复核（2026-06-14），3 项 actionable 落账**：①**R1** proposal §3 **D3 契约补强**——sim 遍历禁裸 `Dictionary`/`HashSet`、强制稳定排序（Box2D 作者亲证无序迭代即非确定；M1 C# 迁移头号风险），同步 architecture §8 不变量；②**R2**（用户校正）proposal §4.4 澄清 Noita Entangled Worlds **不构成 lockstep 反证**——它走状态同步是被闭源不确定引擎所迫（没得选），仅证退路 C 工程可行；**路线 B 仍首选，退路 C 维持兜底，M2 实测确认，不因它调整 B/C 优先级**；③**R3** 刚体桥接浮点确定性陷阱可能使"地形/弹幕/刚体"成三层同步，记入开放问题。另确认：velocity 用 8.8 定点累加器（非概率取整）；C# 数据布局不预设 SoA 收益（"SoA 必快"论断 0-3 否决，需实测）。涉及 `docs/proposals/2026-06-06-deterministic-parallel-and-netcode.md`、`docs/overview/architecture.md`。
- **液体/气体 dispersion rate**（spec `docs/superpowers/specs/2026-06-07-liquid-dispersion-design.md`、plan `.../plans/2026-06-07-liquid-dispersion-plan.md`，commits `fecdc68`→`951f0fa`）：
  材质字段 `dispersion`（water 5/oil 2/lava 1/steam 3，缺省 1），横移一帧沿方向记忆探测最多 N 格、落最远连续 AIR，
  首格保留 ±1 密度置换；探测纯确定（无 RNG）、写域边界夹断。`_move_liquid`/`_move_gas` 共用 `_probe_side` helper（`prototype/core/rules.py`）。
  摊平收敛 800→100 帧（≈8×）；benchmark 128² 26.6 FPS / 192² 13.2 FPS（基线 27.2/13.9，变化 -2.2%/-5.0%，预算内）。
  新增 5 测试 + 1 速度契约测试 + 更新 2 个 ±1-era 测试为方向承诺不变量断言（80 passed）。hash 序列作废（语义变更，与 M0.5/方向承诺修复同口径）。
- `prototype/demo_density.py`：密度沉浮演示场景（沙穿水下沉、油浮上水面）。
- `docs/sessions/2026-06-14-leveling-fix-and-dispersion.md`：本会话总账（液面冻结修复 + dispersion 实施）。

## 2026-06-07

### Added
- **M0.5 单线程 4-pass/chunk 调度器完成**（提案 §5 M0.5 行；fresh evidence：69 passed / 缝隙守恒 / 产物盖戳红绿验证 / replay 仍确定 / M0 hash 序列按预期作废）：
  - `prototype/core/chunks.py`：正方形写域 `[chunk−32, chunk+96)²` + 4-pass parity 纯几何，含"同 pass 写域两两不相交"穷举单测。
  - `prototype/core/grid.py`：update() 重写为所有权制 pass→chunk 扫描；STRIDE 4→5 加 `UPDATED_AT` 世代戳（swap 双方盖戳、`set_cell(stamp=)` 显式参数——决策②）；**删 FLAG_DIRTY/FLAG_STATIC 与每帧清 flag pass**（决策①）；`_check_reactions` 读域夹断 + 产物盖戳；RNG pass_id 接线。
  - `prototype/tests/test_chunks.py` + `test_chunked_semantics.py`（192×128 多 chunk）：材质计数守恒（缝隙无源汇）、沙柱跨水平缝、水过垂直缝、多 chunk 污染测试、产物同帧不动（**红绿验证**：去掉盖戳必红）、写域拒绝直测。
  - 性能意外向好：128² **27.1 FPS，较 M0 后 +18%**（删 O(N) 清 flag pass 收益 > 调度开销），基本回到 M0 前水平；192² 14.0 FPS 数据点入档（决策③）。
- `docs/superpowers/specs/2026-06-07-m05-chunked-scheduler-design.md` + `plans/2026-06-07-m05-chunked-scheduler-plan.md`；提案 §2.3 row 5 同步决策②偏离（观察契约不变）。
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
