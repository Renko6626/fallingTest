# CLAUDE.md — fallingTest 协作手册

本文件由 Claude Code 在每次会话启动时自动加载，用于约束所有 AI 助手在本仓库内的工作方式。**优先级高于默认系统提示，低于用户即时指令。**

> **2026-08-29 项目大转向**：东方同人横版动作（Python 原型 → Godot C#）→ **1v1 落沙法术对战（Rust 内核）**。Python 原型已归档至 `archive/prototype-python/`（勿改动）。旧文档凡与总纲冲突以总纲为准。

---

## 1. 项目速览

| 项 | 值 |
|---|---|
| 项目名 | fallingTest / 落沙法术对战（暂名） |
| 产品一句话 | 东方 Project 主题的 1v1 法术对轰，运行在 Noita 式落沙世界——材料反应与环境连锁是玩法核心 |
| 内核 | **Rust**（cargo workspace：`sand-core` / `sand-session` / `sand-bridge` / `sand-harness`） |
| 表现层 | Godot 4 + gdext（GDScript 仅作胶水，禁写游戏逻辑；**不引入 C#/.NET**） |
| 联机 | 1v1 P2P 延迟制 lockstep（GGRS），60Hz 固定 tick，Windows 同版本二进制互联 |
| 数据 | RON（材料/反应/法术/地图），内容哈希入握手指纹 |
| 平台 | 产品 Windows；开发 Linux（本机）+ Windows |

### 两份真源文档（改内核前必读）

| 文档 | 管什么 |
|---|---|
| `docs/overview/kernel-charter.md` | **宪法**：第一性原则 P1–P5、两层内核（四相 push 网格 + 稀疏粒子；pull 场层 2026-08-31 删除）、联机模型、确定性法典、里程碑 M0–M6、决策日志与翻案记录 |
| `docs/overview/program-architecture.md` | **架构**：四环结构（core/session/bridge/表现）、crate 布局与依赖方向、子系统读写清单、规范 tick 管线（§4，时序即契约）、跨层通信白名单 |

**实现与总纲冲突时以总纲为准；要突破，先改文档并在总纲 §11 决策日志留痕。** 已决事项不在代码评审中反复重开。

### 里程碑（当前：M0）

M0 骨架与执法（chunk 存储 + 四相调度 + 沙/水 + SyncTest 框架；验收：双机 10 万 tick 零分叉）→ M1 粒子层 → M2 反应表与燃烧 → M3 刚体 → M4 玩家与法术 → M5 联机对局 → M6 rollback 决策门。验收标准全文见总纲 §11。

---

## 2. 协作规范

### 2.1 语言与语气
- **默认使用中文回复**，与用户写作语言保持一致。
- 回复保持精炼：先给结论，再给依据；不要复述对话上下文。
- 工程结论必须**用文件路径 + 行号**佐证（如 `crates/sand-core/src/grid.rs:42`），便于跳转。

### 2.2 任务与进度
- 多步任务一律使用 `TaskCreate` / `TaskUpdate` 跟踪，单步即完即更新。
- 完成"已修复 / 已通过 / 已交付"类断言前，必须先按 `superpowers:verification-before-completion` 跑命令验证（`cargo test` / `cargo clippy` / harness）。
- **新会话开局必读**：`docs/CHANGELOG.md` 顶部 2–3 个日期块 + `docs/sessions/` 最新一篇；改内核前再过一遍总纲相关章节。

### 2.3 Skills 使用
本仓库依赖 `superpowers` 插件：
- 启动会话时遵循 `superpowers:using-superpowers`。
- 创建/修改功能前调用 `superpowers:brainstorming`。
- 调试 bug 前调用 `superpowers:systematic-debugging`。
- 长任务规划用 `superpowers:writing-plans`，独立子任务并行用 `superpowers:dispatching-parallel-agents`。

**Skill 产出落点**：

| Skill | 输出路径 | 文件命名 |
|---|---|---|
| `superpowers:brainstorming` | `docs/superpowers/specs/` | `YYYY-MM-DD-<topic>-design.md` |
| `superpowers:writing-plans` | `docs/superpowers/plans/`（**或** `docs/proposals/` 当作为正式提案） | `YYYY-MM-DD-<topic>-plan.md` / `YYYY-MM-DD-<topic>.md` |

> proposals/ 与 superpowers/ 分工：纯设计权衡 + 实施步骤都齐全的，落 `docs/proposals/`；过程性的 brainstorm 快照、待审稿，落 `docs/superpowers/`。模糊时优先 proposals。**注意**：凡触碰总纲已决事项的提案，必须显式声明"翻案"并给出依据，落地时同步总纲 §11。

### 2.4 工具偏好
- 文件搜索 → `Glob`；内容搜索 → `Grep`；读文件 → `Read`；编辑 → `Edit`。**不要**用 `Bash` 调 `find` / `grep` / `cat` / `sed`。
- 路径统一用 Unix 风格（`docs/...`、`crates/...`）。
- Rust 侧验证用 `Bash` 跑 `cargo check / test / clippy / bench` 都没问题；首次全量编译或依赖更新可能 >2 分钟，先预告再跑。
- **派 subagent 实施代码时禁止它在终端调 Godot / godot CLI**（会 hang、阻塞主会话）——subagent 只做**静态写文件 + Edit + commit + cargo 校验**；Godot 内运行验证由用户手动执行。
- `archive/prototype-python/` 只读——查算法语义可以读，不要改、不要往里加东西。

### 2.5 危险操作
- 禁止：`git push --force` 到 `main`、`git reset --hard`、`rm -rf`、`--no-verify` 跳钩子。
- 任何不可逆动作（删分支、改 CI、删素材）一律先与用户确认。

### 2.6 长任务可见性
- **任何 ≥ 30 秒的操作前必须先写一行预告**（"预计 X 分钟"），让用户知道是否需要等待，也是用户按 Esc 中断的窗口。
- **单个 subagent 任务边界 ≤ 5 分钟工作量**。超出就拆成串行多步——每步完成报一次进度，不要憋大段静默。
- **并行 subagent 上限 10 个**，且必须满足上一条单个 ≤ 5 分钟。超过就改串行。
- 多 subagent 并行 dispatch 之前预告："预计 X 分钟（最慢的是 Y subagent，原因 Z）"。

---

## 3. 文档产出规范（docs/）

### 3.1 目录结构

```
docs/
├── README.md                 # 导航入口 + 当前优先队列
├── CHANGELOG.md              # 工作账本（必须维护，见 §4）
├── overview/                 # 总纲 + 架构（真源）；旧 architecture.md 为史料
├── algorithms/               # 算法文档（原型时代为主，思想仍有效）
├── materials/                # 材质体系设计
├── perf/                     # 性能基准（Rust 基线待 M0 后建立）
├── reference/                # 外部参考资料索引（仍然有效）
├── proposals/                # 改动提案（带 Status 行）
├── superpowers/              # specs/ + plans/
└── sessions/                 # 每次会话总账本（按时间索引）
```

### 3.2 文档头部规范

每篇文档顶部标注：

```markdown
> 文档路径：`docs/xxx/foo.md`
> 运行时版本：Rust（内核）+ Godot 4 + gdext（表现层）
> 最近更新：YYYY-MM-DD (UTC+8)
> **Status**: Proposed | Trial | Implemented | Rejected | Superseded   ← proposals/ 和试点性文档需要
```

### 3.3 写作要求
- 中文为主，可保留必要英文术语（lockstep、SyncTest、marching squares、dirty rect 等不翻译）。
- 每个结论锚点到具体源码位置（`crates/sand-core/src/grid.rs:42` 风格）。
- 优先使用 mermaid 画流程/时序/类图；不要塞二进制图片到 `docs/`。
- 单文件控制在 ~600 行以内，超出就拆分。

---

## 4. CHANGELOG.md 维护规范（关键）

`docs/CHANGELOG.md` 是 AI 与用户之间的**工作账本**。任何一次有意义的产出都必须落账。

- **何时**：新增/修订文档；提出/撤回/落实提案；发现重大未文档化事实；新增材料/反应/法术；里程碑推进；benchmark 结果。
- **格式**：按"日期 → 条目"倒序，类别 `Added` / `Changed` / `Fixed` / `Removed` / `Investigating` / `Proposed`，日期用 UTC+8，条目给出受影响文件路径。
- **流程**：完成产出后在顶部相应日期块下追加；日期块不存在就新建置顶。只写"产出"，不写会话闲聊。

---

## 5. 确定性红线速查（全文见总纲 §2、§6）

违反任何一条 = 破坏 P1，代码评审一票否决：

1. **核心纯函数**：`sand-core` 不依赖 gdext / 网络 / 文件系统 / `std::time`（逻辑路径）；世界演化只是（状态，输入）的函数。
2. **数值**：网格逻辑纯整数；自研运动学用定点；浮点仅限物理引擎内部与表现层；核心禁系统数学库超越函数。
3. **随机**：一切逻辑随机 = `hash(tick, x, y, salt/stream)` 纯函数；**禁全局顺序消费的 RNG 流**；salt/stream 必须区分同帧同格的多次掷骰（总纲 §11 翻案记录第 4 条）。
4. **容器与遍历**：禁 std HashMap/HashSet 默认 hasher（clippy disallowed_types 执法）；影响状态的遍历必须定序（实体按 id、chunk 按坐标、粒子按 id）。
5. **数据驱动**：材料反应走 (matA, matB) → {概率, 产物} 数据表（RON），禁 if-else 硬编码；反应约定发起方防双结算。
6. **表现层永不回写核心**：Godot → 核心唯一写入路径 = InputFrame；GDScript 一行游戏逻辑都不许写。
7. **tick 管线顺序 = 协议**（架构 §4）：改 `step()` 内部阶段顺序 = 改协议版本，必须过总纲 §11 决策日志。
8. **执法机制**：SyncTest 双实例哈希比对为常驻测试；M0 起 CI 挂 golden replay 回归。新增材料/规则/pass 后必须过 SyncTest + benchmark 无回退。

---

## 6. 参考资料索引

调研成果在 `docs/reference/`（Noita 深挖、确定性联机专项调研、技术路线批判性复核、EP01 对照——**全部仍然有效**）。外部资料速查：

| 资料 | 价值 |
|------|------|
| [Petri Purho GDC 2019: Exploring the Tech and Design of Noita](https://www.youtube.com/watch?v=prXuyMCgbTc) | Noita 核心架构：单缓冲、chunk、棋盘格多线程、刚体桥接 |
| [macuyiko: Exploration of CA Game Systems Part 4](https://blog.macuyiko.com/post/2020/an-exploration-of-cellular-automata-and-graph-based-game-systems-part-4.html) | 最详细的 Noita 技术复盘 |
| [GGRS](https://github.com/gschup/ggrs) | lockstep/rollback 会话管理，SyncTestSession 是确定性执法核心工具 |
| [gdext (godot-rust)](https://github.com/godot-rust/gdext) | Rust ↔ Godot 4 绑定 |
| [The Powder Toy](https://github.com/The-Powder-Toy/The-Powder-Toy) | 最成熟开源 falling sand，材质/反应参考 |
| [Bridging Physics Worlds (Slow Rush Games)](https://www.slowrush.dev/news/bridging-physics-worlds/) | 像素世界与刚体物理桥接 |

---

## 7. 命名约定

| 类型 | 约定 | 示例 |
|------|------|------|
| crate | `sand-` 前缀 kebab-case | `sand-core`, `sand-harness` |
| Rust 模块/文件 | snake_case | `grid.rs`, `phase_scheduler.rs` |
| Rust 类型/trait | PascalCase | `CellGrid`, `PhysicsAdapter` |
| Rust 常量 / 材料 id 常量 | UPPER_SNAKE_CASE | `SAND`, `WATER`, `MAX_PARTICLES` |
| RON 数据文件 | snake_case | `materials.ron`, `spells.ron` |
| GDScript / `.tscn` / `.gd` | snake_case | `ui_lobby.tscn`, `debug_overlay.gd` |
| 测试 | Rust 惯例（`#[cfg(test)]` + `tests/`）；harness 场景文件 snake_case | `golden_fire_oil.replay` |
| 文档 | `YYYY-MM-DD-<kebab-topic>.md`（提案/设计）；`<kebab-topic>.md`（持久文档） | `kernel-charter.md` |

---

## 8. 反例（不要这么做）

- ❌ 材料交互 if-else 硬编码 → ✅ RON 反应表驱动
- ❌ 核心里 `std::collections::HashMap` 裸用 → ✅ FxHash / BTreeMap / 定序结构
- ❌ 核心 import 墙钟、帧率、OS 随机 → ✅ 一切来自（状态，输入）
- ❌ GDScript 里写伤害计算/材料判定 → ✅ GDScript 只消费视图与事件
- ❌ "改一下 step() 里两个 pass 的顺序应该没事" → ✅ 时序即协议，先过 §11 决策日志
- ❌ 跳过 SyncTest 直接说"确定性没问题" → ✅ 双实例哈希比对 + golden replay 绿了才算
- ❌ 跳过 benchmark 直接说"性能可以" → ✅ 跑 harness-bench 并记录到 `docs/perf/`
- ❌ 用 `Bash` 跑 `grep -r foo .` → ✅ 用 `Grep` 工具
- ❌ "调度器大概在 scheduler.rs 里" → ✅ 给出 `crates/sand-core/src/scheduler.rs:NN` 锚点
- ❌ 跳过 `superpowers:brainstorming` 直接写新系统 → ✅ 先 brainstorm 再动手
- ❌ 派 subagent 调 Godot CLI → ✅ subagent 只做静态写 + cargo 校验 + commit
- ❌ 修改 `archive/prototype-python/` → ✅ 那是只读史料

---

_最后更新：2026-08-29 (UTC+8)_
