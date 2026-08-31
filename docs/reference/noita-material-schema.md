> 文档路径：`docs/reference/noita-material-schema.md`
> 运行时版本：Rust（内核）+ Godot 4 + gdext（表现层）
> 最近更新：2026-08-31 (UTC+8)
> **Status**: Implemented（调研结论，非提案）
> 姊妹篇：`docs/reference/noita-grid-api-and-rng.md`（网格写操作词汇 / 能量射线三兄弟 / 真实 vs 装饰粒子 / PRNG 事故史）——本篇讲材质表本身，那篇讲除材质表之外引擎怎么碰网格。
> 定位：`docs/reference/noita-deep-dive.md` §2.2 的**字段级展开**，为 M2（场层与反应表）的 `materials.ron` schema 设计提供一手锚点。前者讲"Noita 材质模型长什么样"，本文讲"每一个字段叫什么、什么类型、默认值多少、vanilla 实际怎么用"。

# Noita 材料定义字段全解（一手数据核查）

## 0. 取证方法与可信度

| 来源 | 取法 | 可信度 |
|---|---|---|
| **游戏真实数据** `materials.xml`（369 KB） | `git clone --depth 1 https://github.com/vexx32/noita-data`，本地 `xml.etree` 全量统计 | ★★★ 一手，游戏解包原文 |
| **官方 wiki 属性表 + 默认值全清单** | `noita.wiki.gg/api.php?action=parse&prop=wikitext&page=Modding%3A_Making_a_custom_material` | ★★☆ 社区实验 + 逆向，wiki 自己标注了哪些是"存疑/假字段" |
| **Reaction 字段文档** | 同上，`page=Documentation%3A_Reaction` | ★★☆ 同上 |
| **对照组** The Powder Toy `Element.h` / `ElementDefs.h` | `raw.githubusercontent.com/The-Powder-Toy/The-Powder-Toy/master/src/simulation/` | ★★★ 一手源码 |

> 取证注记：`noita.wiki.gg` 的 HTML 页面与 `?action=raw` 被 WAF 拦（403/Blocked），**`api.php` 端点可直连**——后续要复查 wiki 内容走 API。

本次 dump 的规模数字（与 deep-dive §2.2 记录的旧版本略有出入，以本次为准）：

- **445 条材质定义**：`<CellData>` 207 + `<CellDataChild>` 238；`name` 只有 443 个不同值——`rock_box2d` 与 `meat_pumpkin` 各出现两次，即**同名后定义覆盖前定义**（wiki 明确说明这是覆盖语义，不是报错）。
- **255 条 `<Reaction>`** + 5 条 `<ReqReaction>`，与材质**平级**挂在 `<Materials>` 根下。
- 材质属性去重后 **92 个**；子节点 11 类（`Graphics` / `StatusEffects` / `Stains` / `Ingestion` / `StatusEffect` / `ParticleEffect` / `ExplosionConfig` / `Edge` / `EdgeGraphics` / `Images` / `Image`）。

---

## 1. 结构：一条材质由三部分组成

```xml
<Materials>
  <CellData name="..." wang_color="..." cell_type="liquid" density="4" ...>   <!-- ① 属性 -->
    <Graphics color="ff4f5eab" texture_file="data/materials_gfx/water.png"/>  <!-- ② 子节点 -->
    <StatusEffects>
      <Stains>    <StatusEffect type="WET"/></Stains>
      <Ingestion> <StatusEffect type="POISONED" amount="0.1"/></Ingestion>
    </StatusEffects>
  </CellData>

  <CellDataChild _parent="water" name="water_swamp" .../>                     <!-- ③ 继承 -->

  <Reaction probability="80" input_cell1="lava"  input_cell2="water"
                             output_cell1="rock" output_cell2="steam"/>       <!-- 反应与材质平级 -->
</Materials>
```

三条**结构性**事实，比任何单个字段都重要：

1. **反应不是材质的属性**，是与材质平级的全局条目。材质通过 `tags` 参与匹配。→ 与我们总纲 §4"反应 = (matA,matB)→{概率,产物} 数据表"同构。
2. **继承是"复制并覆写"**：`<CellDataChild _parent="X">` 必须有 `_parent`（缺则崩溃），`<CellData>` 则**禁止**有 `_parent`。另有独立开关 `_inherit_reactions`（0/1）决定是否连父材质的反应一起继承——**继承粒度是"属性"与"反应"分开的**。
3. **一个最小可用材质只需 3 个属性**：`name` + `wang_color` + `ui_name`。dump 里的 `air` 就是活证据，它的全部属性是 `{name, ui_name, wang_color}`，其余全吃默认——即 air 在引擎眼里就是一条"默认液体"。

---

## 2. 类型系统：4 个 `cell_type`，6 种实际行为

`cell_type ∈ {solid, liquid, fire, gas}`，默认 `liquid`。但**真正决定行为的是 `cell_type` 与两个布尔开关的组合**：

| 实际行为类 | 判据 | 本次 dump 实测条数（继承解析后） |
|---|---|---|
| 液体 | `liquid` + `liquid_sand=0` | 92 |
| 粉末 | `liquid` + `liquid_sand=1` | 81 |
| **静态地形** | `liquid` + `liquid_sand=1` + `liquid_static=1` | **101** |
| 刚体固体（Box2D） | `solid` | 131 |
| 气体 | `gas` | 20 |
| 火 | `fire` | 18 |

关键结论（wiki 原话转述）：**"地面不是 solid，是静态液体沙"**。`solid` 只留给进 Box2D 的物理块（箱子、冰、玻璃）——"如果你把整个世界做成 solid，它会因为不断自碰撞而卡死"。

> 对我们的意义：我们现在的 `Category { Static, Powder, Liquid }`（`crates/sand-core/src/material.rs:29`）等价于 Noita 的"静态地形 / 粉末 / 液体"三类，**恰好命中它 445 条里的 274 条**。M2 要补的是 `gas`（Layer F）与 `fire`，刚体留 M3。Noita 用"一个类型 + 两个开关"表达 6 类，是为了共用代码路径；我们用显式枚举，可读性更好、匹配 clippy 执法，不必抄这个技巧。

前缀即作用域：`liquid_*` 只在 `cell_type="liquid"` 生效、`solid_*` 只在 `solid` 生效、`gas_*` 只在 `gas` 生效，**写错前缀不报错、静默无效**（wiki 明说）。这是数据驱动系统的典型坑——我们的 RON 加载期应当**显式拒绝**不适用字段，而不是静默忽略。

---

## 3. 属性全表

标注约定：**n=** 本次 dump 中显式出现次数（继承前，即"有多少条材质亲自写了它"）；**默认值**取自 wiki 的 Defaults 清单（该清单本身是社区从引擎逆向导出的完整默认 `<CellData>`）。

### 3.1 通用（跨类型）

| 属性 | 值域 | 默认 | n | 语义 |
|---|---|---|---|---|
| `name` | string | 必填 | 445 | 内部唯一 id，重名 = 覆盖 |
| `ui_name` | string | 同 `name` | 442 | 玩家可见名（`$mat_xxx` 走翻译表） |
| `_parent` | material | — | 242 | 继承源，仅 `CellDataChild` 可用/必填 |
| `_inherit_reactions` | 0/1 | 1 | 185 | 是否连父的反应一起继承 |
| `cell_type` | solid\|liquid\|fire\|gas | liquid | 208 | 见 §2 |
| `tags` | `[a],[b]` | "" | 302 | 反应匹配的唯一多态入口，见 §5 |
| `density` | 1–50（无硬上限） | 1 | 216 | 液体/粉末分层：重的沉。**实测有小数**（acid 2.9、blood 4.1） |
| `hp` | 5–1 000 000 | 100 | 202 | 抗爆能量池，见 §6 |
| `durability` | 0–14（实测出现 16） | 0 | 63 | 破坏门槛，见 §6 |
| `lifetime` | 0–1350 | 0 | 28 | **仅液体/气体**：寿命到就消失（flame=5、smoke=350、steam=1000） |
| `electrical_conductivity` | 0/1 | 1（液体且非沙时），否则 0 | 46 | 导电 |
| `stickyness` | 0.0–∞ | 0 | 11 | 实体在其中的减速量 |
| `slippery` | 0/1 | 0 | 14 | 表面打滑（冰） |
| `stainable` | 0/1 | 1 | 7 | 是否会被染色 |
| `status_effects` | GAME_EFFECTS 枚举 | — | 63 | 简写版状态施加（wiki 疑为旧机制，新机制是 `<StatusEffects>` 子节点） |
| `platform_type` | -1/0/1/2 | -1 | 101 | 0=可穿过、1=可站立 |
| `show_in_creative_mode` | 0/1 | 0 | 231 | 编辑器可见 |
| `gfx_glow` / `gfx_glow_color` | 0–255 / ARGB | 0 | 139/18 | 自发光（表现层） |
| `wang_color` | ARGB hex | 0 | 445 | **必须全局唯一**：世界生成时按像素颜色反查材质 |
| `wang_noise_percent` / `wang_noise_type` / `wang_curvature` | 0–3.5 / 0–2 / 0.25–0.5 | 1 / 0 / 0.5 | 26/13/19 | 世界生成噪声（语义未确证） |
| `is_just_particle_fx` | 0/1 | 0 | 1 | 纯特效材质 |
| `ignore_self_reaction_warning` | 0/1 | 0 | 0 | 仅抑制自反应告警 |
| `cell_holes_in_texture` | 0/1 | 0 | 5 | 纹理 alpha=0 处不放材质 |

### 3.2 液体/粉末（`liquid_*`）

| 属性 | 值域 | 默认 | n | 语义 |
|---|---|---|---|---|
| `liquid_sand` | 0/1 | 0 | 171 | 1 = 粉末（可堆叠、可站立） |
| `liquid_static` | 0/1 | 0 | 38 | 1 = 不流动，放哪儿是哪儿（静态地形） |
| `liquid_slime` | 0/1 | 0 | 5 | 流得更慢 |
| `liquid_gravity` | 0.0–5.0 | 0.5 | 170 | 下落加速度。实测：slime 0.1、lava 0.2、oil 0.5、water 1.5、gunpowder 2、flame 5.0 |
| `liquid_flow_speed` | 0.0–1.0 | 0.9 | 10 | 横向流速 |
| `liquid_viscosity` | 0–150 | 50 | 44 | **社区实测"看不出差别"**，疑似半废弃 |
| `liquid_damping` | 0.9 | 0.8 | 6 | 其他取值会导致 glitch |
| `liquid_sticks_to_ceiling` | 0/1（vanilla 写 50/100） | 0 | 30 | 需 `liquid_sand=1`；用于"不该动的地表沙" |
| `liquid_sand_never_box2d` | 0/1 | 0 | 2 | 刚体可穿过 |
| `liquid_stains` | 0–4 | 0 | 54 | 0 无、1 只给状态、2 染实体+世界、3 只染世界、4 仅 oil 用 |
| `liquid_stains_self` / `liquid_stains_custom_color` | 0/1 / ARGB | 0 / 0 | 24/1 | 自染色 |
| `liquid_sprite_stain_shaken_drop_chance` | 0–5 | 1 | 20 | 污渍抖落速度 |
| `liquid_sprite_stain_ignited_drop_chance` | 0–10 | 10 | 3 | 着火时污渍消失速度 |
| `liquid_sprite_stains_status_threshold` | 0.2–0.3 | 0.01 | 8 | 沾多少才触发状态 |
| `liquid_sprite_stains_check_offset` | -1–1 | 0 | 9 | 语义未确证 |
| `convert_to_box2d_material` | material | air | 11 | 受伤时整块转成刚体材质 |

### 3.3 刚体固体（`solid_*`）

| 属性 | 默认 | n | 语义 |
|---|---|---|---|
| `solid_static_type` | 0 | 50 | 0=会掉落/玩家可穿，1=完全静态，2–5=静态但玩家可穿 |
| `solid_friction` | 0.3 | 94 | 越小越滑 |
| `solid_restitution` | 0.2 | 19 | 弹性 |
| `solid_gravity_scale` | 1 | 11 | 重力倍率（负数 = 往上掉） |
| `solid_collide_with_self` | 1 | 25 | 自碰撞 |
| `solid_go_through_sand` | 0 | 5 | 穿过粉末 |
| `solid_break_to_type` | air | 38 | 碎裂后变成什么 |
| `solid_on_break_explode` | 0 | 1 | 碎裂时爆炸 |
| `solid_on_collision_convert` / `_material` | 0 / air | 1/31 | 撞击后转化 |
| `solid_on_collision_explode` / `_splash_power` | 0 / 1 | 6/8 | 撞击爆炸（需 `<ExplosionConfig>`） |
| `solid_on_sleep_convert` | 0 | 10 | 睡眠时转化 |
| `crackability` | 0 | 13 | 玻璃/冰式碎裂难度（越高越易碎） |

### 3.4 气体（`gas_*`）

| 属性 | 默认 | n |
|---|---|---|
| `gas_speed` | 50 | **0** |
| `gas_upwards_speed` | 100 | **0** |
| `gas_horizontal_speed` | 100 | **0** |
| `gas_downwards_speed` | 90 | **0** |

**四个气体字段在整个 vanilla `materials.xml` 里一次都没被写过**——20 种气体全吃默认。即：Noita 官方内容里所有气体的运动参数完全一致，差异只体现在 `lifetime`、`density`、反应和颜色上。M2 做气体时这是个强信号：**先上一套全局气体参数，per-material 气体调参优先级很低**。

### 3.5 火与燃烧

| 属性 | 值域 | 默认 | n | 语义 |
|---|---|---|---|---|
| `burnable` | 0/1 | 0 | 214 | 能否点燃 |
| `autoignition_temperature` | 0–100 | 100 | 97 | **静态常量**，不是动态温度 |
| `temperature_of_fire` | 0–200 | 10 | 198 | **静态常量**：本材质燃烧时的"火温" |
| `fire_hp` | -1–99999999 | 0 | 74 | 燃料量，**-1 = 永燃** |
| `on_fire` | 0/1 | 0 | 218 | 出生即燃 |
| `on_fire_convert_to_material` | material | "" | 1 | 烧完变成什么 |
| `on_fire_flame_material` | material | fire | 4 | 喷出的火焰材质 |
| `on_fire_smoke_material` | material | smoke | 2 | 产出的烟材质 |
| `generates_flames` | 0–30 | 30 | 4 | 火焰生成量 |
| `generates_smoke` | 0–20（实测 250） | 0 | 193 | 烟生成概率 |
| `requires_oxygen` | 0/1 | 1 | 209 | 0 = 只有暴露在空气的边缘燃烧 |
| `always_ignites_damagemodel` | 0/1 | 0 | 21 | 碰到就点燃实体 |

**点火判据**（社区共识 + 数据自洽）：燃烧源每帧随机选一个方向采样邻居，若 `源.temperature_of_fire ≥ 邻居.autoignition_temperature` 则概率点燃。自洽锚点：`nest` 的 autoignition=85，`fire` 火温 100 能点燃它、`flame` 火温 60 不能。

### 3.6 熔化/冻结（wiki 属性表**未收录**，只在 Defaults 清单里出现）

| 属性 | 默认 | n |
|---|---|---|
| `warmth_melts_to_material` | air | 15 |
| `warmth_melts_chance_rev` | 100 | 5 |
| `cold_freezes_to_material` | air | 14 |
| `cold_freezes_chance_rev` | 100 | 0 |
| `cold_freezes_to_dont_do_reverse_reaction` | 0 | 1 |

这五个字段是 Noita 材质模型里**唯一的"内建相变"**，且用量极小（15/14 条）——绝大多数熔化/冻结走的仍是反应表（`[meltable] + [lava] → [meltable]_molten`）。

### 3.7 敌人 AI 提示 / 音频 / 植被

`danger_fire`(22) `danger_water`(4) `danger_radioactive`(8) `danger_poison`(3)——纯给 AI 寻路避险用。
`audio_physics_material_solid/_wall/_event`(207/208/49) `audio_materialaudio_type`(26) `audio_materialbreakaudio_type`(6) `audio_is_soft`(19) `audio_size_multiplier`(5) `audio_event_name`(3)。
`vegetation_sprite`(8) `vegetation_full_lifetime_growth`(8) `vegetation_random_flip_x_scale`(0)。

### 3.8 **假字段**（wiki 显式列出：vanilla 里写了但引擎不读）

`collapsible`、`supports_collapsible_structures`、`liquid_solid`、`color`、`solid_break_on_explosion_rate`、`explosion_power`。

> wiki 原话："Nolla does that sometimes"。这是抄别人数据表时的常见陷阱：**vanilla 数据里出现 ≠ 引擎实现了**。本次 dump 里这 6 个字段确实都出现过（各 1–6 次），若不看 wiki 会误以为是有效特性。

### 3.9 子节点

| 节点 | n | 内容 |
|---|---|---|
| `<Graphics>` | 374 | `color`(ARGB)、`texture_file`、`fire_colors_index`、`normal_mapped`、`is_grass`，以及 9 个方向性像素色 `pixel_top_left`…`pixel_all_around`（用于边缘着色） |
| `<StatusEffects><Stains>/<Ingestion>` | 126 | 内含 `<StatusEffect type= amount=>`，158 条；type 30 种（`FOOD_POISONING` 43、`POISONED` 23、`INGESTION_ON_FIRE` 22…） |
| `<ParticleEffect>` | 55 | 20 个字段，全是表现层（`vel_random.min_y`、`gravity.y`、`lifetime.min/max`、`airflow_force`…） |
| `<ExplosionConfig>` | 17 | 28 个字段（`ray_energy`、`cell_explosion_power/radius_min/max`、`damage`、`camera_shake`…），与反应共用同一份 ConfigExplosion 结构 |
| `<Edge><EdgeGraphics>/<Images><Image>` | 62/262 | 边缘装饰贴图（表现层） |

**这里有个可直接抄的分层原则**：Noita 把「表现」（Graphics / ParticleEffect / Edge）与「逻辑」（属性 + Reaction）放在同一份文件的**不同子节点**里，而不是打散在属性里。我们的 `materials.ron` 目前把 `color` 当顶层字段——M2 扩表时应当拆成 `visual: (...)` 子结构，因为**颜色不入状态哈希，逻辑字段入**，两者的确定性地位根本不同。

---

## 4. 默认值清单（wiki 逆向导出，M2 直接可参照）

完整默认 `<CellData>` 见 wiki "Defaults" 段。几个对我们有决策价值的：

- `cell_type="liquid"`、`density="1"`、`hp="100"`、`durability="0"`、`lifetime="0"`
- `liquid_gravity="0.5"`、`liquid_flow_speed="0.9"`、`liquid_viscosity="50"`、`liquid_damping="0.8"`
- `autoignition_temperature="100"`（= 默认不自燃）、`temperature_of_fire="10"`、`fire_hp="0"`、`burnable="0"`
- `platform_type="-1"`、`electrical_conductivity="1"`（条件性）、`stainable="1"`
- `ui_name` 默认复制 `name`；`wang_color="0"`

**"默认值必须是安全值"**是这份清单的设计意图：不写任何东西就得到一坨惰性液体，不会自燃、不会腐蚀、不参与任何反应。我们的 `materials.ron` 已经在按同一原则做（`splash_chance` 缺省 0 = 永不溅射、`vaporize_threshold` 缺省 1.0 = 永不汽化、`dispersion` 缺省 1），**继续保持**。

---

## 5. `tags`：唯一的多态机制

- 本次 dump 共 **71 个不同 tag**，302 条材质带 tag。
- Top：`[alchemy]`181、`[corrodible]`162、`[solid]`147、`[earth]`121、`[liquid]`76、`[box2d]`73、`[static]`69、`[impure]`52、`[liquid_common]`51、`[burnable]`41、`[water]`31。
- **255 条反应里 127 条（50%）引用了至少一个 tag**——tag 不是点缀，是反应表的一半。

三种 tag 用法：

1. **普通匹配**：`[corrodible]` + acid → 酸能腐蚀的一切。加一种新岩石只需打上 tag，**不用碰反应表**。
2. **词缀展开**（10 条）：`input="[meltable]" → output="[meltable]_molten"`，即"tag 命中的材质名 + `_molten`"。要求 `wax_molten` 这类材质**确实存在**，否则该条反应静默失效。这是 Noita 表达"整个金属家族的熔化"的手段——一条规则覆盖 16 种可熔金属。
3. **引擎内建 tag**（3 个，非数据定义）：`[*]`=任意材质（未使用）、`[any_liquid]`=`liquid`且`liquid_sand=0`、`[any_powder]`=`liquid`且`liquid_sand=1`。

> 对 M2 的直接结论：**tag 系统必须和反应表同时上，不能推迟**。否则每加一种材质就要手写 N 条反应，正是总纲 §8 明令禁止的组合爆炸。词缀展开建议**先不做**（它把"材质名字符串"变成了逻辑输入，加载期解析失败是静默的——不符合我们"加载期显式报错"的纪律）；用显式的 `产物映射表` 替代。

---

## 6. 破坏模型：`durability` 门槛 + `hp` 能量池（双层）

wiki `Materials` 主页给出的语义（一手表格）：

- **`durability`（0–14）是门槛**：每种法术/爆炸有一个"最大可破坏 durability"，材质 durability 超过它 → **完全无伤**。sand=4、cheese=5、coal/concrete=8、静态肉=9、dense rock/soil=10、rusted steel=11、steel/极密岩=12、dense steel=13、brickwork/cursed rock=14。对应地：光弹能挖 8、火球/火箭/矿工能挖 10、炸药/激光 11、核弹 12、巨核/发光钻头/长枪 14。
- **`hp` 是能量池**：爆炸从爆心发多条射线，每条带 `ray_energy`，逐格扣该格材质的 `hp`，能量耗尽即停。举例：Energy Orb 的 ray_energy=350 000，在 coal（hp=25 000）里能推很远，在 rock（hp=100 000）里只能挖 2–3 格。
- 未定义 durability 时，只按 hardness/hp 计算。

**这正是我们 M1 `blast_cost` 的完整版**。`crates/sand-core/src/material.rs:11` 的注释已经写明"M2 反应表引入 durability/hardness 后此字段语义细化替换"——现在有了具体形状：

```
现状：blast_cost: u32（BLAST_COST_INFINITE 哨兵表达"免疫"）
M2 ：durability: u8   ← 门槛，射线/法术带 max_durability，超过即免疫（哨兵不再需要）
     hp: u32          ← 能量池，等价于现在的 blast_cost
```

`BLAST_COST_INFINITE = u32::MAX` 这个哨兵可以在 M2 干净退役：wall 只要 `durability = 15`（高于任何法术的上限）即可，语义比"无限能量消耗"更直白，也不再依赖"power 不会超过某个界"的隐含假设。

---

## 7. `<Reaction>` 字段全表

| 属性 | 默认 | 实测 n | 语义 / 确定性注记 |
|---|---|---|---|
| `input_cell1` / `input_cell2` | unknown | 255/255 | 材质名或 `[tag]` |
| `output_cell1` / `output_cell2` | unknown | 255/255 | 同上 |
| `input_cell3` / `output_cell3` | "" | 31/15 | **三元反应**（如 flummoxium+blood+oil） |
| `probability` | 0 | 255 | **取 float ×100 转 int**，即两位小数定点。实测出现 `0.3` → 内部是 30/10000 |
| `fast_reaction` | 0 | 19 | 与竞争反应冲突时优先——**顺序依赖，见下** |
| `blob_radius1` / `blob_radius2` | 0 | 30/31 | 输出扩散半径。**实测取值 {2,3,4,5,6,15,40}** |
| `blob_restrict_to_input_material1/2` | 1 | 24/15 | blob 只覆盖对应输入材质 |
| `convert_all` | 0 | **0** | 接触面全转化——**vanilla 一次都没用** |
| `direction` | none | 17 | none/top/bottom/left/right（枚举 -1..3） |
| `req_lifetime` | 0 | 5 | 输入材质 lifetime 门槛 |
| `destroy_horizontally_lonely_pixels` | 0 | 6 | 清理孤立像素 |
| `entity` | "" | 4 | 在反应点生成实体 |
| `cosmetic_particle` | "" | 5 | 纯表现粒子 |
| `audio_fx_volume_1` | 0 | 13 | 音量 |
| `<ExplosionConfig>` 子节点 | — | 17 | 反应触发爆炸 |

`<ReqReaction>`（5 条）：**条件不满足时才触发**的反向反应，全部形如 `X + air → soil + air`（植物在没有支撑时枯萎）。

**`unknown` 容错**：`unknown` 不是真材质，是引擎的"不存在材质"占位符。任何引用 `unknown` 的反应会被**整条丢弃并打日志**，而不是崩溃。这是数据驱动系统面对 mod 的必需品——我们的加载期应当**反过来做**：引用不存在材质 = 加载失败并报错，因为两端数据表必须一致（总纲 §1 P5），静默丢弃反应会造成**双端反应表不同 → 分叉**。这是一个必须与 Noita 反着抄的点。

### 7.1 三条对我们 P4 写域论证的硬约束（重点）

1. **`blob_radius` 直接决定反应的写半径**。Noita 实测最大到 **40**。我们的四相棋盘论证目前 r=12、余量 4（总纲 §4 Layer G，`window.rs` 的 `MAX_WRITE_RADIUS` 编译期断言）。→ **M2 反应表必须给 `blob_radius` 设上界，且上界进同一条编译期断言**；否则一条数据配置就能悄悄破掉并行确定性论证。这不是手感旋钮，与 `dispersion` 同性质（`material.rs:23` 的先例）。
2. **`convert_all` 的写域无界**（"如同两个输入都有极大 blob 半径"）。vanilla 零使用。→ **不要抄**。
3. **`fast_reaction` 是顺序语义**："与其他反应冲突时优先"。在我们这里必须落成**确定性定序**（按反应表声明序/id 排序后取最小者胜），绝不能是"先匹配先赢"依赖遍历顺序。同理 Noita 的 `direction` 与我们总纲已定的"发起方约定（id 小者）"是同一类问题的两种解法——我们已有的方案更简单，保留。

---

## 8. 温度：Noita 没有温度场（本次 dump 再次核实）

本次全量属性统计 92 个字段中，与温度相关的只有 **2 个，且都是材质静态常量**：`autoignition_temperature`（97 次，0–100）与 `temperature_of_fire`（198 次，0–200）。**不存在任何逐像素动态温度、热容、导热、热扩散字段**。lava 点燃可燃物是反应表条目，lava 固化是接触反应（+water/+blood/+cement），**没有"随时间冷却"**。

对照 The Powder Toy 的 `Element` 结构（`src/simulation/Element.h`，一手源码）：

```cpp
float Advection, AirDrag, AirLoss, Loss, Collision, Gravity, NewtonianGravity, Diffusion, HotAir;
int   Falldown, Flammable, Explosive, Meltable, Hardness, Weight;
unsigned char HeatConduct;  float HeatCapacity;           // ← 真·热力学
float LowPressure;    int LowPressureTransition;          // ← 压强相变阈值
float HighPressure;   int HighPressureTransition;
float LowTemperature; int LowTemperatureTransition;       // ← 温度相变阈值
float HighTemperature;int HighTemperatureTransition;
unsigned int Properties;                                   // PROP_CONDUCTS / PROP_RADIOACTIVE / PROP_LIFE_DEC ...
int (*Update)(UPDATE_FUNC_ARGS);                           // ← 每种元素一个 C 函数
```

**两条路线泾渭分明**：

| | Noita | The Powder Toy |
|---|---|---|
| 状态量 | 材质 id + 少量位 | 每粒子 type/temp/life/tmp/ctype/vx/vy |
| 相变 | 反应表（数据） | 温度/压强阈值字段 + 每元素 `Update` C 函数 |
| 温度 | **无场**，两个静态常量 | 真温度场 + `HeatConduct`/`HeatCapacity` + 环境热 |
| 扩展方式 | 加一行数据 | 写一个 C 函数 |

> **与我们总纲的关系（必须点明）**：总纲 §11 翻案记录第 2 条已经**推翻**了"跟 Noita 走无温度场路线"，把温度定为 Layer F 的 pull 双缓冲扩散场，并注明"fire spec v2 的 burn pass / 反应表分工在 M2 实施前按本文重审"。本次调研**不构成翻案依据**——Noita 的无温度场是它的选择，不是它的能力上限。但调研给出两条实操参考：
> - 走温度场后，Noita 那两个静态常量应当**换形**：`autoignition_temperature` → 材质的**着火点**（与场值比较），`temperature_of_fire` → 燃烧时的**产热率**。字段名可留，语义从"静态比大小"升级为"场的源与阈"。
> - TPT 的 `HeatConduct`（u8）+ `HeatCapacity`（float）是温度场必需的两个 per-material 字段，Noita 完全没有。我们做 Layer F 就要补上，且**必须定点化**（总纲 §6：网格逻辑纯整数）。

---

## 9. 继承的实际用法（一手观察）

- 242 条材质用了 `_parent`，指向 **56 个不同父材质**；最热门的父：`aluminium`(24)、`rock_static`(21)、`templerock_static`(16)、`meat`(14)。
- **继承按用途而非分类**：`gold_static` 的 `_parent` 是 **`wood_static`**——因为它要的是"静态、可燃、有 fire_hp"这组行为，不是因为金和木有分类关系。
- `_inherit_reactions` 用了 185 次（165 次为 1、20 次为 0）——即**有 20 条材质刻意继承属性但不继承反应**，典型是"外观像水但不参与水的反应"的魔法液体。

> 对 M2 的建议：继承是把 445 条材质压缩到可维护规模的关键机制，值得抄，但**必须在加载期展开成扁平表**（core 只见扁平结果），且**禁止循环**（加载期检测）。属性继承与反应继承分开开关这个设计很实用，一并抄。

---

## 10. 对 M2 的落地清单

我们现在的材质定义（`data/materials.ron`，4 种材质 8 个字段）：

```ron
(id, name, category, density, color, blast_cost, vaporize_threshold, dispersion, splash_chance)
```

按本次调研，M2 建议的字段演进（**仅为调研结论，正式设计走 `superpowers:brainstorming` + proposals**）：

| 动作 | 字段 | 依据 |
|---|---|---|
| **拆分** | `color` → `visual: (color, glow, …)` 子结构 | §3.9：表现字段不入哈希，逻辑字段入，两者确定性地位不同 |
| **新增** | `tags: [...]` | §5：反应表的一半；没有它必然规则组合爆炸（总纲 §8 反例第 1 条） |
| **替换** | `blast_cost` → `durability: u8` + `hp: u32` | §6：Noita 双层破坏模型；顺带干掉 `BLAST_COST_INFINITE` 哨兵 |
| **新增** | `lifetime`（气体/火） | §3.1：flame=5、smoke=350、steam=1000，是气体层的基本调参位 |
| **新增** | `burnable` / `ignition_temp` / `fire_hp` / `heat_output` | §3.5 + §8：温度场版的着火点与产热率 |
| **新增** | `heat_conduct` / `heat_capacity`（定点） | §8：TPT 有、Noita 没有，走温度场路线就必须补 |
| **暂缓** | 气体 `gas_*` 四参数 | §3.4：vanilla 20 种气体全吃默认，per-material 调参优先级极低 |
| **不抄** | `convert_all`、词缀展开、`unknown` 静默丢弃 | §5 / §7.1 / §7：分别是写域无界、静默失败、双端表不一致三类风险 |
| **加断言** | 反应 `blob_radius` 上界 | §7.1：写域论证的输入，必须进 `window.rs` 同一条编译期断言 |

数值表示注记（总纲 §6 纯整数/定点红线）：

- Noita 的 `density` **有小数**（acid 2.9、blood 4.1）——我们 `density: u16` 若要保留同等分辨率，应约定单位（如 ×10）而非改浮点。
- `probability` 在 Noita 内部就是 **×100 的定点整数**（实测出现 0.3）——我们的反应概率直接定为整数万分比即可，RON 里写小数、加载期一次性量化，**沿用 `vaporize_threshold` / `splash_chance` 的既有体例**（`material.rs:53`、`material.rs:74`）。
- `liquid_gravity` 0.0–5.0 → 我们已有 Q3.2 竖直速度位段（`cell.rs` bits 17–21，上限 4.0 格/tick），量级吻合，无需改位段。

---

## 11. 引用

| 资料 | 用途 |
|---|---|
| [vexx32/noita-data](https://github.com/vexx32/noita-data)（`materials.xml`, 369 KB） | 一手材质数据，本文全部统计数字的来源 |
| [Modding: Making a custom material](https://noita.wiki.gg/wiki/Modding:_Making_a_custom_material) | 属性分组表 + **完整默认值清单** + 假字段清单（走 `api.php` 取） |
| [Documentation: Reaction](https://noita.wiki.gg/wiki/Documentation:_Reaction) | Reaction 全字段 + 引擎内建 tag + `unknown` 语义 |
| [Materials（wiki 主页）](https://noita.wiki.gg/wiki/Materials) | durability 门槛表 / hp 能量池语义 |
| [The Powder Toy `Element.h`](https://github.com/The-Powder-Toy/The-Powder-Toy/blob/master/src/simulation/Element.h) | 对照组：热力学 + 相变阈值 + per-element `Update` 函数 |
| `docs/reference/noita-deep-dive.md` §2.2–§2.4 | 本文的上层概览，结论一致，本文提供字段级细节 |
