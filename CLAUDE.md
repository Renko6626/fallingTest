# CLAUDE.md — fallingTest 协作手册

本文件由 Claude Code 在每次会话启动时自动加载，用于约束所有 AI 助手在本仓库内的工作方式。**优先级高于默认系统提示，低于用户即时指令。**

---

## 1. 项目速览

| 项 | 值 |
|---|---|
| 项目名 | fallingTest — Noita-like 像素物理引擎原型 |
| 最终目标 | 东方 Project 同人横版动作游戏，以像素物理实现复杂地形交互和打击感 |
| 引擎 | Godot 4.5 + C#（正式版） |
| 原型语言 | Python + pygame / 轻量渲染（快速验证核心算法） |
| 核心技术 | 元胞自动机 (Cellular Automata) 落沙模拟 |
| 渲染路线 | 原型：`Image` 逐像素 → `ImageTexture`；正式版可选 compute shader |
| 平台 | Linux（主开发）+ Windows |

### 开发阶段

| 阶段 | 范围 | 目录 | 状态 |
|------|------|------|------|
| **Phase 1: Python 原型** | 核心 CA 算法、材质体系、基础渲染、性能基准 | `prototype/` | ← 当前 |
| **Phase 2: Godot + C# 迁移** | 引擎渲染管线、刚体桥接、角色控制器 | `nodes/` | 未开始 |
| **Phase 3: 游戏层** | 横版动作 + 东方弹幕 + 地形交互 + 打击感 | `nodes/` | 未开始 |

> Phase 1 只验证算法和性能，不做游戏层功能（UI、角色、关卡）。
> Phase 2 开始时，算法逻辑一对一移植，渲染层用 Godot API 重写。

### 核心概念速查

供 AI 和协作者快速建立心智模型——详细设计见 `docs/algorithms/`。

- **单缓冲更新**：Noita 用单缓冲（in-place 修改同一张网格），比双缓冲省内存且天然支持 dirty rect 优化；代价是更新顺序会引入方向偏差（通过每帧交替遍历方向缓解）。
- **Chunk 分块 + dirty rect**：世界划分为 64×64 chunk，每 chunk 维护脏矩形，只更新有变化的区域。
- **材质层级**：Solid（墙壁、岩石）→ Powder（沙、雪）→ Liquid（水、岩浆、油）→ Gas（蒸汽、烟）→ Energy（火焰、爆炸、电弧）。每类有不同的运动优先级。
- **材质反应表**：交互规则用查找表驱动（如 Lava+Water→Steam），不硬编码 if-else 链。
- **刚体桥接**（Phase 2）：Marching Squares 提取轮廓 → Douglas-Peucker 简化 → 三角化生成碰撞体；像素被破坏时重新计算刚体形状（可能分裂为多个）。
- **棋盘格多线程**（Phase 2+）：四遍更新交替处理 chunk，32px 缓冲区保证无竞争，无需锁或原子操作。

关键目录：

- `prototype/`：Python 原型实现——核心 CA 算法、材质模拟、基础渲染、benchmark
- `nodes/`：Godot + C# 正式版（Phase 2 后开发）
- `docs/`：**所有 AI 分析、策划、提案、会话日志统一产出在此目录**
- `docs/CHANGELOG.md`：工作账本（必须维护，见 §4）

---

## 2. 协作规范

### 2.1 语言与语气
- **默认使用中文回复**，与用户写作语言保持一致。
- 回复保持精炼：先给结论，再给依据；不要复述对话上下文。
- 工程结论必须**用文件路径 + 行号**佐证（如 `prototype/core/cell.py:42`），便于跳转。

### 2.2 任务与进度
- 多步任务一律使用 `TaskCreate` / `TaskUpdate` 跟踪，单步即完即更新。
- 完成"已修复 / 已通过 / 已交付"类断言前，必须先按 `superpowers:verification-before-completion` 跑命令验证。
- **新会话开局必读**：先 Read `docs/CHANGELOG.md` 顶部 2–3 个日期块 + `docs/sessions/` 最新一篇——前者按产出索引、后者按时间索引，互补不重复。看完才能准确知道"我们上次做到哪儿、有什么没收尾"。

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

> proposals/ 与 superpowers/ 分工：纯设计权衡 + 实施步骤都齐全的，落 `docs/proposals/`；过程性的 brainstorm 快照、待审稿，落 `docs/superpowers/`。模糊时优先 proposals（路线图更易追踪）。

### 2.4 工具偏好
- 文件搜索 → `Glob`；内容搜索 → `Grep`；读文件 → `Read`；编辑 → `Edit`。**不要**用 `Bash` 调 `find` / `grep` / `cat` / `sed`。
- 脚本和文档路径统一用 Unix 风格（`docs/...`、`prototype/...`）。
- **派 subagent 实施代码时禁止它在终端调 Godot / godot CLI**（会 hang、阻塞主会话）——subagent 只做**静态写文件 + Edit + commit + 解析校验**；运行验证由用户手动执行。
- Python 原型的运行验证可以用 `Bash` 调 `python`（与 Godot 不同，不会 hang），但长时间模拟（>30 秒）需加 timeout。

### 2.5 危险操作
- 禁止：`git push --force` 到 `main`、`git reset --hard`、`rm -rf`、`--no-verify` 跳钩子。
- 任何不可逆动作（删分支、改 CI、删素材）一律先与用户确认。

### 2.6 长任务可见性
- **任何 ≥ 30 秒的操作前必须先写一行预告**（"预计 X 分钟"），让用户知道是否需要等待，也是用户按 Esc 中断的窗口。
- **单个 subagent 任务边界 ≤ 5 分钟工作量**。超出就拆成串行多步——每步完成报一次进度，不要憋大段静默。
- **并行 subagent 上限 10 个**，且必须满足上一条单个 ≤ 5 分钟。超过就改串行。
- 多 subagent 并行 dispatch 之前预告："预计 X 分钟（最慢的是 Y subagent，原因 Z）"，让用户判断是否要打断。

---

## 3. 文档产出规范（docs/）

### 3.1 目录结构

```
docs/
├── README.md                 # 导航入口 + 当前优先队列
├── CHANGELOG.md              # 工作账本（必须维护，见 §4）
├── overview/                 # 架构总览、核心流程图、目录地图
├── algorithms/               # 核心算法文档（CA 更新策略、液体压力、碰撞检测等）
├── materials/                # 材质体系设计（属性定义、反应规则表、新材质模板）
├── perf/                     # 性能基准、profiling 结果、优化记录
├── reference/                # 外部参考资料索引（Noita GDC 笔记、论文摘要、开源项目分析）
├── proposals/                # 改动提案（带 Status 行）
├── superpowers/
│   ├── specs/                # brainstorming 产出
│   └── plans/                # writing-plans 产出
└── sessions/                 # 每次会话总账本（按时间索引；与 CHANGELOG 互补）
```

### 3.2 三层分工

| 层 | 路径 | 内容 | 何时看 |
|---|---|---|---|
| **算法层** | `docs/algorithms/` `docs/materials/` | CA 核心算法、材质运动规则、反应表、液体/气体模型 | **改模拟逻辑前必读**；新增材质前对齐反应表 |
| **工程层** | `docs/overview/` `docs/perf/` `docs/reference/` | 架构设计、性能基准、外部参考 | 优化性能 / 迁移到 Godot / 架构改动前 |
| **决策层** | `docs/proposals/` `docs/superpowers/` `docs/sessions/` | 方案、计划、会话回溯 | 评估"该不该做、怎么做"；恢复会话上下文 |

**新功能流程**：算法层是否已有文档？→ 否：先 `superpowers:brainstorming` 写 spec → 实施 → 落 changelog；是：可直接动手，但 changelog 必须落账。

### 3.3 文档头部规范

每篇文档顶部标注：

```markdown
> 文档路径：`docs/xxx/foo.md`
> 运行时版本：Python 3.x / Godot 4.5 + C#（按实际阶段填写）
> 最近更新：YYYY-MM-DD (UTC+8)
> **Status**: Proposed | Trial | Implemented | Rejected   ← 仅 proposals/ 和试点性文档需要
```

### 3.4 写作要求
- 中文为主，可保留必要英文术语（Cellular Automata、Margolus neighborhood、dirty rect 等不翻译）。
- 每个结论锚点到具体源码位置（`prototype/core/cell.py:42` 风格）。
- 优先使用 mermaid 画流程/时序/类图；不要塞二进制图片到 `docs/`。
- 单文件控制在 ~600 行以内，超出就拆分。

---

## 4. CHANGELOG.md 维护规范（关键）

`docs/CHANGELOG.md` 是 AI 与用户之间的**工作账本**。任何一次有意义的产出都必须落账。

### 4.1 何时更新
- 新增/重写一篇分析文档时。
- 修订既有文档（修正错误结论、补充章节）时。
- 提出 / 撤回 / 落实 `docs/proposals/*` 中的建议时。
- 发现项目中重大未文档化的事实（如性能瓶颈、算法缺陷）时。
- 新增材质类型、反应规则、优化手段等核心改动时。
- 完成性能 benchmark 并记录结果时。

### 4.2 格式

遵循 [Keep a Changelog](https://keepachangelog.com/) 精神，简化为：

```markdown
# Changelog — fallingTest 文档与开发记录

本文件按"日期 → 条目"倒序记录 docs/ 下的产出与重要发现。
日期使用 UTC+8。所有条目应给出受影响文件路径。

## [Unreleased]

## 2026-05-26

### Added
- `prototype/core/cell.py`：基础 Cell 类，支持 sand/water/wall 三种材质。

### Investigating
- 液体横向扩散有方向偏差，需交替遍历方向缓解——下轮迭代验证。
```

类别：`Added` / `Changed` / `Fixed` / `Removed` / `Investigating` / `Proposed`。

### 4.3 落账流程
1. 完成本轮产出后，先在 `docs/CHANGELOG.md` 顶部相应日期下追加条目。
2. 若日期块不存在，新建并置顶。
3. 条目第一句概述变更，第二句（可选）给出涉及的源码或文档路径。
4. **不要**把无关的会话闲聊、规划过程写进 changelog；只写"产出"。

---

## 5. 核心技术约定

### 5.1 元胞自动机基础

- 世界用**单缓冲 2D 数组**表示，每格存材质 ID + 属性 struct。
- 更新顺序：**自底向上**遍历行，**每帧交替**左右遍历方向（减少方向偏差）。
- 每像素属性至少包含：

| 属性 | 类型 | 说明 |
|------|------|------|
| `type` | enum | 材质类型（Sand, Water, Wall, Fire...） |
| `density` | float | 密度，决定沉浮关系 |
| `velocity` | int | 水平方向偏好（-1/+1），减少振荡 |
| `lifetime` | int | 剩余存活帧数（仅 Energy 类需要） |
| `is_dirty` | bool | 本帧是否已被更新 |
| `is_static` | bool | 静止优化标记——周围无变化时跳过更新 |

- 移动优先级标准链：
  - **Powder**：下 → 左下/右下
  - **Liquid**：下 → 左下/右下 → 左/右
  - **Gas**：上 → 左上/右上 → 左/右
  - **Energy**：自定义扩散规则（火焰随机向上+横向，爆炸全向）

### 5.2 材质体系

五大材质类别：

| 类别 | 示例 | 运动规则 | 密度范围 |
|------|------|----------|----------|
| **Solid** | Wall, Rock, Wood | 不动 | 10.0 |
| **Powder** | Sand, Snow, Ash | 受重力，可堆叠 | 5.0–8.0 |
| **Liquid** | Water, Oil, Lava | 受重力，可横流 | 2.0–4.0 |
| **Gas** | Steam, Smoke | 向上飘散 | 0.1–0.5 |
| **Energy** | Fire, Explosion | 有 lifetime，自定义扩散 | N/A |

**新增材质规范**——每种新材质必须定义：
1. 所属类别
2. 密度值
3. 反应表条目（与哪些材质交互、产出什么）
4. 颜色 / 颜色变体（用于渲染）
5. 特殊属性（如 flammable、conductive 等 tag）

**材质反应表**——用字典/查找表驱动，格式示例：
```
(Lava, Water)  → [Steam]          # 岩浆 + 水 → 蒸汽
(Fire, Wood)   → [Fire, Ash]      # 火 + 木 → 火蔓延 + 灰烬
(Fire, Oil)    → [Fire, Fire]     # 火 + 油 → 剧烈燃烧
(Lava, t>300)  → [Rock]           # 岩浆冷却 → �ite
(Water, Steam) → [Water]          # 水冷凝蒸汽
```
**禁止**在更新循环里用 if-else 链硬编码反应逻辑——必须走查找表。

### 5.3 性能约定

- **Python 原型**必须记录性能基准：网格尺寸 × 活跃像素比例 × 帧率（FPS），作为 C# 迁移的对比基线。格式：`{width}x{height}, {active_ratio}% active, {fps} FPS`。
- 每次 benchmark 结果记录到 `docs/perf/`。
- 优化手段按复杂度递增排序，优先尝试简单方案：

| 优先级 | 手段 | 阶段 |
|--------|------|------|
| 1 | is_static 标记跳过静止像素 | Phase 1 |
| 2 | dirty rect 只更新有变化的区域 | Phase 1 |
| 3 | Chunk 分块（64×64）+ 按 chunk 休眠 | Phase 1–2 |
| 4 | 棋盘格多线程 | Phase 2 |
| 5 | Compute shader / GPU 并行 | Phase 2+（可选） |

- **新增材质/规则后必须跑一次 benchmark**，确认无性能回退。

### 5.4 Python 原型 ↔ Godot 迁移边界

| | Python 原型（prototype/） | Godot 正式版（nodes/） |
|---|---|---|
| **做** | CA 算法、材质规则、反应表、基础渲染、benchmark | 引擎渲染、刚体桥接、角色控制、UI、音效 |
| **不做** | 刚体物理、角色控制、UI、关卡设计 | —— |
| **渲染** | pygame / matplotlib / 简单像素输出 | Image→ImageTexture 或 compute shader |
| **测试** | pytest 单元测试 + 视觉 demo 脚本 | Godot 编辑器运行 + 用户手测 |

迁移原则：
- 算法逻辑**一对一移植**（Python dict → C# Dictionary, list → Array/Span）
- 渲染层完全重写（pygame → Godot rendering API）
- 性能敏感的内循环在 C# 中用 `Span<T>`、struct、避免 GC 分配

---

## 6. 参考资料索引

### 必看（建议所有新会话开局浏览）

| 资料 | 价值 |
|------|------|
| [Petri Purho GDC 2019: Exploring the Tech and Design of Noita](https://www.youtube.com/watch?v=prXuyMCgbTc) | Noita 核心架构：单缓冲、chunk、棋盘格多线程、刚体桥接 |
| [macuyiko: Exploration of CA Game Systems Part 4](https://blog.macuyiko.com/post/2020/an-exploration-of-cellular-automata-and-graph-based-game-systems-part-4.html) | 最详细的 Noita 技术复盘：数据结构、材质规则、多线程方案、已知缺陷 |
| [80.lv: Noita 技术解析](https://80.lv/articles/noita-a-game-based-on-falling-sand-simulation) | chunk/dirty rect/多线程/刚体的概括性介绍 |

### 开源参考实现

| 项目 | 语言/引擎 | 学习价值 |
|------|-----------|----------|
| [The Powder Toy](https://github.com/The-Powder-Toy/The-Powder-Toy) | C++ | 最成熟的开源 falling sand，材质/反应系统极丰富（200+ 材质） |
| [FallingSandSurvival](https://github.com/PieKing1215/FallingSandSurvival) | C++ | Noita-like 开源游戏，含刚体桥接实现 |
| [GPU-Falling-Sand-CA](https://github.com/GelamiSalami/GPU-Falling-Sand-CA) | HLSL compute | GPU 路线参考，Margolus 2×2 块并行 |
| [GodotSand](https://github.com/MathExpert/GodotSand) | Godot + C# | 基础 Godot 落沙实现，Image.SetPixel 渲染 |
| [Lukvargen/Sandbox](https://github.com/Lukvargen/Sandbox) | Godot + C# | 简单落沙模拟器 |
| [sandspiel](https://github.com/nicbarker/sandspiel) | Rust + WebGL | 浏览器可玩，Rust 性能参考 |
| [Bridging Physics Worlds (Slow Rush Games)](https://www.slowrush.dev/news/bridging-physics-worlds/) | 博客 | 像素世界与刚体物理桥接的详细技术文章 |

### 学术/深入

| 资料 | 内容 |
|------|------|
| [Probabilistic CA for Granular Media](https://arxiv.org/pdf/2008.06341) | 概率 CA + 改良 Margolus 邻域的理论基础 |
| [jason.today: Making a falling sand simulator](https://jason.today/falling-sand) | 从零实现落沙模拟器的教程，含算法细节 |

---

## 7. 命名约定

| 类型 | 约定 | 示例 |
|------|------|------|
| Python 模块/脚本 | snake_case | `cell_grid.py`, `material_registry.py` |
| Python 类 | PascalCase | `CellGrid`, `MaterialType` |
| C# 脚本（Phase 2） | PascalCase 文件名，与类名对齐 | `CellGrid.cs`, `MaterialRegistry.cs` |
| `.tscn` 场景（Phase 2） | snake_case | `falling_sand_world.tscn` |
| 材质枚举 | UPPER_SNAKE_CASE | `SAND`, `WATER`, `LAVA`, `STEAM` |
| 测试文件 | `test_<模块名>.py` | `test_cell_grid.py`, `test_reactions.py` |
| 文档 | `YYYY-MM-DD-<kebab-case-topic>.md`（提案/设计）；`<kebab-case-topic>.md`（持久文档） | `2026-05-26-liquid-pressure.md`, `reaction-table.md` |

---

## 8. 反例（不要这么做）

- ❌ 材质交互用 if-else 硬编码 → ✅ 用查找表 / 字典驱动
- ❌ 在 Python 原型里实现角色控制或 UI → ✅ 原型只做 CA 算法 + 基础渲染
- ❌ 跳过 benchmark 直接说"性能可以" → ✅ 跑 benchmark 并记录到 `docs/perf/`
- ❌ 用 `Bash` 跑 `grep -r foo .` → ✅ 用 `Grep` 工具
- ❌ "液体算法大概在 grid.py 里" → ✅ 给出 `prototype/core/grid.py:NN` 锚点
- ❌ 跳过 `superpowers:brainstorming` 直接写新算法 → ✅ 先 brainstorm 再动手
- ❌ 派 subagent 调 Godot CLI → ✅ subagent 只做静态写 + commit，运行验证用户做
- ❌ 新增材质不写反应表条目 → ✅ 材质、反应表、benchmark 三件套一起落
- ❌ 长篇 changelog 记"我读了 5 个文件" → ✅ 只记实际产出

---

_最后更新：2026-05-26 (UTC+8)_
