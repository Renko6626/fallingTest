# M2 反应表与燃烧：数据驱动反应 + Noita 式燃烧 + 气体作为第四个 Category

> 文档路径：`docs/superpowers/specs/2026-08-31-m2-reactions-and-fire-design.md`
> 运行时版本：Rust（内核）+ Godot 4 + gdext（表现层）
> 最近更新：2026-08-31 (UTC+8)
> **Status**: Proposed（brainstorm 已完成并经用户逐节确认，待实施）
> 上游：`docs/overview/kernel-charter.md` §11 翻案记录第 6 条（删除 Layer F 场层）、M2 里程碑重定义
> 调研依据：`docs/reference/noita-material-schema.md`（材质字段一手核查）、`docs/reference/noita-grid-api-and-rng.md`（引擎侧笔记与 PRNG 事故史）
> 解挂复用：`docs/superpowers/specs/2026-05-26-fire-system-design.md`（fire spec v2，Noita 式主线随翻案 6 重新生效）

---

## 实施进度

| Task | 内容 | 状态 |
|---|---|---|
| 0 | 数据层：材质新字段 + `reactions.ron` + 加载期契约 | ✅ 2026-08-31（材质字段部分；`reactions.ron` 随 Task 2） |
| 1 | 气体：`Category::Gas` + `gas_step` | ✅ 2026-08-31（GIF 目检待用户） |
| 2 | 反应表：tag 展开 + 发起方约定 + 结算；`blast_cost` → `hp`+`durability` | ✅ 2026-08-31 |
| 3 | 燃烧：`counter` 位段 + 点燃/推进/衰变链 | ✅ 2026-08-31（实施补记见 §5.3.1；GIF 目检待用户） |

Task 0 是 1–3 的共同地基，不单独验收，随 Task 1 一起落地。

---

## 0. 验收标准

1. **总纲 M2 标准**：火油连锁 demo 的 replay 终态哈希一致；SyncTest 双实例 2 万 tick 零分叉。
2. **线程数不变性**：1 / 8 / 16 线程逐 tick 状态哈希逐位相同。
3. **休眠不退化**：未点燃的可燃物静置时全图入睡（执法测试 `resting_wood_lets_chunk_sleep`）。
4. **分布回归**：点燃方向骰四向均匀、反应触发率贴近声明概率（见 §7.2，这是本 spec 新立的规矩）。
5. **golden 重录取证**：无火无爆炸的场景状态哈希逐位不变（`hashrun --grid-only` diff 取证后再重录）。
6. **GIF 目检**（用户手动执行）：火油连锁、烟上升、木头由外向内烧。
7. **bench 无回退** + u64 对照成本入档 `docs/perf/`。

---

## 1. 背景与范围

### 1.1 为什么是现在

总纲翻案第 6 条（2026-08-31，用户裁决）删除了 Layer F 场层，M2 从"场层与反应表"重定义为"反应表与燃烧"。温度不再是扩散场，而是回归 Noita 体系的**材质静态常量 + 反应表**；气体不再是场，而是 **Layer G 的第四个 Category**。本 spec 是该翻案的落地设计。

### 1.2 三件事的依赖链

```
气体（反应产物与火的衰变链需要落点）
  → 反应表（材质互转的通用机制）
    → 燃烧（既要反应表结算、又要产出气体）
```

故分三个 Task 串行，照 Layer G 三 Task 的成例：逐 Task 落地、提交、目检。

### 1.3 材质清单（用户裁决 2026-08-31：最小集）

| id | 名 | Category | 角色 |
|---|---|---|---|
| 0 | air | Static | 既有 |
| 1 | wall | Static | 既有 |
| 2 | sand | Powder | 既有 |
| 3 | water | Liquid | 既有，本轮加 `extinguisher` |
| 4 | oil | Liquid | 新增，可燃、密度低于水（浮油） |
| 5 | wood | Static | 新增，可燃、`requires_oxygen` |
| 6 | fire | Gas | 新增，高 `fire_temp`、短 `lifetime`、衰变成 smoke |
| 7 | smoke | Gas | 新增，长 `lifetime`、衰变成 air |

**tag 机制不随清单缩水。** 总纲 §8 反例第一条禁止硬编码材质对，且 tag 展开是加载期一次性的事、成本接近零。反应照样按 `[burnable]` 这类 tag 写，只是本轮 `[burnable]` 只有 oil 与 wood 两个成员——机制到位、内容留白，M3/M4 加材质时不返工。

### 1.4 不做（Non-goals，全部附理由）

| 项 | 理由 |
|---|---|
| `blob_radius` / `convert_all` | 产物扩散半径是 P4 写域论证的输入。要做必须与"反应独立成 pass"和 `window.rs` 的 r 编译期断言**一起上**，见 §6.3。Noita 实测 `blob_radius` 到 40、`convert_all` 写域无界（且 vanilla 零使用）。 |
| 三元反应（`input_cell3`） | 最小集用不上；加一维会让稠密索引表退化。 |
| `direction` 方向性反应 | 我们的发起方约定解决的是双结算，与方向性是两回事（详见 §4.3）。要做需单开字段，本轮无需求。 |
| 自反应（同材质相邻） | 发起方约定 `id_a < id_b` 天然排除。本轮无需求。 |
| 词缀展开（`[meltable]_molten`） | 把材质名字符串变成逻辑输入，解析失败是**静默**的，违反我们"加载期显式报错"的纪律。用显式产物映射替代。 |
| 温度场 | 翻案 6 已删。复议条件见该条。 |
| O3 粉末惯性 / 粒子穿水弹跳 / M1 两条测试债 | 用户裁决 2026-08-31 全部推迟，与反应/燃烧无真依赖。 |
| Cell 扩 u64 | 本轮 u32 够用（counter 8 位 + 1 位留白）。只做"随时可扩"的封装 + 量一次成本，见 §2.3。 |
| 材质扩到 11+ 种 | 用户裁决最小集。 |

### 1.5 方案选型（已裁决）

三个候选整体方案，用户 2026-08-31 采纳**方案 1**：

- **方案 1（采纳）**：全部塞进现有 `Ctx::eval`，零新增 pass。写域论证零改动、tick 管线不变（不构成协议版本变更）、`stamp` 白送一层防连锁。
- 方案 2：反应独立成一个四相 pass。职责更清晰、为 `blob_radius` 留余地；代价是改 tick 管线 = 改协议版本、多一遍扫描、`stamp` 语义要拆成两份（很可能再抢一位）。**本轮不做 blob，故换不来对应好处。**
- 方案 3：纯反应表做火，不引入 per-cell 燃料池。**我们自己试过且不够用**——fire spec v2 §1 记着四条毛病：火焰生成即上浮离开燃料、没有由外向内的燃烧、概率参数难调、不支持不同材质不同燃烧速度。

---

## 2. Task 0 — 数据层

### 2.1 `materials.ron` 新增字段

全部可选、缺省安全（沿用 `dispersion` / `splash_chance` 的既有体例：不声明即退化为改动前行为）。

| 字段 | 类型 | 缺省 | 语义 |
|---|---|---|---|
| `category` | enum | — | 新增取值 `Gas` |
| `tags` | `[String]` | `[]` | 反应匹配，加载期展开为 id 集合 |
| `hp` | u32 | 1 | **原 `blast_cost` 改名**：爆炸射线逐格能量消耗 |
| `durability` | u8 | 0 | 破坏门槛：`durability > op.max_durability` ⇒ 免疫 |
| `ignition_temp` | u8 | 100 | 着火点（Noita `autoignition_temperature`，静态常量） |
| `fire_temp` | u8 | 10 | 火温（Noita `temperature_of_fire`，静态常量） |
| `fire_hp` | u8 | 0 | 燃料池初值，**0 = 不可燃**；被点燃时装填进 counter |
| `lifetime` | u8 | 0 | 寿命初值，**出生即装填**进 counter |
| `decay_to` | String | `"air"` | counter 归零后的转化目标 |
| `requires_oxygen` | bool | true | 为真：只有邻接 air 的格才推进燃烧（由外向内） |
| `extinguisher` | bool | false | 邻接即扑灭燃烧（water 为真） |

**加载期校验**：`fire_hp` 与 `lifetime` **至多声明其一**（两者共用同一个 counter 位段，语义靠"何时装填"区分，同时声明是配置错误而非可用组合）。

### 2.2 `blast_cost` → `hp` + `durability`

Noita 的破坏模型是**双层**（一手核查见 `noita-material-schema.md` §6）：`durability` 是门槛（超过即完全免疫），`hp` 是能量池（射线逐格扣减）。而"我能打穿多硬的东西"是**操作侧**参数——爆炸、激光、地形松动三个独立系统各带一个 `max_*durability*` 字段，没有一个把门槛写死在材质侧。

落地：

- `Op::Explode { x, y, r, power }` → `Op::Explode { x, y, r, power, max_durability }`。
  场景 RON 缺省 **10**（对齐 Noita `ConfigExplosion.max_durability_to_destroy` 的默认值）。
- `explode::fire_ray` 的逐格判定改为：`durability > max_durability` ⇒ **断线**（等价于现在撞 `BLAST_COST_INFINITE` 的行为）；否则按 `hp` 扣能量。
- **`BLAST_COST_INFINITE` 哨兵退役**：wall 改成 `durability: 15`（高于任何法术上限），语义比"无限能量消耗"直白，且不再依赖"power 不会超过某个界"的隐含假设。

### 2.3 Cell 位段与 u64 封装

```text
| bits  | 宽 | 字段 |
| 0–7   | 8 | material |
| 8–15  | 8 | stamp |
| 16    | 1 | dir |
| 17–21 | 5 | vy（Q3.2 无符号） |
| 22    | 1 | free_falling（O3 预留，恒 0） |
| 23    | 1 | 留白 |
| 24–31 | 8 | counter（本 spec 新增） |
```

`counter` 是**通用倒计时器**，语义随材质而定：燃料格存剩余燃料、fire/smoke 格存剩余寿命。两者都是"每 tick 减一、归零触发转化"，机制同构，不开两个位段。**不设 `burning` 标志位**：`counter > 0` 且材质可燃即为正在燃烧；未点燃的可燃物是 `counter == 0`；烧尽后立刻转材质，故"未燃"与"刚烧完"这两个 0 不会撞车。

**8 位的调参天花板（审阅补记 2026-08-31）**：每 tick 减一 ⇒ 最长燃烧/寿命
255 tick ≈ 4.25 秒/格。最小集够用；将来要"慢烧材质"，解法是**分频递减**
（每 k tick 减一，k 为材质字段），不是加宽 counter 位段——此处先记下，
免得实施期把扩位当默认解。

**u64 封装（用户裁决 2026-08-31：不扩，但做成随时可扩 + 本轮量成本）**：

1. 引入 `type CellRepr = u32;`，堵上 `Cell(pub u32)` 这个 pub 字段缺口，位段访问全部收敛到 `cell.rs` 访问器后面 —— 扩宽退化为改一行别名 + 几个掩码常量。
2. 本轮 bench 用 feature flag 量一次 u64 在 720p/1080p 全活跃下的真实成本，落 `docs/perf/`。**这与翻案 6 立的"重开场层前必须先有 bench"是同一条纪律**，不给自己破例。

### 2.4 `data/reactions.ron` 与加载期契约

```ron
(
    reactions: [
        (input: ["water", "fire"], output: ["water", "smoke"], probability: 0.80),
        // 本轮内容稀薄是预期的，见 §4.6
    ],
)
```

**四条加载期契约**：

1. **引用不存在的材质或 tag ⇒ 加载失败并报错。** Noita 的 `unknown` 是静默丢弃整条反应——那是给 mod 的容错，对我们是**双端反应表不一致 → 分叉**（P5 红线）。这一条必须与 Noita 反着抄。
2. **tag 在加载期展开成扁平表，core 侧不出现任何字符串。**
3. **发起方在加载期规范化**：每条反应只按 `id_a < id_b` 注册一次。`archive/prototype-python/core/reaction.py:44-46` 是正反双向都注册，那正是总纲警告的双结算来源，移植时必须改掉。
4. **概率 RON 写小数、加载期一次性量化成整数阈值**，沿用 `vaporize_threshold` / `splash_chance` 体例。

**指纹**：新增 `reactions_fp`，与 `materials_fp` 同等待遇入握手指纹（P5），并同样走行尾归一化（§11 实施期决策第 4 条的 CRLF 教训）。

### 2.5 core 侧查找结构

材质 id 是 `u8`，**按加载期实际材质数 `n` 开稠密二维索引表**（不是按 `u8` 的 256 域）：`Vec<u16>` 长度 `n×n`，值是结果表偏移，0 表示无反应。

- 本轮 `n=8` ⇒ 64 项 = 128 字节；`n=64` ⇒ 8KB；满编 256 才 128KB。
- 查找是一次索引载入、无分支。**这条查找在全引擎最热的循环里**（每活跃 cell 每 tick 对邻居查若干次），比稀疏方案的二分 + 分支预测失败划算。
- 顺带绕开"禁 std HashMap 默认 hasher"红线，且天然定序。

**作者格式是稀疏的**：`reactions.ron` 写的是一条条反应，人手写人手读；稠密表纯粹是加载期由稀疏条目构建的内部索引，任何人都不需要写或看一个 n×n 矩阵。

**切换判据（写死，不是"以后看着办"）**：材质数越过 **64 种**，或 bench 显示该表 cache 行为成为瓶颈，就换成"per-material 位掩码提前退出 + 稀疏结果表"。表放在 `ReactionTable::get(a, b)` 这一个访问器后面，换实现时调用方一行不动。

### 2.6 `eval` 准入与 per-material 预计算标志（**self-review 补漏**）

现有 `eval` 开头是 `if self.table.is_static(m) || c.stamp() == self.stamp { return; }`
（`crates/sand-core/src/rules.rs:150`）——**Static 材质根本不进 eval**。wood 是 Static，
若不动这一句，它永远不会被点燃、也不会推进燃烧，Task 3 在 wood 上直接失效。

**修法**：把"是否静态"从**准入条件**降为**运动分支的条件**，另立一个更准的准入判定。

```text
eval(x, y):
    if c.stamp() == stamp:            return      // 本 tick 已处理，不变
    if !needs_eval(c):                return      // 新的快速退出
    if !is_static(m):                 ...运动（含溅射脱格，可能提前返回）...
    ...反应结算（在落点坐标上）...
    ...燃烧推进 / 点燃 / 产火...
```

**`needs_eval(c)` = `!is_static(mat)` ∨ `counter > 0` ∨ `initiates_reaction(mat)`**

- `initiates_reaction` 是**加载期预计算的 per-material 布尔**：反应表里是否存在以该材质为
  发起方（`id_a`）的条目。一次数组查，与反应表规模无关。
- **点燃与灭火不需要目标进 eval**：两者都由"燃烧源"发起、由源检查邻居，未点燃的 wood
  （`counter == 0`）和 water 都无须自己被评估。
- 结果：wall 与未点燃的 wood 仍然一个分支就退出，**M0 的稀疏扫描性能不受影响**；
  只有真正在动、在烧、或会发起反应的格子付出邻居检查的成本。
- **实现提示（审阅补记 2026-08-31）**：别让早退从一次查表变三次——`is_static` /
  `initiates_reaction` 合并进单个 per-material flags 字节（一次载入），
  `counter > 0` 是位测试，保住 `rules.rs:150` 现有的退出成本。

> 这正是"反应数据是稀疏的"这个直觉真正该兑现的地方——不是在 `(a,b)` 查找表上
> （那里稠密更快，见 §2.5），而是在**每格的准入判定**上。

---

## 3. Task 1 — 气体

### 3.1 逻辑

`Category::Gas` 走 `gas_step`，与 `liquid_step` 严格镜像（powder 没有水平扩散步）：**向上 → 两个斜上 → 水平扩散**。

**扩散恒 1 格、不读 `dispersion` 字段（审阅补漏 2026-08-31）**：§3.4 与 §6.1 的
r_gas = 1 依赖这一条。加载期校验：Gas 材质声明 `dispersion` 即配置错误（体例同
§2.1 的 `fire_hp`/`lifetime` 互斥）。将来气体要大扩散，改这条校验 + §6.1 表即可。

`displace` 的密度比较**方向反转**：下沉类要求"目标更轻才让路"，上浮类要求"目标更重才让路"。实现上把比较写成沿运动方向的密度梯度，两类共用一个函数，**不复制代码路径**。

### 3.2 三件明确不做

- **不碰速度位段**：气体恒定 1 格/tick。位段是无符号竖直（向下）速度，气体本来就用不了它——这正是翻案 6 里"气体不占速度位段、Cell 位段规划不受影响"的承诺兑现处。
- **不进 `substeps` 循环**：n 恒为 1。
- **不设独立的 Fire category**：`fire` 就是一种 Gas，"火"性质全部由材质字段（高 `fire_temp`）与燃烧机制表达，少一条代码路径。

### 3.3 扫描方向与连锁

现有扫描自下而上，对下沉材质正确，但对上浮气体天然是"连锁方向"——气泡本该一帧升一格，自下而上扫会让它一帧升很多格。

**现有 stamp 机制已堵死**：`displace` 给两格都盖当前戳，`eval` 开头 `c.stamp() == self.stamp` 即跳过。气体不需要任何额外处理。需要一条回归测试钉死这个行为（§7.1）。

### 3.4 写域

移动 1 格 + 扩散 1 格 ⇒ **r_gas = 1**。见 §6.1 合并论证。

---

## 4. Task 2 — 反应表

### 4.1 结算位置与顺序

在 `Ctx::eval` 中、**运动判定之后、在落点坐标上**做（运动可能已把该 cell 挪走）。
检查 4 邻域，顺序是**编译期常量数组**（不是运行时决定的顺序），第一个命中即 `break`。

同一对材质挂多条反应时，按加载序逐条掷骰，取第一个命中；每条用不同的 `attempt`（见 §4.4），使各条概率语义相互独立。

### 4.2 发起方约定

只有 `mat(自己) < mat(邻居)` 时才发起。这防双结算，顺带砍掉一半邻居检查。

副作用：**同材质相邻永远不发起**，即自反应做不了。本轮不需要，已列入 Non-goals。

### 4.3 与 `direction` 的关系（澄清）

Noita 的 `<Reaction direction=>`（vanilla 17 条在用）表达的是**玩法上的方向性**（Alchemy 页教学"中和毒泥只能自上而下，水必须在毒泥上方"）。我们的发起方约定解决的是**双结算**，两者是不同的问题。**别指望发起方约定顺带实现方向性反应**——要做需单开字段，本轮无需求。

### 4.4 确定性纪律（硬要求）

同一格同一 tick 要对 4 个邻居各掷一次骰。**这正是总纲 §11 翻案第 4 条点名的"同帧同格多次掷骰"。**

| 流 | key | salt | attempt |
|---|---|---|---|
| `STREAM_REACT`（新增） | 发起格坐标 | **邻居方向索引 0–3** | 反应条目序号 |

`salt` 那一维不是可选优化：漏了它，同一格对四个邻居掷出同值，反应就会沿固定方向偏。外部实例见 `noita-grid-api-and-rng.md` §5.2——Noita 因同类问题（同坐标二次播种导致序列重放）使宝箱近半战利品永不出现，扛了数年才修。

### 4.5 写入语义

反应命中后：两格都写产物 + **盖当前戳**；**产物的 `counter` 重置为产物材质的初值**（`lifetime` 或 0）；**速度清零**——材质换了，动量语义不再成立。

**戳的防连锁语义要说全（审阅补漏 2026-08-31）**：盖戳防的是产物格本 tick **发起**
反应；单靠它防不住产物格**再被反应**——扫描序靠后的另一发起方查邻居时看到的已是
产物材质，可命中 (发起方, 产物) 这条新反应对，同一格一 tick 内二次转化。故定死：
**反应邻居检查跳过已盖当前戳的格**。两种语义都是确定的（不分叉），选跳过是为了
"一格一 tick 至多转化一次"，与防连锁的意图自洽。

产物严格 **1:1**（无 blob）。

### 4.6 本轮反应表内容稀薄是预期的

8 种材质下，真正需要的反应只有"水灭火"一类。**Task 2 的交付物是基础设施而非内容**：tag 展开、发起方规范化、加载期契约、稠密索引表、RNG salt 纪律、`durability` 替换。内容留给 M3/M4 加材质时按数据补——而"加一种新材质只需打 tag、不碰反应表"正是这套基础设施要证明的性质。

> 实施期可选（不改变本 spec 结论）：若目检时觉得水火循环不闭合，加一种 `steam`（`water + fire → water + steam`、steam 衰变回 water）是**纯数据改动**，不动任何代码——这本身就是数据驱动性质的现场演示。是否加由用户在 Task 3 目检时定。

### 4.7 改动面

`material.rs`（新字段与访问器）、新增 `reaction.rs`（表结构与查找）、`rules.rs`（`eval` 内插入结算）、`world.rs`（`Op::Explode` 加 `max_durability`）、`explode.rs`（双层破坏模型）、`sand-harness::scenario`（加载、校验、量化、指纹）。

---

## 5. Task 3 — 燃烧

### 5.1 统一规则：counter 归零即衰变

**`counter` 每 tick 减 1，归零即转化为材质声明的 `decay_to`。** 这一条机制表达三段链条：

```
wood（燃料耗尽）→ air        oil（燃料耗尽）→ air
fire（寿命耗尽）→ smoke      smoke（寿命耗尽）→ air
```

燃料烧尽、火熄成烟、烟散成空气，是同一个机制的三次应用。**不需要为烟单独设产出机制——烟就是火的尸体。**

**装填时机**区分两类：`lifetime`（fire/smoke）出生即装填；`fire_hp`（wood/oil）**只有被点燃时才装填**。

**递减只对 `counter > 0` 的格子发生**——`counter == 0` 即不是燃烧中、也不是有寿命的材质，
直接跳过（这条与 §5.6 的休眠红线是同一件事的两种说法）。

### 5.2 点燃判定

**燃烧源的定义（审阅补漏 2026-08-31）**：只有 **`counter > 0`** 的格才是燃烧源——
覆盖燃烧中的燃料（`fire_hp > 0` 且已装填）与 fire（lifetime 出生装填）。这道门不可省：
oil 是 Liquid、每 tick 都进 eval，若不加此门，**未点燃的冷油**只要声明了较高
`fire_temp`（燃烧的 oil 要能点燃 wood 就必须声明）就会自发点燃邻居。smoke 的
`counter` 同样 > 0，靠**低 `fire_temp` 过不了温度比较**天然免疫——这是一条数据约束：
写 materials.ron 时 smoke 的 `fire_temp` 必须低于全部材质的 `ignition_temp`。

照 Noita：燃烧源每 tick **随机选一个方向**采样邻居，若

```
源.counter > 0                                    （燃烧源门，见上）
且  源.fire_temp ≥ 目标.ignition_temp
且  目标.fire_hp > 0  且  目标.counter == 0
且  （目标.requires_oxygen ⇒ 目标邻接 air）        （氧气前置，见下）
```

则装填 `目标.counter = 目标材质.fire_hp` 并**盖当前戳**。

**末条氧气前置（审阅补漏 2026-08-31）**：不查它，燃烧表面格的方向骰指向大块 wood
内部时会装填其 counter，下一 tick 该格因无 air 邻接被 §5.4 熄灭清零——形成
装填/清零的写 ping-pong，点燃骰也浪费在注定闷熄的方向上。加这一条只在候选命中时
多 4 次邻居读，成本可忽略。

随机选一个方向而不是检查四邻：蔓延自带随机性、不会四面齐爆，成本也低。自洽锚点参照 Noita 实测（nest 的 `autoignition=85`，火温 100 的 fire 能点燃它、火温 60 的 flame 不能）。

### 5.3 产火（**self-review 补漏**）

点燃只装填燃料格的 counter，**火本身还得有人生**。燃烧中的格子（`counter > 0` 且
`fire_hp > 0`）每 tick 按概率向一个邻接 air 格写入 `fire` 材质并盖戳，
`fire.counter` 装填为 `fire` 的 `lifetime`。

方向与是否产出共用 `STREAM_IGNITE` 的"产火骰"（§5.8），与方向骰不同 `attempt`。
产出概率是材质字段（对应 Noita 的 `generates_flames`），本轮给 oil 与 wood 各一个值即可，
不新开机制。

于是完整链条闭合：**燃料被点燃 → 燃料产火 → 火衰变成烟 → 烟衰变成空气**。

### 5.3.1 实施补记（2026-08-31，Task 3 落地）

三条实施期决定，机制不变、语义微调，随 Task 4 一并入总纲 §11：

1. **产火产物走数据字段 `flame_to`**（材质名，加载期解析成 id；`fire_chance > 0`
   必须显式声明）——产物不硬编码"fire"，与 §8 禁硬编码材质语义同一条纪律。
2. **燃料材质也声明 `fire_temp`**（oil/wood 取 100）：火是气体、升离水平表面
   只要一 tick，油面横向过火**必须**靠燃烧中的燃料自身温度直接点燃同类
   （实测：只靠 fire 气体点燃，油池表面永远烧不开）。§5.2 的源门（`counter > 0`）
   恰好使这样做安全——冷燃料完全惰性。
3. **氧气判定 = 邻接 air 或任意 Gas**（§5.4 的"邻接 air"放宽）：火/烟本身是
   气相，贴着燃料的火不该把燃料"闷"住——否则火占掉油面 cell 唯一的 air 邻居，
   表面永远点不着（实测踩坑）。实心内部四邻全固/液仍然无氧，由外向内不变。

### 5.4 由外向内烧

`requires_oxygen` 为真时，**只有邻接 air 的格才推进 counter**。这正是 fire spec v2 §1 记的第二条毛病（大块木头瞬间消失）的解法。四周无 air 则熄灭——顺带把"闷熄"做了。

### 5.5 灭火走数据字段

材质加 `extinguisher: bool`，燃烧格邻接到它就清零 counter。

**不能走反应表**：反应表匹配的是**材质**，而"正在燃烧"是 **cell 状态**，表达不了。这不是偷懒，是机制边界。

### 5.6 休眠红线（正确性，不是优化）

燃烧格每 tick 都写 counter ⇒ 所在 chunk 永不入睡。这是**对的**，燃烧本来就是活动。

但**未点燃的可燃物必须零写入**：静置 wood 的 `counter == 0`，就直接什么都不做，不能"减到 0 再写个 0"。

这与 Layer G Task 2 那条写回纪律（`rules.rs` 的 `eval` 文档、`resting_pile_lets_every_chunk_sleep`）是同一条命：破了它整张图永不入睡，M0 建立的稀疏性能当场退回全量扫描。执法测试 `resting_wood_lets_chunk_sleep`。

### 5.7 延迟点燃队列不移植

fire spec v2 设计了**延迟点燃队列**，为的是防火在一帧内沿扫描方向烧穿整根木头。

**我们不需要它**：点燃时给目标盖当前戳，`eval` 开头 `c.stamp() == self.stamp` 就跳过。现有 stamp 机制白送。本条须写入 spec 决策记录并注明理由，避免日后有人照 v2 补队列。

**落地待办（审阅补漏 2026-08-31）**：总纲翻案 6 的连带变更明文"复用其设计
（**尤其延迟点燃队列**）"（`kernel-charter.md` §11），与本条相抵。Task 3 落地时
须在总纲 §11 实施期决策补一条、修正该处措辞——否则两份真源打架。

### 5.8 确定性纪律

| 流 | key | salt | attempt |
|---|---|---|---|
| `STREAM_IGNITE`（新增） | 源格坐标 | 0 | 方向骰 / 产火骰 |

与 `STREAM_REACT` 分流。现有已占 0–5（DIAG / EMIT / EXPLODE / FALLSTEP / SCANDIR / SPLASH），新增两个流接续编号。

### 5.9 counter 随材质移动

流动中的 oil 带着 counter 一起走：`displace` 移动的是整个 `Cell` 字（含 counter 位），自动跟随。**实施时须确认 `with_stamp` / `with_vel` 等访问器只改各自掩码位、不清掉 counter 位。**

### 5.10 改动面

`cell.rs`（counter 位段 + `CellRepr` 别名）、`material.rs`（燃烧字段）、`rules.rs`（`eval` 内燃烧推进与点燃）、`rng.rs`（两个新流）、`sand-harness::scenario`（校验）。

---

## 6. 确定性合并论证

### 6.1 写域：r 不变

| 新增写入 | 半径 |
|---|---|
| 气体移动 | 1 |
| 气体横向扩散 | 1 |
| 反应写邻居 | 1 |
| 点燃写邻居 | 1 |

全部 **r ≤ 1**，远在现有 `window.rs::MAX_WRITE_RADIUS = 12` 之内（该值 = `(V_MAX_CELL/VEL_ONE − 1) + DISPERSION_MAX + 1`，由 Layer G Task 1+2 的串接路径决定）。

**结论：M2 不改变 r，四相写域论证与编译期断言原样成立。** 本节须在实施时以显式论证形式写进代码注释，而不是默认成立。

### 6.2 定序

- 反应与点燃都发生在 `eval` 内，走既有扫描序（自下而上 + 每 tick 全局行方向相位 `(y ^ scan_flip) & 1`），**不引入新的顺序依赖**。
- 加载期：tag 展开顺序、反应条目排序按 `(id_a, id_b, 条目序)` 定序；稠密表构建与声明序无关。

### 6.3 blob 的前置条件（写给未来的自己）

若将来要做 `blob_radius`（一格反应产出一团），**三件事必须一起上**：① 反应独立成 pass；② 产物半径进 `window.rs` 的 r 编译期断言；③ 半径上界写死并在 I/O 层校验。理由与 `dispersion` 同性质（§11 实施期决策第 2 条）：凡取值直接决定 `WriteWindow` 读写半径的字段，越界即破坏 P4 写域论证本身。**Noita 实测 `blob_radius` 到 40。**

### 6.4 协议不变

tick 管线一个阶段都不加 ⇒ **不构成协议版本变更** ⇒ M2 只需在总纲 §11 记实施期决策，不需要新的翻案记录。

### 6.5 反应可达性与休眠（已知语义，审阅补记 2026-08-31）

反应推进依赖**发起方被 eval**。本轮成立的原因：所有反应都含火的一侧，而燃烧/寿命格
每 tick 必有 counter 写入，经 `mark_dirty_around` 的 ±1 边距（`window.rs:172`）
把相邻发起方（含跨 chunk）持续唤醒。**推论：两端皆眠的反应对永远不推进**——这按
Noita 语义是对的（睡眠 chunk 不模拟），且休眠状态本身是确定性的、不构成分叉风险。
但将来若加"静-静"反应（如 acid + wall），必须回头解决唤醒来源，立此存照。

---

## 7. 测试与验收

### 7.1 常规四件套（沿用 Layer G Task 3 体例）

1. **单测**：位段访问器（含 counter 不被其他 `with_*` 清掉）、加载期校验（引用不存在材质**必须报错**）、反应表规范化（正反只注册一次）、counter 装填/递减/归零转化。
2. **行为测试**（`tests/rules_behavior.rs`）：气体上浮不一帧连跳、发起方约定不双结算、由外向内燃烧（大块 wood 中心格在表面烧完前不减 counter）、`resting_wood_lets_chunk_sleep`。
3. **线程数不变性**：1/8/16 线程逐 tick 状态哈希逐位相同。
4. **SyncTest**：新场景 `fire_oil_chain`，双实例 2 万 tick 零分叉。

### 7.2 分布回归测试（本 spec 新立的规矩）

**概率分支必须验分布，不能只验哈希。**

理由：RNG salt 维度缺失这类 bug **两端一样地错，SyncTest 抓不到**——它不崩、不分叉，只是让某些结果永远不出现。外部实例见 `noita-grid-api-and-rng.md` §5.2（Noita 宝箱战利品事故，扛了数年）。既有先例是 Layer G Task 3 的 `splash_probability_is_per_cell_not_all_or_nothing`。

本轮至少两条：点燃方向骰四向均匀；反应触发率贴近声明概率。

### 7.3 golden 处置

`materials.ron` 改了（`blast_cost` → `hp` + `durability`）⇒ `materials_fp` 变 ⇒ **四个 golden 全部重录**。

- 含爆炸的 `explosion_ci`：状态哈希会因 durability 门槛而变（**预期内**）。
- 无火无爆炸的场景（`sand_pile` 等）：状态哈希应**逐位不变**，重录前用 `hashrun --grid-only` diff 取证——照 Task 2/3 的既有做法。

新增火油连锁 golden。

### 7.4 bench

反应/燃烧的每 cell 成本增量，落 `docs/perf/`。**顺带做 u64 feature 的对照测量**（§2.3）。

### 7.5 GIF 目检（用户手动执行）

火油连锁、烟上升、木头由外向内烧。

> 派 subagent 实施时禁止其在终端调 Godot / godot CLI（CLAUDE.md §2.4）；harness 出 GIF 属 cargo 侧，可由 subagent 跑。

---

## 8. 决策记录（brainstorm 2026-08-31，用户逐节确认）

1. **切法**：一份 spec、三 Task 串行（气体 → 反应表 → 燃烧），照 Layer G 成例。`durability` 替换并入 Task 2。
2. **材质规模**：最小集 8 种。**但 tag 机制不缩水**——总纲 §8 反例第一条禁止硬编码材质对，且 tag 展开加载期一次性完成、成本接近零。
3. **遗留债**：O3 粉末惯性、粒子穿水/弹跳、M1 两条测试债全部推迟，本 spec 记一笔即为裁决落地。
4. **整体方案**：采纳方案 1（全部塞进 `eval`，零新增 pass）。方案 2（反应独立 pass）的价值在 blob，本轮不做 blob 故换不来；方案 3（纯反应表做火）我们试过且不够用。
5. **Cell u64**：不扩，但做成随时可扩（`CellRepr` 别名 + 堵 pub 字段）+ 本轮量一次成本。理由：刚为性能删掉 Layer F，紧接着做未测过的 2× 内存加宽方向矛盾；且 bit 语义不变而字宽变会导致全部 golden 重录。
6. **反应查找结构**：作者格式稀疏（一条条反应），内部查找用**按实际材质数开的**稠密表。切换判据写死（材质 > 64 种或 bench 显示瓶颈），表放在单一访问器后面。
7. **counter 位段复用**：燃料池与寿命共用 bits 24–31，不设 `burning` 标志位。
8. **延迟点燃队列不移植**：现有 stamp 机制已解决 fire spec v2 要它解决的问题。
9. **审阅补漏（2026-08-31，spec 评审）**：① 点燃判定加"源 `counter > 0`"门 +
   目标氧气前置（§5.2，堵冷油自燃与装填/清零 ping-pong）；② 反应邻居检查跳过
   已盖当前戳的格（§4.5，堵同格一 tick 二次转化）；③ §5.7 与总纲翻案 6
   "延迟点燃队列"措辞相抵，Task 3 落地时同步总纲 §11；④ gas 扩散恒 1、不读
   `dispersion` 并加载期校验（§3.1）。另记三条备忘：counter 8 位天花板与分频
   解法（§2.3）、休眠 × 反应可达性（§6.5）、`needs_eval` flags 合并提示（§2.6）。
