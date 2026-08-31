> 文档路径：`docs/superpowers/specs/2026-05-26-fire-system-design.md`
> 运行时版本：Python 3.11+
> 最近更新：2026-06-06 (UTC+8)
> **Status**: **主线设计有效（2026-08-31 解挂）**，但运行时过时——本文写于 Python 原型时代。
> **状态变迁**：2026-08-29 曾因总纲翻案第 2 条（温度回归 Layer F 扩散场）被标为 Superseded/待重审；**2026-08-31 总纲翻案第 6 条删除 Layer F 场层、温度与燃烧回归 Noita 体系，本文主线（fire_hp 燃料池 + 静态着火点比较 + requires_oxygen + 延迟点燃队列）随之重新生效**，予以解挂。附录 A（温度场实验分支）维持降级。
> **实施方式**：M2 按 Rust 内核另立 spec，复用本文的机制设计与数值；**延迟点燃队列**尤其要带过去——它防的是帧内沿扫描方向的连锁偏置，与总纲 §11 翻案记录第 4 条同源。

# 火焰系统设计 v2：Noita 式 fire_hp 燃烧 + 静态温度比较（主线）

> **修订说明（2026-06-06，v1 → v2）**：调研证实 Noita **没有**每像素温度场与热传导（`docs/reference/noita-deep-dive.md` §2.3、§5.3），v1 的"温度场 + 传导"主线按用户裁决整体降级为本文**附录 A（实验分支）**；v1 完整原文见 git 历史（`git show f6c8917:docs/superpowers/specs/2026-05-26-fire-system-design.md`），旧反应表火焰的调参留档于 commit `b99b2ec`。
> 本版主线 = Noita 式；**所有随机判定走确定性契约**（`docs/proposals/2026-06-06-deterministic-parallel-and-netcode.md` D1/D2，下称"提案"）。
> **实施时序**：M0（counter RNG / hash / demo 回放）→ M0.5（单线程 4-pass 语义原型）之后实施本 spec。

---

## 1. 动机

旧反应表火焰（`materials.toml` 的 `fire+[flammable]` 概率替换）的问题不变：

- 火焰生成后立刻上浮远离燃料，无法持续蔓延
- 没有从外向内的燃烧效果，大块木头瞬间消失
- 概率参数难调平衡——太高爆燃，太低灭火
- 不支持不同材质不同燃烧速度

**v1 的误判修正**：v1 写"Noita 的做法是燃烧通过温度传播驱动"——调研证伪。Noita 的真实机制是：材质静态常量比较（火源 `temperature_of_fire` ≥ 邻居 `autoignition_temperature`）+ 每帧随机方向采样 + `fire_hp` 消耗，**无温度场、无传导**；连 lava 点火/固化都走反应表。v2 对齐该机制。

## 2. 目标与范围

| 做（主线） | 不做（主线） |
|---|---|
| `fire_hp` 消耗 + 燃尽转化（wood→ash）；`-1` 永燃 | 每像素温度场 / 热传导 → **附录 A 实验分支** |
| 静态点燃判定：源 `temperature_of_fire` ≥ 邻居 `autoignition_temp` | 对流、熔化等温度衍生效果 |
| `requires_oxygen` 表面燃烧 | 烟雾专用材质（暂复用 steam；字段已预留） |
| 灭火：缺氧 / `extinguisher` 接触 | 爆炸力学 |
| 水蒸发：复用"水被烧穿"机制，零额外系统 | stains 系统（Phase 3） |
| 燃烧颜色混合渲染 | 温度影响物理属性 |
| 全部随机走 counter RNG（完整 key，提案 D2） | |

## 3. 像素属性扩展

STRIDE 4 → 5（v1 的 TEMPERATURE 字段**不进主线**）：

```python
TYPE_ID  = 0
VELOCITY = 1
LIFETIME = 2
FLAGS    = 3
FIRE_HP  = 4   # 新增：剩余耐燃血量（运行时副本，set_cell 时初始化为材质 fire_hp）
STRIDE   = 5

FLAG_DIRTY   = 0b001
FLAG_STATIC  = 0b010
FLAG_BURNING = 0b100   # 新增：正在燃烧
```

### MaterialDef 新增字段

| 字段 | 类型 | 默认 | 语义 |
|---|---|---|---|
| `fire_hp` | int | 0 | 耐燃血量；0 = 不可燃；**-1 = 永燃**（Noita 同款语义） |
| `autoignition_temp` | int | 1000 | 点燃阈值（静态常量，0–1000 自定标度） |
| `temperature_of_fire` | int | 0 | 作为热源时辐射的"火温"（燃烧中的像素，或 fire/lava 这类天然热源） |
| `burn_rate` | int | 1 | 燃烧中每帧 `fire_hp` 减量 |
| `burn_to` | str | `"air"` | 燃尽转化目标材质 |
| `generates_fire` | float | 0.0 | 燃烧时在邻居 air 生成 fire 像素的概率 |
| `requires_oxygen` | bool | false | 燃烧是否需接触 air |
| `generates_smoke` / `smoke_material` | float / str | 0 / `"steam"` | 烟参数化（对应 Noita `generates_smoke` + `on_fire_smoke_material`）；**首版不实现，字段预留** |

**概率字段的确定性加载（提案 D1）**：TOML 中仍以 float 书写（保可读性），`MaterialRegistry` 加载时一次性量化为 u32 阈值——`threshold = min(round(p × 2**32), 2**32 - 1)`（2 的幂缩放无精度损失），运行时判定一律 `rng_u32(...) < threshold`。

## 4. 点燃与燃烧机制（主线）

### 4.1 概念

- **热源** = 带 `FLAG_BURNING` 的像素 ∪ **天然热源**（`temperature_of_fire > 0` **且 `fire_hp == 0`** 的材质——即 fire、lava）。⚠️ `fire_hp ≠ 0` 的可燃物（wood/oil）**只有燃烧中才辐射火温**——若不加 `fire_hp==0` 门控，冷油池（120 > 自身阈值 100）会无火自燃并蒸干邻水（评审 B1）。
- **点燃判定（Noita 式）**：每帧每个热源**随机采样 1 个方向**（counter RNG，salt=`FIRE_DIR`），若该邻居满足：`fire_hp ≠ 0`（可燃）且 `源.temperature_of_fire ≥ 邻居.autoignition_temp` 且（`不需氧` 或 `邻居接触 air`）→ 点燃（置 `FLAG_BURNING`）。**"每帧只采样 1/4 方向"本身就是蔓延速率的概率闸**（对应 Noita 原话 "look in a random direction to see if it can ignite that pixel"）。
- 点燃是即时判定，无热量累积——`temperature_of_fire` / `autoignition_temp` 只是比较用常量。

### 4.2 蔓延行为由数值编码（设计注记）

| 配置 | 推导出的行为 |
|---|---|
| wood：燃烧火温 80 < wood 阈值 150 | **燃烧的木头不能直接点燃相邻木头**——蔓延必须经由它喷出的 fire 像素（火温 200 ≥ 150）+ 氧气接触 → 火沿木头**表面**爬，无氧内部不烧（Noita 同款表面燃烧） |
| oil：燃烧火温 120 ≥ oil 阈值 100，`requires_oxygen=false` | **油直接相邻闪燃**，包括水下油层——油池一点即轰（Noita 同款） |
| water：阈值 100 ≤ fire 200 / lava 300，`fire_hp=1`，`burn_to="steam"` | 水被火/岩浆"点燃"后 1 帧烧穿成 steam——**蒸发复用燃烧机制，零额外系统** |
| 燃烧火温 80（wood）< water 阈值 100 ≤ 燃烧油火温 120 | 燃烧的木头烧不开水；**燃烧的油能**（120≥100）——但贴水的油通常先被扑灭（灭火规则），仅对角/侧面构型可见（评审 m6，测试钉死该行为） |

### 4.3 burn pass（确定性版）

```python
SALT_FIRE_DIR, SALT_FLAME_POS, SALT_FLAME = 10, 11, 12  # decision_salt 注册表

def _burn_pass(self):
    fire_id = self.registry.get_by_name("fire").type_id
    ignite_queue: list[tuple[int, int]] = []   # 延迟点燃
    spawn_queue: list[tuple[int, int]] = []    # 延迟生成 fire 像素

    for y in range(self.height):               # 固定遍历序（D3）；方向任选但锁死
        for x in range(self.width):
            base = self._base(x, y)
            type_id = self.cells[base + TYPE_ID]
            if type_id == AIR:
                continue
            mat = self.registry.get_by_id(type_id)
            burning = self.cells[base + FLAGS] & FLAG_BURNING

            # 1) 灭火检查（先于点燃，顺序固定）
            if burning:
                if mat.requires_oxygen and not self._has_air_neighbor(x, y):
                    self.cells[base + FLAGS] &= ~FLAG_BURNING; burning = False
                elif self._touching_tag(x, y, "extinguisher"):
                    self.cells[base + FLAGS] &= ~FLAG_BURNING; burning = False

            # 2) 热源点燃邻居：随机采样 1 个方向 → 入延迟队列
            # 天然热源须 fire_hp==0（评审 B1：否则冷油自燃）
            is_natural_source = mat.temperature_of_fire > 0 and mat.fire_hp == 0
            heat = mat.temperature_of_fire if (burning or is_natural_source) else 0
            if heat > 0:
                d = rng_choice(seed, tick, pass_id, x, y, SALT_FIRE_DIR, n=4)
                nx, ny = NEIGHBORS4[d](x, y)
                if self._can_ignite(nx, ny, heat):
                    ignite_queue.append((nx, ny))

            # 3) 燃烧推进
            if burning:
                if mat.fire_hp != -1:                       # -1 = 永燃
                    hp = self.cells[base + FIRE_HP] - mat.burn_rate
                    self.cells[base + FIRE_HP] = hp
                    if hp <= 0:
                        self.set_cell(x, y, self.registry.get_by_name(mat.burn_to).type_id)
                        continue
                if mat.generates_fire_threshold:            # 喷火苗 → 入延迟队列
                    for i, (nx, ny) in enumerate(neighbors4_shuffled(seed, tick, pass_id, x, y, SALT_FLAME_POS)):
                        if (self.in_bounds(nx, ny) and self.get_type_id(nx, ny) == AIR
                            and rng_u32(seed, tick, pass_id, x, y, SALT_FLAME, attempt=i) < mat.generates_fire_threshold):
                            spawn_queue.append((nx, ny)); break   # 每帧至多 1 个

    # 4) pass 末尾统一应用（防帧内沿扫描方向的连锁偏置：
    #    本帧被点燃的像素下一帧才成为热源；本帧生成的火苗下一帧才参与）
    for x, y in ignite_queue:
        # apply 时复检：目标可能已在本帧燃尽转化（如 wood→ash），陈旧条目会点燃 ash 并湮灭成 air（评审 M1，与 spawn_queue 的 AIR 复查对称）
        if self.registry.get_by_id(self.get_type_id(x, y)).fire_hp != 0:
            self.cells[self._base(x, y) + FLAGS] |= FLAG_BURNING
    for x, y in spawn_queue:
        if self.get_type_id(x, y) == AIR:    # 可能已被先入队者占用，按队列序确定裁决
            self.set_cell(x, y, fire_id)
```

- RNG 全部用完整 key `(seed, tick, pass_id, x, y, salt, attempt)`（提案 D2；Phase 1 串行 pass_id=0）。
- **延迟队列是刻意设计**：点燃/生成若在扫描中即时生效，先扫到的火会在同一帧内沿扫描方向连锁推进 → 方向性偏置（确定但难看）。队列化后每帧蔓延恰好一层，且队列按扫描序构建 → 仍然确定。
- `_can_ignite(nx, ny, heat)`：`in_bounds` 且目标 `fire_hp ≠ 0` 且 `heat ≥ 目标.autoignition_temp` 且（`不需氧` 或 `_has_air_neighbor(nx, ny)`）。
- `_touching_tag`：四邻居中存在带指定 tag 的材质（走 `registry.get_ids_by_tag`，数据驱动，不硬编码材质名）。**忽略与自身相同 type_id 的邻居——材质不能灭自己**：否则水池内部被点燃的水会被邻居水立刻扑灭，蒸发机制失效（倒置 bug：只有孤立水滴能蒸发）。已知代价：燃烧的油浮在水面会被下方水扑灭（Noita 中油膜可持续燃烧）——先接受，后续可用"浸没占比"规则细化。
- **pass_id 约定（评审 M5）**：Phase 1 的 burn pass 是全网格独立 pass，RNG key 的 `pass_id` 固定取 **4**（movement 棋盘格用 0–3；M0 串行期 movement 取 0）。**M1 并行化时 burn pass 按 chunk 拆入棋盘调度**：点燃采样的越界目标与 ignite/spawn 队列遵守与 movement 相同的写域/延迟规则，届时本节伪代码相应分块化（已记入提案 M1 范围）。

### 4.4 状态机

```
[正常] ──源火温≥阈值 且可燃 且(无需氧|邻air)──► [燃烧中] ──fire_hp≤0──► 变为 burn_to
   ▲                                              │
   └────── 缺氧 / extinguisher 接触 ◄─────────────┘   （已耗 fire_hp 不恢复）
```

对比 v1：没有"温度降到阈值以下熄灭"分支（无温度场）；灭火只有缺氧与灭火剂两条路。

## 5. materials.toml v2（主线目标态）

变更点：**density 整数化**（提案 D1，原 float ×10）；新增火属性与 ash；water 加 `extinguisher` tag；删除 3 条旧火焰反应。

```toml
[meta]
version = 2
default_grid_size = [128, 128]

[materials.wall]
cell_type = "solid"
density = 100
color = [128, 128, 128]
tags = ["solid"]

[materials.rock]
cell_type = "solid"
density = 90
color = [100, 100, 100]
color_variance = 8
tags = ["solid"]

[materials.wood]
cell_type = "solid"
density = 80
color = [139, 90, 43]
color_variance = 10
tags = ["solid", "flammable"]
fire_hp = 200
autoignition_temp = 150
burn_rate = 1
burn_to = "ash"
temperature_of_fire = 80
generates_fire = 0.15
requires_oxygen = true

[materials.ash]
cell_type = "powder"
density = 20
color = [60, 60, 60]
color_variance = 5
tags = ["powder"]

[materials.sand]
cell_type = "powder"
density = 60
color = [194, 178, 128]
color_variance = 15
tags = ["powder"]

[materials.water]
cell_type = "liquid"
density = 10
color = [48, 96, 255]
tags = ["liquid", "water", "conductive", "extinguisher"]
fire_hp = 1
autoignition_temp = 100
burn_to = "steam"
requires_oxygen = false

[materials.oil]
cell_type = "liquid"
density = 8
color = [80, 60, 30]
tags = ["liquid", "flammable"]
fire_hp = 60
autoignition_temp = 100
burn_rate = 2
burn_to = "air"
temperature_of_fire = 120
generates_fire = 0.3
requires_oxygen = false

[materials.lava]
cell_type = "liquid"
density = 30
color = [255, 96, 0]
tags = ["liquid", "lava", "hot"]
temperature_of_fire = 300

[materials.steam]
cell_type = "gas"
density = 1
color = [200, 200, 255]
lifetime = 300
tags = ["gas"]

[materials.fire]
cell_type = "energy"
density = 0
color = [255, 160, 40]
color_variance = 40
lifetime = 120   # 注：Noita flame≈5 帧；若大面积燃烧形成"两秒火云"，优先下调此值（评审 n4）
tags = ["energy", "hot"]
temperature_of_fire = 200

# --- 反应表（只剩纯化学反应，Noita 实证同款） ---

[[reactions]]
input = ["lava", "water"]
output = ["rock", "steam"]
probability = 0.8

# 必要补丁（评审 m5）：wood requires_oxygen=true，完全浸没岩浆时点燃判定永不触发（无 air 邻居）
# ——Noita 同款解法是反应表条目（materials.xml dump 第 14569 行 [lava]+[burnable]→[lava]+fire）
[[reactions]]
input = ["lava", "[flammable]"]
output = ["lava", "fire"]
probability = 0.1

# 删除（由燃烧系统取代）：
# fire+[flammable]→fire+fire / [hot]+wood→_self+fire / [hot]+water→_self+steam
```

## 6. update() pass 顺序

```
1. 清 dirty flags
2. 移动 pass（现有逻辑；random 已在 M0 换 counter RNG）
3. 反应 pass（现有逻辑，反应表精简后只剩 lava+water）
4. 燃烧 pass（新增 — §4.3，无温度场）
5. lifetime 衰减
6. frame_count++
```

## 7. 渲染（沿用 v1 设计）

燃烧颜色混合：`burn_ratio = 1 − fire_hp / max_fire_hp`，`blend = 0.3 + 0.7 × burn_ratio`，向 `FIRE_COLOR = (255, 100, 20)` 插值；`CellGrid.get_burn_state_array()` 返回 `(flags, fire_hp, max_fire_hp)` 三数组供 numpy 向量化混合。

## 8. 改动文件总览

| 文件 | 改动 | 内容 |
|---|---|---|
| `core/cell.py` | 修改 | STRIDE 4→5、`FIRE_HP`、`FLAG_BURNING` |
| `core/material.py` | 修改 | +7 字段；概率 u32 量化加载（D1） |
| `core/grid.py` | 修改 | `set_cell` 初始化 FIRE_HP；`_burn_pass` / `_has_air_neighbor` / `_touching_tag` / `get_burn_state_array`；update() 加 burn pass |
| `core/rules.py` | 不变 | fire 仍按 energy 运动（其随机在 M0 已换 counter RNG） |
| `core/reaction.py` | 不变 | — |
| `data/materials.toml` | 修改 | §5 全文 |
| `render/pygame_renderer.py` | 修改 | 燃烧颜色混合 |
| `tests/test_fire.py` | 新增 | §9 |
| `tests/test_grid.py` / `test_materials.py` | 修改 | 适配 STRIDE=5、新字段 |

## 9. 测试策略

counter RNG 使所有测试**天然确定**（固定 seed → 逐帧可断言，无 flaky）：

- **点燃阈值**：火温 80 vs 阈值 150 不点燃；200 ≥ 150 点燃；恰好相等点燃（≥ 语义）
- **requires_oxygen**：被实心包围的 wood 不点燃；表面点燃；烧出空腔后火向内推进
- **fire_hp**：每帧按 burn_rate 递减；归零转 `burn_to`；`-1` 永燃不减
- **灭火**：接触 `extinguisher` 清 FLAG_BURNING；缺氧熄灭；已耗 fire_hp 不恢复
- **灭火不自灭**：水池内部被点燃的水不被邻居水扑灭（同 type_id 忽略），正常蒸发；burning wood 接触水被扑灭
- **水蒸发**：fire 邻接 water → water 下帧变 steam（经燃烧机制，无需反应表）
- **蔓延路径**：wood 只经火苗+氧气蔓延（直接相邻不点燃）；oil 直接相邻闪燃（含无氧环境）
- **延迟队列**：同帧大面积火源只向外推进一层（无扫描方向连锁偏置）
- **集成**：火烧木从外向内→ash；油池闪燃；lava 蒸发水 + lava+water→rock
- **冷油静置**（评审 B1 回归）：纯油池 + 邻接水，静置 N 帧不自燃、水不蒸发
- **陈旧点燃条目**（评审 M1 回归）：hp 临界帧邻接双热源，燃尽产物 ash 存活不被点燃
- **燃油烧水**（评审 m6 钉行为）：对角构型下燃烧油可蒸发水；贴水油先被扑灭
- **lava 浸没木头**（评审 m5）：无 air 接触的 wood 经反应表 `lava+[flammable]→lava+fire` 点燃；lava+water 双机制（反应表 vs 点燃蒸发）的结果比例钉到具体场景
- **灰烬闷熄**（评审 m10，已知特性）：平顶木块燃烧产生的 ash 堆积隔绝 air 可闷熄火——集成测试"火烧木从外向内"用**侧立面/斜面几何**，平顶闷熄单独立测试钉为特性
- **确定性**：同 seed 同 hash（接 M0 回归套件）；任一随机点改动测试即红

---

## 附录 A：温度场 + 热传导（实验分支，未实施）

v1 主线设计整体降级至此，作为日后差异化实验——它能做 Noita 式近似不了的效果：**岩浆随时间冷却、热水渐沸、冰冻扩散、热传导穿墙预警**。

**开启前置条件**（三者缺一不开工）：

1. 温度休眠条件设计：温差 < ε 即夹断为环境温度并停更，否则全场 diffusion 使 chunk 永不休眠，对冲 dirty rect 全部收益（提案 §6 风险 1 同源）；
2. benchmark 过关（CLAUDE.md §5.3：新增系统必须跑基准）；
3. 确定性合规：增量缓冲固定遍历序、整数温度、扩散系数定点化（D1/D3）。

**要点存档**：STRIDE 追加 TEMPERATURE 字段；`DIFFUSION_RATE=0.1`、`NATURAL_COOLING=1`、`TEMP_MAX=1000`；增量缓冲两遍法（先算 delta 后统一应用）；热源钉温（燃烧中/天然热源温度钉在 `temperature_of_fire`）；air 作散热器（k=1.0）。完整算法、参数表与测试清单见 v1 原文：`git show f6c8917:docs/superpowers/specs/2026-05-26-fire-system-design.md`。

另注：CLAUDE.md §5.2 的示例反应 `(Lava, t>300) → [Rock]`（按温度冷却固化）属于本附录范围——Noita 中不存在该机制（lava 固化全部由接触反应触发），是我们的自选差异化动作。
