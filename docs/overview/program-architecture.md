> 文档路径：`docs/overview/program-architecture.md`
> 运行时版本：Rust（内核）+ Godot 4 + gdext（表现层）
> 最近更新：2026-08-29 (UTC+8)
> **Status**: Trial（随总纲于 2026-08-29 一并采纳）

# 落沙法术对战（暂名）· 程序架构文档

版本 v0.1 · 2026-08-15 · 状态：草案
配套文档：`kernel-charter.md`（顶层设计总纲）。总纲管"什么必须永远成立"，本文管"程序由哪些件构成、数据怎么流"。两者冲突时以总纲为准。粒度到模块与数据流，不到函数签名。

---

## 1. 全景图

程序分四个环，越靠内确定性纪律越严。箭头是数据流向，实心箭头是唯一的写入路径。

```
┌─────────────────────────── Godot 进程（Windows）───────────────────────────┐
│                                                                            │
│  Ring 2 · 表现层（GDScript / GLSL / Godot 场景）                             │
│    UI·菜单·大厅界面 │ 世界渲染 │ 音频 │ 装饰碎屑 │ 调试面板                     │
│       ▲ Channel A：只读状态视图    ▲ Channel B：cosmetic 事件                 │
│       │                          │                    │ 原始本地输入         │
│  ┌────┴──────────────────────────┴────────────────────▼─────────────────┐  │
│  │ Ring 2 · sand-bridge（Rust · gdext cdylib）                           │  │
│  │   tick 累加泵 │ 脏块纹理上传 │ 事件分发 │ InputFrame 编码                │  │
│  └────▲──────────────────────────────────────────────▼──────────────────┘  │
│       │ 状态视图 / 哈希                      InputFrame │                     │
│  ┌────┴──────────────────────────────────────────────▼──────────────────┐  │
│  │ Ring 1 · sand-session（Rust）                                         │  │
│  │   握手与指纹校验 │ GGRS lockstep │ desync 检测 │ 回放录制/重放            │  │
│  └────▲──────────────────────────────────────────────▼──────────────────┘  │
│       │ 逐帧状态哈希                          确认输入序列 │                   │
│  ┌────┴──────────────────────────────────────────────▼──────────────────┐  │
│  │ Ring 0 · sand-core（Rust · 纯库，无 I/O）                              │  │
│  │   step(state, inputs) 固定管线：                                       │  │
│  │   实体/法术 → 刚体+盖章 → 网格四相 → 粒子 → 场 → 对账 → 事件/哈希          │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────────┘
        ▲ UDP / matchbox ▼                        sand-harness（CLI，无 Godot）
      对端 / relay（期权）                      SyncTest · golden replay · bench
```

三条铁律先立在图旁：

1. **Godot → 核心的写入路径只有一条**：InputFrame。其余一切从 Godot 流向内环的箭头都不存在。
2. **Ring 0 不知道外界存在**：不依赖 gdext、网络、文件系统与墙钟；它唯一的世界就是（状态，输入）。
3. **表现层的本地预反馈直接消费原始输入**（瞄准线、起手动画），完全绕过核心——这条旁路只产生画面与声音，永不产生状态，因此不构成对铁律 1 的违反。

## 2. 工程布局与依赖方向

```
workspace/
  crates/
    sand-core/       Ring 0：确定性核心（纯库）
    sand-session/    Ring 1：握手、GGRS、回放、传输抽象
    sand-bridge/     Ring 2：gdext cdylib，Godot 唯一入口
    sand-harness/    工具：无头 CLI（SyncTest / replay / bench）
    sand-relay/      期权：VPS UDP 转发器（后期，独立部署）
  data/              材料表·反应表·法术表·地图（RON）
  godot/             Godot 项目：场景、GDScript、shader、音频资产
```

依赖方向单向向内：`bridge → session → core`，`harness → session → core`。反向依赖一律禁止。执法手段写进 CI：`sand-core` 的依赖清单里出现 godot / gdext / 网络库 / 逻辑路径上的 `std::time`，构建即失败。

`sand-core` 必须能脱离 Godot 完整运行——这不是洁癖，是 SyncTest、CI 回归与未来模糊测试的前提条件。

## 3. 子系统清单

### Ring 0 · sand-core（语言：Rust，纯库）

| 模块 | 职责 | 读 | 写 |
|---|---|---|---|
| state | 状态本体：网格（chunk 化）、粒子 SoA、物理世界、实体表、tick 计数。快照/恢复、per-chunk 序列化与哈希树 | — | 状态内存的唯一属主 |
| scheduler | `step(state, inputs)`：按 §4 规范管线驱动各模块，管理 rayon 有界线程池 | 确认输入 | 调用序即时序契约 |
| grid（Layer G） | 四相棋盘 push、材料规则（含气体上浮/扩散）、反应表结算、燃烧与破坏 | cells、材料/反应表、hash-RNG | cells、脏矩形、粒子生成队列、Channel B 事件 |
| particles（Layer P） | 弹道积分（并行）+ 落格提交（串行按 id）、法术弹体 payload | cells（DDA 碰撞查询）、生成队列、hash-RNG | cells（落格/爆炸写入）、物理冲量队列、Channel B 事件 |
| stamp | 像素刚体管线：位图↔网格盖章、浮力采样、破坏对账、marching squares → 简化 → 三角化重提取（滞回+每 tick 限额） | cells、body 变换、body 位图 | cells（盖章/反盖章）、物理力队列、collider 重建请求 |
| physics-adapter | 物理引擎适配层（trait 隔离 Box2D v3 / Rapier 待决项）：固定 dt 步进、按调用序建删 body、只读查询、（rollback 期）快照/恢复 | 冲量与力队列 | body 变换、接触事件（定序） |
| entities & spells | 玩家运动学控制器（自研定点，采样网格碰撞，不用物理引擎 body）、法术实例与状态效果、loadout 实例化 | 确认输入、cells、法术表 | 玩家状态、粒子生成队列、物理冲量队列、Channel B 事件 |
| rng | `hash(tick, x, y, salt, stream)` 纯函数族 | — | —（无状态，人人可读） |
| events | Channel B 缓冲：事件带 (tick, 序号) 唯一 id，供桥侧去重（为 rollback 重放预留） | 各模块投递 | 每 tick 封帧，只出不回 |

### Ring 1 · sand-session（语言：Rust）

| 模块 | 职责 | 读 | 写 |
|---|---|---|---|
| handshake | 房间码、构建/数据表指纹比对、地图与种子协商、loadout 交换 → 产出 MatchConfig（初始状态配方） | 本地配置、对端消息 | MatchConfig |
| lockstep | GGRS 会话封装：输入延迟、确认帧推进、SyncTestSession 复用同一接口 | 本地 InputFrame、对端输入、core 状态哈希 | 确认输入序列 → core |
| desync-watch | 周期全局哈希比对 + chunk 哈希树定位分叉；触发即终止对局并双端落盘 replay | 哈希流 | 终止信号、诊断包 |
| replay | 录制（指纹 + 输入流 + 周期哈希）与重放驱动；golden replay 的 CI 载体 | 输入序列 | replay 文件 |
| transport | 传输抽象：UDP 直连 / matchbox / （期权）Steam GNS | socket | socket |

### Ring 2 · 桥与表现

| 模块 | 语言 | 职责 | 读 | 写 |
|---|---|---|---|---|
| bridge-pump | Rust (gdext) | 固定 tick 累加泵（挂 Godot 主循环）、会话驱动 | Godot 时钟（仅节拍，不入状态） | core 步进指令 |
| bridge-view | Rust (gdext) | Channel A 消费：脏块像素 → 纹理上传；粒子/实体渲染表 → MultiMesh；调试视图 | core 只读视图（逐帧现取，禁跨 tick 缓存） | Godot 渲染资源 |
| bridge-input | Rust (gdext) | 原始输入 → InputFrame（位打包按键 + BAM 定点瞄准角，约 8 字节） | Godot 输入 | session 本地输入 |
| world-render | GLSL + 场景 | 调色板 shader、屏幕特效 | 网格纹理 | 帧缓冲 |
| ui-flow | GDScript | 菜单、大厅、HUD、结算 | Channel A 数值视图、session 状态 | 界面；向 session 发起握手指令 |
| audio & vfx | GDScript | Channel B 事件驱动的音效、粒子特效、震屏 | Channel B（按事件 id 去重） | 音画 |
| debris | GDScript | 装饰碎屑：Godot RigidBody2D，事件触发，两端各玩各的 | Channel B | 仅表现 |
| debug-overlay | GDScript | 脏矩形、chunk 哈希、tick 耗时分解可视化 | Channel A 调试视图 | 仅表现 |

### 工具面

| 模块 | 语言 | 职责 |
|---|---|---|
| harness-synctest | Rust | 双实例逐帧哈希比对（常驻测试入口） |
| harness-replay | Rust | golden replay 回归：重放输入流，断言终态哈希 |
| harness-bench | Rust | 最坏情况性能剖析（全屏爆炸场景），回填总纲 §7 |
| data 资产 | RON | 材料/反应/法术/地图；开发期热重载仅限单机模式，对局中永不重载；内容哈希入指纹 |

## 4. 规范 tick 管线（时序即契约）

`step()` 内部的执行顺序是确定性协议的一部分：**改顺序 = 改协议版本**，必须过决策日志。每一步读到的是哪个版本的数据，写死如下：

1. **输入应用**——确认 InputFrame → 玩家意图。
2. **实体与法术**（按实体 id 序）——玩家运动学（采样本 tick 起始网格）、施法结算：弹体入粒子生成队列、冲量入物理队列。
3. **刚体**——反盖章上一 tick 像素 → 浮力/阻力按上一 tick 淹没重叠采样入力队列 → 物理步进（固定 dt）→ 按新变换重新盖章。
4. **网格四相 pass**——材料运动、反应表结算、破坏与点燃；期间产生的溅射入粒子生成队列。
5. **粒子层**——生成队列按入队序赋 id → 全体并行积分 + DDA → 串行按 id 提交落格（同格冲突 id 小者胜）。
6. ~~**场层**~~——**已删除（2026-08-31，`kernel-charter.md` §11 翻案记录第 6 条）**。原文："读本 tick 网格源项 + F_prev，写 F_next"。气体改由第 4 步的网格四相 pass 承担，温度改为材质静态常量 + 反应表。**本步编号刻意保留不重排**：重排会使"粒子相 = 第 5 步"等既有决策日志引用失效，而编号本身属于协议表述。
7. **刚体对账**——盖章像素被删者 → body 位图更新 → 滞回判定后将重提取任务入队（每 tick 限额，超限顺延，队列本身入状态）。
8. **封帧**——Channel B 事件定序封帧；周期性计算状态哈希交给 session。

## 5. 跨层通信规则（白名单）

- 核心内部模块间通信只允许四种介质：**网格 cell、粒子生成队列、物理力/冲量队列、Channel B 事件缓冲**。禁止模块间私设旁路状态。
- **stamp 是全系统唯一允许同时读写 grid 与 physics 的模块**。其他任何模块最多持有其一。
- Channel A 是借用语义的只读视图：桥侧逐帧现取现用，禁止缓存跨 tick 引用。
- Channel B 只出不回：核心永远不读事件缓冲。
- GDScript 一行游戏逻辑都不许写——它只消费视图与事件、只发起界面级指令（开始匹配、退出对局）。
- 线程数是本地自由参数：四相设计保证任意调度同结果，两端核数不同不影响确定性。线程数只影响快慢，不影响对错。

## 6. 生命周期

启动 → 加载 data/ 并计算内容指纹 → 主菜单（GDScript）→ 握手（房间码 → 指纹校验 → 地图/种子/loadout 协商 → MatchConfig）→ `core::init(MatchConfig)` 构造初始状态 → lockstep 循环（§4 管线每 tick 一转）→ 对局结束 → replay 落盘 → 回到菜单。

desync 分支：哈希比对失败 → session 终止对局 → 双端 replay 落盘 → UI 提示 → 诊断包留存。

## 7. 语言与格式分配总表

| 语言/格式 | 用在哪 | 铁律 |
|---|---|---|
| Rust | core、session、bridge、harness、relay | 一切游戏逻辑与确定性代码的唯一语言 |
| GDScript | UI 流程、音频/VFX 分发、装饰碎屑、调试面板 | 胶水语言，禁写逻辑；不引入 C#/.NET 运行时 |
| GLSL (Godot) | 调色板 world shader、屏幕特效 | 只作用于像素，永不回读入逻辑 |
| RON | 材料/反应/法术/地图数据 | 数据即确定性输入，内容哈希入握手指纹 |

## 8. 架构级决策记录

**已决（随本文生效）**

- 玩家不是物理引擎 body，而是自研定点运动学控制器直接采样网格——手感可调、确定性零风险、与法术位移类效果同一套运动学。
- G↔F 耦合定为一 tick 延迟的双单向边，换取 tick 内无环的干净分层。
- tick 泵挂 Godot 主循环（累加器模式），核心内部用 rayon 有界并行；若实测帧间抖动不可接受，再升级为专用模拟线程 + 三缓冲交接（预留决策点，非现在做）。
- 物理引擎藏在 adapter trait 之后，Box2D v3 / Rapier 的生死判定（总纲 M6）不扩散到任何其他模块。
- 数据格式选 RON（serde 原生、Rust 侧零摩擦）；若日后需要非程序员编辑法术表，再评估上层编辑器，不换底格式。

**待决（附时点）**

- Channel A 粒子渲染表的形态（bridge 组 MultiMesh vs 暴露原始数组给 GDScript）：M1 渲染压测后定。
- 调试面板的开关粒度与发布版裁剪策略：M5 前定。
- relay 服务器的部署与协议细节：随发行通道决策，不阻塞内核。