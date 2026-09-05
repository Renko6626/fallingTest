# 手感旋钮总表

> 文档路径：`docs/tuning-knobs.md`
> 运行时版本：Rust（内核）+ Godot 4 + gdext（表现层）
> 最近更新：2026-09-06 (UTC+8)
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
| `BLAST_BODY_FACTOR` | 0.25 | 爆炸推刚体：每个半径内像素的冲量 = 此系数 × `REF_BLAST_DENSITY` × `EXPLODE_SPEED` × (1 − d/r)，方向爆心→像素；整箱在爆心附近 Δv ≈ 0.25 × 8 格/tick × 40/ρ | A（手感）：Noita `physics_explosion_power` 的对应物；1.0 时木箱会像粒子一样以 8 格/tick 飞出场地 |
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

## 8. M4 生物与法术旋钮（`crates/sand-core/src/creature.rs`、`projectile.rs`、`spell.rs`、`data/creatures.ron`、`data/spells.ron`）

### 8.1 核心常量

| 常量 | 现值 | 管什么 | 类别 |
|---|---|---|---|
| `CREATURE_MAX_STEP` | 8 格 | 生物逐轴扫掠碰撞单 tick 最多检查/跨越的整格边界数（`creature.rs::sweep_axis`） | **B**：与 `MAX_SPEED`/DDA 步数上界同一纪律——生物单 tick 速度理论上不该超过这个格数，调大只是放宽安全网，调小会让高速生物"穿墙"（扫掠提前截断） |
| `MAX_CREATURES` | 16 | 生物池上限（超限 `SpawnCreature` 确定性拒绝，粒子池同口径） | A（容量）：改了要同步 `spell.rs::SPRAY_OP_IDX_BASE` 的推导（`u16::MAX + 1 - MAX_CREATURES`），否则 Spray 的 RNG 盐值区间会跟着挪 |
| `MAX_PROJECTILES` | 4096 | 弹体池上限（超限 `Projectiles::spawn` 确定性拒绝、不排队） | A（容量） |
| `MAX_BOUNCE_RESTARTS` | 4（`projectile.rs`） | 弹跳（`bounces`）单 tick 最多重开几次 `dda::CellWalk`——安全网，不是常规路径（`bounces` 字段全部 ≤ 2） | **B**：太小会让高 `bounces` 法术在角落里提前"罢弹"（还有预算却不弹了）；太大是纯粹的最坏情形计算量上界，正常游戏数值下不可观测 |

### 8.2 `data/creatures.ron`（生物模板，改动会改 `creatures_fp`，六份 golden 需重录）

| 字段 | 现值（`player`） | 管什么 | 类别 |
|---|---|---|---|
| `half_w` / `half_h` | 2 / 5 | AABB 半宽半高（格）：碰撞体、排开扫描范围、命中判定框全用它 | A：改了连带影响出生点是否卡进地形、`muzzle_offset` 是否够用 |
| `hp_max` / `mana_max` | 100.0 / 100.0 | 血量/蓝量上限（加载期 `quantize_milli` 量化成千分位整数） | A |
| `mana_regen` | 20.0（点/秒） | 被动回蓝速率，加载期折成 `mana_regen_per_tick`（`quantize_milli_per_tick`，×1000/60 round） | A：与法术 `mana` 成本联动，决定"打完一发要等多久才能再打" |
| `run_speed` / `jump_speed` | 0.67 / 2.9（格/tick） | 地面移动速度、起跳竖直速度 | A（手感） |
| `accel_ground` / `accel_air` | 0.05 / 0.005（格/tick²） | 地面/空中加速度（空中远小于地面，符合"空中操控受限"的直觉） | A（手感） |
| `climb_over_y` | 3 格 | 扫掠碰撞里"跨台阶"的最大高度差 | A：太大会把矮墙当台阶跨过去，太小连正常台阶都上不去 |
| `swim_buoyancy_idle`/`_up`/`_down` | 1.2 / 1.4 / 0.7 | 三档浮力系数（不按键/按 `BTN_JUMP`/按 `BTN_DOWN`），净竖直加速度 = 本 tick 重力 − `GRAVITY × coeff` | A（手感）：**`_up` 必须 > `_idle`**（M4 Task 5 评审 Important 已裁决，不要"修正"回 Noita 原值 0.9——见 `creature.rs::CreatureTpl::swim_buoyancy_up` 文档，我们没有 Noita 那份独立喷射推力） |
| `swim_drag` | 0.95 | 游泳时速度收敛系数，把浮力这个无界累积量收敛到有限终速度 | A：越小终速度越低（越"粘稠"） |
| `damage_from` | `[("fire", 3.0)]`（点/秒，折算每 tick 千分位） | 材质接触伤害表，按材质 id 升序定序遍历 | A/数据驱动：材质名未知即加载报错；**当前只留 `fire` 一项**（`lava`/`acid` 缺口是范围裁剪不是缺陷，见文件头注） |
| `min_cell_count` | 4 | AABB 内某接触材质格数达到此阈值才计伤害（防止蹭到一格火就扣血） | A |
| `max_displace_per_tick` | 24 | 生物排开液体/粉末单 tick 最多处理几格（超限不排开、不排队，确定性拒绝） | A（性能/手感） |
| `muzzle_offset` | 3 格 | 施法出生点沿瞄准方向偏移量（起步 = `half_w + 1`） | A：太小会在自己身体里出生（第一帧自撞），太大出枪口手感怪 |

### 8.3 `data/spells.ron`（法术表，改动会改 `spells_fp`，六份 golden 需重录）

三个 `kind`（`spark_bolt`/`digger`/`expensive_bolt` 是 `Bolt`，`bomb` 是 `Blast`，`oil_spray` 是 `Spray`）共享下面这组顶层字段；`kind` 自身的字段（`damage`/`knockback`、`power`/`radius`/`max_durability`、`material`/`count`/`speed`/`jitter`）不在此表——那是"打出去是什么"，这张表是"打出去怎么飞、怎么撞"。

| 字段 | 现值举例 | 管什么 | 类别 |
|---|---|---|---|
| `mana` / `cooldown` | 8.0–90.0 / 6–90 tick | 施法双闸门（spec §6.1）：任一不满足即不出、无副作用 | A（数值平衡） |
| `speed` | 5.0–10.0（格/tick） | 出射初速大小；出生点沿瞄准方向 ×`speed` | A（手感） |
| `life` | 90–180 tick | 出生寿命，每 tick 未命中递减，归零销毁（或先炸，见 `on_lifetime_out_explode`） | A：定得太短会让弹体"莫名其妙消失"，太长则占满弹体池 |
| `gravity` | 0.0（`spark_bolt`/`digger`/`expensive_bolt`）/ 0.25（`bomb`） | 每 tick 施加的竖直速度增量 | A：直射弹恒 0（不受重力影响，手感是"激光"），抛射弹非零（手感是"炮弹"）——**弹跳精度测试对这个值敏感**，见下方"弹跳" |
| `spread_deg` | 0.0–2.0（BAM 量化，加载期校验 0..=180） | 出射散布半幅，`> 0` 才掷 `STREAM_SPREAD` 骰 | A（手感）：`0` 是精确瞄准（如 `digger`），非零是霰弹式散布 |
| `grace` | 0–20 tick | 防自伤宽限：`owner` 在此窗口内跳过自身命中判定 | A：太短会出生瞬间自伤（尤其近战法术），太长会让"贴脸打自己"这个操作被过度容忍 |
| `dig_power` | 0（普通弹）/ 900（`digger`，生产值） | 侵彻能量预算（Noita `ground_penetration_*`）：`= 0` 撞硬格立即终结（普通弹），越大能打穿的材质总 hp 越多 | A（手感）：**生产值与测试值刻意不同**——`common::test_spell_table` 的 `digger` 只给 90（见该文件头注：生产值配 `stone.hp=6` 够打穿 150 格，测试石块只有 41 列宽，会让"不得挖穿"这句断言落空） |
| `max_durability` | 10–12 | 该弹自身的侵彻门槛：`目标 durability > 此值` ⇒ 门槛免疫，直接终结（不侵彻）——与 `SpellKind::Blast::max_durability`（爆炸自身的破坏门槛）是两个独立字段 | A：`digger` 取 12（> `stone` 的 8，能钻；< `wall` 的 15，钻不动） |
| `air_friction` | 0.9（`slow_bolt`）/ 1.0（其余） | 每 tick 速度衰减乘子（Noita `air_friction`），`(vx,vy) *= air_friction`，`gravity` 之后立即生效 | A（手感）：`< 1` 才有衰减，`= 1` 是中性缺省（不衰减） |
| `liquid_drag` | 0.7（`wet_bolt`）/ 0.8–0.9（其余非 1） | 若**本 tick 起点格**是 Liquid，再叠加一次的速度衰减乘子——只采样起点，不沿途逐格重采（`projectile.rs::advance` 文档"液体阻力采样口径"） | A（手感）：想让某法术"入水必停"就配合 `pass_through` 不含 `liquid`（见下） |
| `pass_through` | `["gas"]`（多数）/ `["gas","liquid"]`（`digger`/`wet_bolt`） | 穿透掩码（`Category::bit()` 位或）：命中格材质的 `Category` 在掩码内就直接穿过，不算命中、也不排开 | A/**语义红线**：`projectile.rs::blocks_projectile` 文档——弹体对 Liquid/Gas **默认挡路**（不是 `material::is_solid` 那种天生豁免），不给 `"gas"` 会被烟雾/火焰当墙撞停，`data/spells.ron` 里每条法术因此都显式给了它 |
| `displace_liquid` | `true`（`bomb`）/ `false`（其余） | 命中 Liquid/Powder 格（且未被 `pass_through` 豁免）时是否推开成粒子——**`pass_through` 优先于本字段**，穿过去就不推开 | A（手感）：`false` 时未 `pass_through` 的液体格会被当硬格处理（撞停/侵彻），不是"什么都不做地飞过去" |
| `bounces` / `bounce_energy` | 2 / 0.4（`bomb`，其余为 0） | 剩余弹跳次数与每次反弹后速度保留比例（对应轴速度取反 × 此值，法线取自 DDA 撞击轴，纯整数） | A（手感）：`bounce_energy` 精度测试对 `gravity` 敏感——`gravity × bounce_energy` 必须显著小于测试容差（`1/16`），否则"反弹前速度多算一 tick 重力"的系统性偏差会把断言顶穿（`common::spell_table` 头注有完整推导，生产值 `gravity=0.25` 配 `bounce_energy=0.4` 时 `0.25×0.4=0.1 > 1/16`，仅用于"弹几次就死"这类不看精确数值的断言） |
| `physics_impulse` | 0.3（`expensive_bolt`）/ 0（其余） | 命中刚体盖章格时的单点冲量系数（Noita `physics_impulse_coeff`：`Impulse = coeff × velocity`），不做半径加权、直接施于命中像素 | A（手感）：**量级敏感**——`body.rs::apply_projectile_impulse` 文档的教训：这不是"看着像 Noita 配置"就能照抄的数字，20.0 这种量级会在小刚体（十几像素）上把 `Δv = J/mass` 冲出合理范围，一两 tick 内推穿世界边界、被墙弹回来，观测到的位移方向反而是错的；调大前先跑 `projectile_pushes_a_rigid_body_it_hits` |
| `on_lifetime_out_explode` | `true`（`bomb`）/ `false`（其余） | 寿命耗尽（`life` 归零、且此 tick 未命中任何东西）时是否补一次 `Blast` 结算 | A：只对 `Blast` kind 有实际效果（`Bolt` 的 `cid=None` 分支天然 no-op），`Bolt`/`Spray` 法术留 `false` |

