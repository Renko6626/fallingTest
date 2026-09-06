> 文档路径：`docs/overview/program-architecture.md`
> 运行时版本：Rust（内核）+ Godot 4 + gdext（表现层）
> 最近更新：2026-09-06 (UTC+8)
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
  tools/             四环之外的开发工具（map-editor：手绘场景 → data/scenarios，改完即渲）
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
| stamp | 像素刚体管线（`sand-core::body`，2026-09-02 落地）：位图↔网格实心光栅化盖章/反盖章（counter 往返）、水面线浮力采样、破坏对账、4 连通分量重提取（滞回 + 每 tick 限额 2）；几何走行程矩形覆盖 | cells、body 变换、body 位图 | cells（盖章/反盖章、碎片脱格粒子）、物理力、collider 重建 |
| physics-adapter | 物理引擎适配层（`sand-core::physics`，**已定 Rapier2D 0.35**，2026-09-02）：固定 dt 步进、按 body id 序建删/施力/查询、serde 快照/恢复；rapier 类型不出模块 | 浮力/阻力（整数计数 × 常量）、地形矩形 | body 变换（f32，边界唯一出口） |
| entities & spells | 生物与法术（`sand-core::{creature,projectile,spell,input}`，2026-09-06 落地）：`InputFrame` 唯一入核通道；生物为自研定点 kinematic（AABB 逐轴分离扫掠、逐像素采样网格，**不用物理引擎 body**），含排开液体/游泳/材质接触伤害/HP 墓碑；弹体为独立 SoA（复用 `dda`/`fixed` 模块而非粒子池），含侵彻/弹跳/阻力/穿透/刚体点冲量；法术表扁平三原语 Bolt/Blast/Spray + cooldown·mana 双闸门。**状态效果（stain）与施法状态机顺延**（总纲 §11 第 18 条 ①） | 确认输入（InputFrame）、cells、法术表、生物模板表 | 生物与弹体状态、cells（排开/侵彻删格）、粒子生成队列、`pending_blasts`、Channel B 事件 |
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

1. **ops 应用**——按 `enumerate` 下标（op 序号）逐条 apply（含 `Op::SpawnCreature`）；M4 spec §1.1 把这一步从旧称"输入应用"改名——同一 tick 里紧接着的 2a 才是真正的"玩家输入 → 意图"，两步都叫"输入应用"会让读者分不清指的是哪一个（评审文档漂移条目，2026-09-06 订正）。
2. **实体与法术**（按实体 id 序）——**2026-09-06 由占位变生效**（总纲 §11 实施期决策第 18 条），内部展开为四个子步骤，此展开同属协议：
   - **2a 输入应用**——`InputFrame[controller]` → 生物意图（按 creature id 序）。
   - **2b 生物运动学与世界互动**——**代码实现是两趟独立的全表遍历，不是"每个生物走完整条链再轮下一个"**（评审 I2 文档漂移订正，2026-09-06）：
     - **2b-i `step_kinematics`（按 creature id 序，对全体生物）**——AABB 逐轴分离扫掠（**先 x 后 y，顺序即协议**），采样本 tick **起始**网格；本函数签名只借 `&World`（只读），不产生任何网格写入。
     - **2b-ii `step_world_interaction`（按 creature id 序，对全体生物，独立于 2b-i 的第二趟遍历）**——排开液体/粉末（脱格进粒子生成队列，写 `&mut World`）→ 游泳浮力与阻力 → 材质接触伤害 → HP 墓碑。

     两趟拆开纯粹是借用检查的产物：`step_kinematics` 只读 `&World` 即可完成运动学积分与碰撞扫掠，`step_world_interaction` 要写 `&mut World`（排开液体要把格子改成 air）——同一遍历若合并成一趟，`&World`（供扫掠读)与 `&mut World`（供排开写）会在同一次借用里冲突，Rust 借用检查器不允许对同一个 `World` 既共享借用又可变借用。拆成两趟各自完整借用一次是最直接的解法（`crates/sand-core/src/lib.rs::Sim::step` 的两行连续调用）。

     **这不是纯粹的实现细节，是可观测的多生物定序差异**：若真按"每个生物一条完整链"读——生物 0 先做完 2b-i+2b-ii（含排开水），生物 1 才开始 2b-i——生物 1 的扫掠会看到生物 0 已经排开过的水（水已经变 air/粒子，碰撞判定不一样）。但实际两趟遍历下，**全体生物的 2b-i 都用同一份本 tick 起始网格**（此刻没有任何生物排开过水，因为 2b-ii 还没开始跑），生物 1 的扫掠必然看到的是"生物 0 还没排开水之前"的水——多生物场景下两种读法给出不同结果，"时序即协议"红线要求文档描述代码的真实行为，而不是更好读但失真的简化版本。
   - **2c 弹体积分与命中**（按弹体下标序）——沿 DDA 路径**先到者优先**判定（同一格内先生物后硬格）：侵彻删格 / 弹跳 / `Bolt` 扣血击退 / `Blast` 走既有 `apply_explode` + `pending_blasts` / 刚体点冲量。读到的是本 tick **已移动后**的生物位置。
   - **2d 施法结算**（按 creature id 序）——cooldown + mana 双闸门 → 产弹体（本 tick 不积分，下 tick 起飞）或 `Spray` 走既有 `apply_emit` 入粒子生成队列。
   
   三条定序理由承重：2b 在 2c 之前（弹体命中的是本 tick 移动后的位置）；2c 在 2d 之前（新生弹体出生点在生物身上，当帧就走 DDA 会自撞）；整个第 2 步在第 3/4 步之前（生物与弹体读同一份本 tick 起始网格，弹体炸出的洞在同 tick 网格四相里就被消化）。
3. **刚体**——反盖章上一 tick 像素 → 浮力/阻力按上一 tick 淹没重叠采样入力队列 → 物理步进（固定 dt）→ 按新变换重新盖章。
4. **网格四相 pass**——材料运动、反应表结算、破坏与点燃；期间产生的溅射入粒子生成队列。
5. **粒子层**——生成队列按入队序赋 id → 全体并行积分 + DDA → 串行按 id 提交落格（同格冲突 id 小者胜）。
6. ~~**场层**~~——**已删除（2026-08-31，`kernel-charter.md` §11 翻案记录第 6 条）**。原文："读本 tick 网格源项 + F_prev，写 F_next"。气体改由第 4 步的网格四相 pass 承担，温度改为材质静态常量 + 反应表。**本步编号刻意保留不重排**：重排会使"粒子相 = 第 5 步"等既有决策日志引用失效，而编号本身属于协议表述。
7. **刚体对账**——盖章像素被删者 → body 位图更新 → 滞回判定后将重提取任务入队（每 tick 限额，超限顺延，队列本身入状态）。
8. **封帧**——Channel B 事件定序封帧；周期性计算状态哈希交给 session。

## 5. 跨层通信规则（白名单）

- 核心内部模块间通信只允许四种介质：**网格 cell、粒子生成队列、物理力/冲量队列、Channel B 事件缓冲**。禁止模块间私设旁路状态。
- **stamp（`body.rs`）是全系统唯一允许同时*读写*（发起对两者的实际读写调用）grid 与
  physics 的模块**。其他任何模块最多持有其一——但这条规则约束的是"谁能对两者发起
  读写"，不是"谁的函数签名里同时出现两个类型的引用"。一个模块若只是把 `&mut
  PhysicsWorld` 原样**转手传递**给 `body.rs` 的函数、自己不调用任何 physics 类型的
  方法，不算"持有"，仍然合规（评审 I2，2026-09-06 明确措辞）。
  实例：`Projectiles::advance`（`sand-core::projectile`，M4 spec §5.5 单点冲量）
  形参同时有 `world: &mut World` 与 `phys: &mut PhysicsWorld`，但函数体内唯一涉及
  `phys` 的语句是 `bodies.apply_projectile_impulse(phys, ..)`（`projectile.rs` 头注
  已有同一措辞）——真正同时读写 grid 与 physics 的调用发生在 `body.rs::
  apply_point_impulse` 内部，`advance` 只是转手。

  评审曾提出改造成 `pending_point_impulses` 队列（弹体命中记一条冲量请求，攒到
  第 7' 步与 `pending_blasts` 一起统一施加），**裁定不做**：①`advance` 现在在
  第 2c 步（`lib.rs::step` 管线）同步调用 `apply_point_impulse`，早于本 tick的
  `physics.step()`/`stamp_all()`（第 3 步），冲量当 tick 就体现在盖章位置上；
  `pending_blasts` 在第 7' 步——**本 tick 的** `physics.step()` 早就跑完——才统一
  施加，效果要等**下一 tick**的 `physics.step()` 才会移动、被盖章看见（这是评审
  指出的两条命中路径本就存在的计时差，早于本次改动，不在本 Task 范围内一并抹平）。
  把弹体命中也塞进第 7' 队列，会把它现在"当 tick 生效"的手感真实地推迟一 tick——
  这是对一条已测试、已验收的机制做行为改动，不是单纯的架构整理；②不管走不走队列，
  真正同时读写 grid 与 physics 的调用点始终是 `body.rs::apply_point_impulse`，
  队列只改变"何时调用"，不改变"谁在调用"——不能让 `advance` 变得更不"持有"
  physics，对 I2 本身（"谁在文档措辞里算持有"）没有任何架构收益；③新增队列字段
  会改 `entity_hash` 结构，逼一次不为任何行为诉求服务的 golden 重录。三条理由
  叠加，维持现状（直传引用）+ 修文档措辞是收益/成本比更高的选择。
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