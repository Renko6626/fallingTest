# 手感旋钮总表

> 文档路径：`docs/tuning-knobs.md`
> 运行时版本：Rust（内核）+ Godot 4 + gdext（表现层）
> 最近更新：2026-09-03 (UTC+8)
> **Status**: Implemented（随内核同步维护）

**这份文档的用途**：把散在 `sand-core` 各模块与 `data/materials.ron` 里的**可调参数**
集中到一处，便于统一调和。每个 Task 的设计论证仍在各自的 spec / proposal 里，这里只回答
三个问题——**它现在是多少、拧它会怎样、拧它要付什么代价**。

**这不是真源。** 数值真源永远是代码与 RON；本表若与代码冲突，以代码为准，并**顺手改这里**。

---

## 0. 先看这个：三类参数，代价完全不同

| 类别 | 例子 | 拧它的代价 |
|---|---|---|
| **A. 纯手感旋钮** | `splash_chance`、`SPLASH_RESTITUTION`、`EXPLODE_SPEED` | 改完必须重录 golden + 跑 SyncTest。**改 RON 还会改 `materials_fp`**（P5 握手指纹），两端必须同版本 |
| **B. 带编译期契约的数值** | `V_MAX_CELL`、`DISPERSION_MAX`、`VEL_ONE` | 同上，**外加**：越界会让 `window.rs` 的 `r ≤ HALO` 断言编译不过。这是有意的——它们是 P4 写域论证的输入 |
| **C. 根本不是旋钮** | 位段布局、RNG stream 编号、tick 管线顺序、行扫描定向 | 改动即改协议。必须先过总纲 §11 决策日志，见 §5 |

**任何一类改完都要跑**：`cargo test --workspace` + `clippy --all-targets` +
SyncTest 六配置 + golden 重录（预期先落后验）+ bench 对照。这不是仪式，
是总纲 §2 的确定性法典要求的。

---

## 1. 材料表字段（`data/materials.ron`）

改这个文件会改 `materials_fp`（内容哈希入握手指纹），**全部 golden 的 `materials_fp` 行必须重录**。

| 字段 | 类型 / 域 | 现值 | 管什么 | 注意 |
|---|---|---|---|---|
| `density` | u16 | air 0 / wall 100 / sand 40 / water 16 / oil 12 / wood 12 / stone 40 / fire 1 / smoke 2 | 密度置换判定（重的沉、轻的浮；气体反向：上浮要目标更重）；爆炸冲量按 `REF_BLAST_DENSITY/density` 缩放；**Static 材质的 density 自 M3 起 = 作为刚体的密度**（浮沉：< 水 16 浮） | 改沙的密度会同时改爆炸手感；改 wood/stone 会改箱子浮沉 |
| `hp` | u32 | air 0 / water 1 / sand 2 / wood 3 / stone 6 / wall 100 | 爆炸射线逐格能量消耗（原 `blast_cost`，M2 改名） | 能量池一侧；wall 靠 `durability` 免疫而非无限 hp |
| `durability` | u8 | wall 15 / stone 8 / 其余 0 | 破坏门槛：`> Op::Explode.max_durability`（RON 缺省 10）即完全免疫 | Noita 双层破坏；哨兵 `BLAST_COST_INFINITE` 已退役 |
| `tags` | `[String]` | oil/wood `["burnable"]` | 反应表 tag 匹配（加载期展开） | 只在 `reactions.ron` 里被引用 |
| `ignition_temp` | u8 | oil 40 / wood 80 / 缺省 100 | 着火点：`源.fire_temp ≥ 目标.ignition_temp` 才点得着 | 缺省 100 = 缺省火温 10 点不着 |
| `fire_temp` | u8 | fire 100 / oil 100 / wood 100 / 缺省 10 | 作为燃烧源的火温 | **燃料也要声明**，否则油面横向过火靠不了升离表面的火（M2 §5.3.1） |
| `fire_hp` | u8 | oil 90 / wood 250 | 燃料池（点燃时装填 counter，每 tick −1） | 8 位上限 255 tick ≈ 4.25 s/格；慢烧走分频不扩位 |
| `lifetime` | u8 | fire 40 / smoke 200 | 寿命（出生装填） | 与 `fire_hp` 互斥（加载期报错） |
| `decay_to` | 材质名 | fire→smoke，其余→air | counter 归零的转化目标 | 引用不存在材质加载期报错 |
| `requires_oxygen` | bool | 缺省 true | 只有邻接 air **或 Gas** 的格才推进燃烧（由外向内） | 火是气相，贴燃料的火不算闷熄 |
| `extinguisher` | bool | water true | 燃烧格邻接即清零 counter | 走数据字段不走反应表（燃烧是 cell 状态） |
| `fire_chance` | f64 → u8 ×255 | oil 0.6 / wood 0.3 | 燃烧格每 tick 向邻接 air 产火的概率 | 需同时声明 `flame_to` |
| `flame_to` | 材质名 | oil/wood → fire | 产火产物 | `fire_chance > 0` 必须声明 |
| `rise_chance` | f64 → u8 ×255 | fire 0.5 / 缺省 1.0 | Gas 每 tick 尝试上浮的概率 | 越低火焰越"黏"燃料；smoke 恒升 |
| `body_passable` | bool | 缺省 false | 为真的材质不进刚体地形硬格掩码（Noita `liquid_sand_never_box2d`） | 沙缺省托得住箱子（M3 B′） |
| `debris_to` | 材质名 | wood → wood_debris / stone → stone_debris / 缺省自身 | 爆炸摧毁或刚体碎片脱格时粒子取此材质 | Static 材质务必指向 Powder，否则碎屑落地成悬空静态格、还会卡住刚体 |
| `vaporize_threshold` | f64 `0.0..=1.0` → u8 ×255 | sand 0.95 / water 0.4 | 爆炸近心汽化比例：剩余能量比**严格超过**即删除、不溅射 | 缺省 1.0 = 永不汽化。sand 经三轮目检 0.7→0.9→0.95 |
| `dispersion` | u8 `1..=8` | water 5，其余缺省 1 | 液体单 tick 横移格数（"最远可达空格"） | **B 类**：直接等于写入半径，越界破坏 P4。两道防线：加载期报错 + `rules::side` 用 `DISPERSION_MAX` clamp |
| `splash_chance` | f64 `0.0..=1.0` → u8 ×255 | water 0.6 / sand 0.1 | 撞停时脱格成粒子的概率 | 缺省 0.0 = 永不溅射。粉末也吃这条 |

**只对某些 Category 有意义**：`dispersion` 只影响 `Category::Liquid`（粉末不走 `side()`）；
`vaporize_threshold` / `blast_cost` 只在爆炸路径生效；Static 材质基本不进 `eval`。

---

## 2. Layer G 运动常量（`crates/sand-core/src/cell.rs`）

| 常量 | 现值 | 含义 | 拧它 |
|---|---|---|---|
| `VEL_ONE` | 4 | 1.0 格/tick = 几个位段单位（Q3.2 的定标） | **必须是 2 的幂**（编译期断言）——`substeps` 的无偏取模依赖它 |
| `V_MAX_CELL` | 16（= 4.0 格/tick） | 网格内终端速度 | **B 类**：提到 8 格/tick 会撑爆 `r ≤ HALO`，编译不过 |
| `G_ACCEL` | 1（= 0.25 格/tick²） | 每 tick 重力增量 ⇒ 16 tick 达终端速度 | 调它 = 调整个世界的"下落节奏"。**`zero-gravity` feature 把它压成 0**，仅供取证，绝不可进产品构建 |
| `SPLASH_MIN_SPEED` | 8（= 2.0 格/tick） | 撞击溅射的最低速度 | 调低 ⇒ 水花更容易冒；低到 0 会让每粒静置沙落地都炸水花 |
| `VEL_SHIFT` / `VEL_BITS` | 17 / 5 | 速度位段位置与宽度 | **C 类**，见 §5 |

---

## 3. 溅射常量（`crates/sand-core/src/rules.rs`）

| 常量 | 现值 | 含义 | 拧它 |
|---|---|---|---|
| `SPLASH_RESTITUTION` | 0.5 | 脱格粒子竖直初速 = `−v1 × 本值` | 这是**阻尼的来源**：0.5 ⇒ 弹跳两三次收敛。调到接近 1.0 会让水面持续沸腾 |
| `SPLASH_JITTER` | 0.5 格/tick | 溅射速度逐轴抖动幅度 | 必须 **< 阈值速度 × RESTITUTION**（= 1.0），否则部分粒子会向下钻。有单测钉死 |
| `SPLASH_CHANCE_DEN` | 255 | 概率骰量化分母 | 与 harness 的 `×255 round` 同口径，别单独改一边 |

`MAX_SPLASH_PER_CHUNK`（64，在 `window.rs`）：每 chunk 每 tick 的脱格上限。
超限即不脱格（**不排队**——排队要跨 tick 状态，会把限流变成状态机）。
640×384 = 60 chunk ⇒ 最坏 3840 粒子/tick。

---

## 4. 粒子层与爆炸常量

**粒子层（`particle.rs`）**

| 常量 | 现值 | 含义 |
|---|---|---|
| `GRAVITY` | 0.25 格/tick² | 粒子重力。**与 `G_ACCEL` 是两套数**——前者 Fx、后者位段单位，数值上恰好相同但没有强制绑定 |
| `MAX_SPEED` | 16.0 格/tick | 粒子逐轴速度 clamp。是 DDA 步数上界的**数值纪律**，不随手感调 |
| `MAX_PARTICLES` | 65536 | 全局容量，满则确定性拒绝。拒绝路径有**已知质量守恒缺口**（脱格已置 air） |

**爆炸（`explode.rs`）**

| 常量 | 现值 | 含义 |
|---|---|---|
| `EXPLODE_SPEED` | 8.0 格/tick | 出射速度上限（2026-08-30 目检从 16 降到 8，"粒子更重"）。与 `MAX_SPEED` 解耦：前者管手感，后者管数值纪律 |
| `EXPLODE_JITTER` | 0.5 格/tick | 溅射速度抖动 |
| `REF_BLAST_DENSITY` | 40（= 沙的密度） | 冲量→速度按 `本值/density` 缩放 ⇒ 沙系数恒 1（手感锚点），水系数 2.5 |
| `EXPLODE_FLUCT_DIV` | 4 | 射线涨落幅度 = `v/4`（能量 ±25%，射程 −25% 单边） |

**发射器（`emit.rs`）**：`MAX_EMIT_JITTER_RAW`（`2^30−1`）是防定点重缩放溢出的上界，
不是手感旋钮。

---

## 5. 明确「不是旋钮」的东西

改这些**必须先改总纲并在 §11 决策日志留痕**，不许当作调参顺手动：

| 项 | 位置 | 为什么 |
|---|---|---|
| `Cell` 位段布局 | `cell.rs` 头部表格 | 一次性定死，避免每加字段抢一次位。22 位是 `free_falling` 预留，23–31 留白 |
| RNG stream 编号 | `rng.rs` 的 `STREAM_*`（0–5 已用） | 新调用点**追加**，禁复用。复用即同帧同格同值偏置（§11 翻案 4） |
| tick 管线阶段顺序 | 架构 §4 | 时序即协议 |
| 行扫描定向 | `rules::update_chunk` | 必须是 `(tick, y)` 的纯函数。禁读活矩形/脏状态/chunk 索引/线程上下文——O1 三模式逐位等价的承重条件 |
| `HALO` = 16 | `window.rs` | P4 写域互斥的几何前提 |
| `CHUNK` = 64 | `chunk.rs` | 四相棋盘与窗口几何全绑在它上面 |
| squirrel5 的 N/P 常量 | `rng.rs` | 与 Python 原型交叉锚定的金值 |

---

## 6. 待裁决 / 已知缺口

| # | 项 | 状态 |
|---|---|---|
| 1 | `MovedSide` 撞停是否也触发溅射 | **现为「是」**（spec §6.1①）。副作用：高速水贴地横流会冒向上的水花。若目检认为过量，改成"仅 `Blocked` 触发"是一行判别 |
| 2 | 横向撞击动量被丢弃 | 速度位段是**无符号竖直**速度，网格无水平速度场。补它要再开位段，留 M2 之后 |
| 3 | 四相棋盘镜像不对称残留 −0.8% | **已裁定为特性，不修**（2026-08-31）。规避 = 竞技地图镜像轴避开 64 的倍数 |
| 4 | 溅射被全局容量拒绝时质量消失 | 与 M1 爆炸路径同一已知权衡，两端行为一致故不破坏确定性。补它要在粒子层加"拒绝即回填网格"的反向通路 |
| 5 | 斜滑是否清零速度 | **已裁定：不清零**（2026-08-31，取 jason.today / Noita 默认）。反面选项随时可切，水平 r 还会从 11 降到 4 |

---

## 7. 相关文档

| 文档 | 管什么 |
|---|---|
| `overview/kernel-charter.md` §2/§4/§11 | 确定性法典、三层内核、决策日志（**改 C 类必读**） |
| `superpowers/specs/2026-08-31-layer-g-velocity-design.md` | 色散 / 速度积分 / 溅射三 Task 的完整论证与决策表 |
| `perf/2026-08-31-layer-g-task{1,2,3}-*.md` | 每次调参的性能对照口径与实测 |
| `proposals/2026-08-31-powder-scan-direction-bias.md` | 行扫描定向为什么不能是周期 2 |

## 6. M3 刚体常量（`crates/sand-core/src/body.rs`、`physics.rs`）

| 常量 | 现值 | 管什么 | 类别 |
|---|---|---|---|
| `K_DRAG` | 200.0 | 逐淹没像素阻力：线 `F = −K_DRAG × Σw × v`，角 `τ = −K_DRAG × Σw·\|r\|² × ω`（同一系数；16×12 木箱阻尼比 ≈ 0.8，全淹没时角阻尼率 = 线阻尼率） | A（手感）：越大入水越快静止；太小会来回振荡、横摇不衰减（2026-09-03 前没有角阻尼，木条自旋到 22 rad/s） |
| `SURFACE_REACH` | 5 格 | 水面线采样：从接触格沿接触行向外穿过连续液体，取边外第 2 格起最多这么多格的列（紧邻列只兜底）；读数自上而下、各列取最低 | A：太小会采到自身溅水/空腔回填的列（浮体睡不着）；太大只是多几次格读取，连通性由接触行保证、不会穿墙 |
| `SETTLED_VEL_MAX` | 2 格/tick（`vel` 原始值 8） | 液体格竖直速度低于此才算"沉降"、才能给浮力/顶面载荷 | A：0 会把入水冲击扰动的池水（≈1 格/tick）全滤掉、木条入水不减速砸池底；太大则落水流擦过也给浮力 |
| `WAKE_TOP_LOAD_CELLS` | 16 格 | 睡眠浮体顶上堆到这么多高于周围水面的水即唤醒（让它沉一沉、把水丘滑掉） | A：太小会被溅到箱顶的几滴水反复叫醒 |
| `WAKE_H_ROWS` | 2 行 | 睡眠浮体的唤醒门槛：周围 chunk 有写入时重采 `h`，与入睡时 `last_h` 相差 ≥ 此行数即唤醒 | A：1 会被池面 ±1 行常抖叫醒（睡了又醒）；太大水退时浮体挂在半空更久（阶梯式下降） |
| `SLEEP_LINEAR_THRESHOLD` / `SLEEP_ANGULAR_THRESHOLD` | 1 格/s / 0.3 rad/s | 刚体入睡阈值（带碰撞体的刚体角阈值由线阈值÷尺寸推导） | A：太大会把正在翻倒的箱子冻在半途；浮体靠分数淹没平滑 + 施力不唤醒才睡得着，睡着后靠 `WAKE_H_ROWS` 随水位醒来 |
| `MIN_BODY_PIXELS` | 12 | 小于此面积的碎片脱格成粒子 | A |
| `MAX_REEXTRACT_PER_TICK` | 2 | 每 tick 重提取限额（超限顺延） | A（性能）；队列入哈希 |
| `TERRAIN_MARGIN` | 1 chunk | 刚体 AABB 外扩多少 chunk 生成地形碰撞 | A（性能） |
| `OUT_OF_WORLD_MARGIN` | 64 格 | 整体出界超过即移除刚体 | A |
| `MAX_BODIES` | 256 | 刚体上限（超限 SpawnBody 确定性拒绝） | A |
| `GRAVITY_CELLS_PER_S2` | 900 | 引擎重力（= 网格 0.25 格/tick²） | **C**：与网格重力对齐，改它两套物理脱节 |
| `DT` | 1/60 | 引擎步长 | **C**：与 tick 同步，不可改 |

