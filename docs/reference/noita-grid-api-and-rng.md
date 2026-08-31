> 文档路径：`docs/reference/noita-grid-api-and-rng.md`
> 运行时版本：Rust（内核）+ Godot 4 + gdext（表现层）
> 最近更新：2026-08-31 (UTC+8)
> **Status**: Implemented（调研结论，非提案）
> 姊妹篇：`docs/reference/noita-material-schema.md`（材质字段全解）。那篇讲"一个材质有哪些字段"，本篇讲"**除了材质表之外，Noita 还怎么碰这块网格**"——上层系统写网格的接口词汇、能量射线三兄弟、真实/装饰粒子分界、以及它的随机数事故史。

# Noita 引擎侧笔记：网格写操作词汇、粒子分界与 PRNG 事故

## 0. 这份文档从哪来

上一轮只读了 3 个 wiki 页面就收工，属于**取样不足**。本轮把 `noita.wiki.gg` 的 `Modding:`（36 页）与 `Documentation:`（198 页）两个命名空间全部枚举，挑出 38 篇与内核相关的抓下原始 wikitext 通读。

取法（HTML 页面被 WAF 拦，API 端点不拦）：

```bash
# 枚举
curl "https://noita.wiki.gg/api.php?action=query&format=json&list=allpages&apprefix=Documentation%3A&aplimit=500"
# 批量取正文（一次最多 50 页）
curl "https://noita.wiki.gg/api.php?action=query&format=json&formatversion=2&prop=revisions&rvprop=content&rvslots=main&titles=A|B|C"
```

> 这条路对以后复查同样有效：**不需要开浏览器**，wikitext 比渲染页更适合 grep 和批量处理。

---

## 1. 核心发现：上层碰网格靠一套**封闭的操作词汇**，不是任意访问

Noita 的实体层（component/system）想改动落沙网格，只能通过一组**参数化组件**，每个组件对应一种网格操作。翻完 Documentation 命名空间，这套词汇是：

| 组件 | 网格操作 | 关键参数 |
|---|---|---|
| `CellEaterComponent` | 半径内吃掉格子 | `radius` / `eat_probability`(0–100) / `limited_materials`+`materials`(排序表) / `ignored_material_tag` / `only_stain` / `eat_dynamic_physics_bodies` |
| `MagicConvertMaterialComponent` | 半径内材质转化 | `radius`+`min_radius`（**环带**）/ `is_circle` / `steps_per_frame`=10 / `from_material_tag` / `from_any_material` / `from_material_array`→`to_material_array` / `extinguish_fire` / `ignite_materials`(概率) / `fan_the_flames`(调 `UpdateFire()` N 次) / `temperature_reaction_temp`（走材质的 `cold_freezes_to`/`warmth_melts_to`） |
| `LiquidDisplacerComponent` | 推开液体 | `radius`(0–20) / `velocity_x` / `velocity_y` |
| `LooseGroundComponent` | 射线松动地形/塌方 | `probability` / **`max_durability`** / `max_distance`=256 / `max_angle` / `min_radius`–`max_radius` / `chunk_probability`（掉 box2d 块）/ `collapse_images` |
| `MaterialSuckerComponent` | 吸取材质入容器 | `material_type`(0 液/1 沙/2 气) / `barrel_size` / **`num_cells_sucked_per_frame`=1** / `suck_tag` / `suck_static_materials` |
| `MaterialAreaCheckerComponent` | **只读查询**：AABB 是否全是某材质 | `area_aabb` / `material`+`material2` / `update_every_x_frame` / `count_min` / `mPosition`（"keeps track where we are"） |
| `MaterialSeaSpawnerComponent` | 铺一片材质（海/湖） | **`speed`=每帧每方向覆盖多少像素** / `sine_wavelength`/`amplitude` / `noise_scale`/`threshold` |
| `PixelSpriteComponent` | 把 PNG 盖章成网格材质 | `image_file` / `material` / `diggable` / `clean_overlapping_pixels` / `create_box2d_bodies` |
| `PhysicsImageShapeComponent` | 把 PNG 变成刚体 | `image_file` / `material` / `is_circle` / `body_id`（分组） |
| `VegetationComponent` | 长植物 | `is_real_pixels`（真格子）vs `is_visual`（一个格子 + 贴图）/ `material_on_top_of` / `height_check` |
| `ParticleEmitterComponent` | 发射粒子 | `create_real_particles` / `emit_real_particles` / `emit_cosmetic_particles` / `collide_with_grid` / `count_min`–`count_max` / `emission_interval_min_frames` |
| `ExplosionComponent` + `ConfigExplosion` | 爆炸 | 见 §2 |
| `LaserEmitterComponent` + `ConfigLaser` | 激光 | 见 §2 |
| `ElectricityComponent` | 闪电 | 见 §2 |

**这套词汇里反复出现的五个参数模式**（比任何单个组件都值钱）：

1. **半径 + 形状**（`radius`、`min_radius` 环带、`is_circle`）
2. **概率是 0–100 的整数**，不是浮点（`eat_probability`、`create_cell_probability`、`ignite_materials`）
3. **材质筛选靠 tag 或"排序过的材质表"**（`suck_tag`、`ignored_material_tag`、`materials` 注释原文 "is a list of accepted materials **sorted**"）
4. **每帧预算**（`steps_per_frame`、`num_cells_sucked_per_frame`、`speed`、`update_every_x_frame`、`reaction_speed`）——**几乎每个能改网格的系统都带一个**
5. **`durability` 门槛是通用的**（不止爆炸用，`LooseGroundComponent.max_durability`、`ConfigLaser.max_cell_durability_to_destroy` 都用）

### 对我们的意义

我们的 `Op` 枚举（`crates/sand-core/src/world.rs:16`）现在是 `Brush / Fill / Emit / Explode` 四条。Noita 走到成品形态时这套词汇大约十来条——**这验证了我们"跨层通信白名单 = 一个封闭的 Op 枚举"的路子是对的**，也给出了扩展时的形状参考：新增能力 = 新增一条参数化 Op，而不是给上层开放网格写权限。

三条可以直接拿的：

- **每帧预算必须进 Op 参数**。Noita 加预算是为了性能（`steps_per_frame`、"5–10 seems like a nice speed"）；对我们是**确定性刚需**——预算必须是状态的纯函数，绝不能是"这帧还剩多少毫秒"。`MaterialAreaChecker` 用 `mPosition` 记住扫到哪儿、跨帧续扫，这个"游标存进状态"的做法我们可以照抄。
- **概率统一为整数**。Noita 组件层用 0–100 int，材质反应层用 ×100 定点。我们已有的"RON 写小数、加载期量化"体例（`vaporize_threshold` / `splash_chance`）与之一致，继续统一。
- **材质筛选统一为 tag + 排序表**。"sorted" 那个注释值得注意：即便在不追求确定性的引擎里，材质列表也是排序的。我们的定序遍历红线（总纲 §6）在这里能白拿一个佐证。

---

## 2. 能量射线三兄弟：爆炸 / 激光 / 闪电共用一套模型

三个系统结构完全同构，都是"**带能量预算的射线 + durability 门槛**"：

| 系统 | 能量 | durability 门槛 | 备注 |
|---|---|---|---|
| 爆炸 `ConfigExplosion` | `ray_energy`=20000 | `max_durability_to_destroy`=10 | 官方注释原文："If cells have a hp of 20, rays with 100 energy can penetrate 5 cells" |
| 激光 `ConfigLaser` | `damage_to_cells`=5000 | `max_cell_durability_to_destroy`=12 | `max_length`=512、`beam_radius`=2.5 |
| 闪电 `ElectricityComponent` | `energy`=1000 | —（用 `electrical_conductivity` 走线） | `speed`=32、`splittings_min/max`（**分叉**）、`splitting_energy_min/max`、`probability_to_heat`、`hack_is_set_fire` |

这就把 `noita-material-schema.md` §6 的双层破坏模型钉死了：**`hp` 是材质的能量池、`durability` 是材质的门槛，而"我能打穿多硬的东西"是操作（法术）的参数**。三个独立系统各自带一个 `max_*durability*` 字段，没有一个把门槛写死在材质侧。

### `ConfigExplosion` 逻辑字段 vs 我们的 `Op::Explode`

`ConfigExplosion` 共 72 个字段，绝大多数是表现（sprite/light/audio/camera_shake/sparks）。剔掉表现后的逻辑子集，与我们 `crates/sand-core/src/explode.rs` 逐条对照：

| Noita 字段（默认值） | 语义 | 我们的对应物 |
|---|---|---|
| `ray_energy`=20000 | 每条射线的能量 | `Op::Explode { power }`（`world.rs:32`） |
| `max_durability_to_destroy`=10 | 可摧毁的最高 durability | **缺**——M2 随 `durability` 字段一起补 |
| `explosion_radius`=20 | 半径 | `Op::Explode { r }` |
| `hole_enabled`=true | 是否真的挖洞 | 恒 true |
| `hole_destroy_liquid`=**false** | 撞到液体是**摧毁**还是**抛向空中** | 我们的 `vaporize_threshold`（`material.rs:53`） |
| `create_cell_material`=fire / `create_cell_probability`=5 | 被摧毁的格子有 5% 概率变成火 | **缺**——M2 火层的廉价点火源 |
| `destroy_non_platform_solid_enabled`=true | 是否摧毁非平台固体 | 我们用 `blast_cost` 哨兵表达免疫 |
| `sparks_inner_radius_coeff`=0.333 | 火花只生成在 0.333R–R 的**环带** | 我们的"近心汽化、外圈溅射"是同一类内外分区 |
| `material_sparks_scale_with_hp` / `material_sparks_min_hp`=10000 | 材质越硬火花越多 | 表现层可选 |
| `pixel_sprites_enabled`=true | 是否切开 pixel sprite | M3 刚体后再说 |
| `delay`=ValueRangeInt / `explosion_delay_id` | 延迟爆炸、桶连爆共用同一 id | M4 法术连锁 |
| `never_cache`=true | 不缓存，当帧就炸 | 说明**爆炸默认是可缓存/批处理的** |

两条值得记的：

1. **`hole_destroy_liquid` 默认 false = "液体被炸飞而不是被删除"**。我们的做法更细：不是一个 bool，而是 per-material 的能量比例阈值（近心删除、外圈脱格成粒子）。这条对照说明我们在这个点上做得比 Noita 细，不是漏做。
2. **爆炸可缓存/批处理**（`never_cache` 的存在暗示默认路径会攒批）。在我们这里，攒批的**顺序**就是协议——如果将来做爆炸批处理，入队序必须是状态的纯函数，与 Layer P 的粒子 id 序同性质。

---

## 3. 真实粒子 vs 装饰粒子：引擎自己就分了两层

`ParticleEmitterComponent` 有三个并列开关：

- `create_real_particles`
- `emit_real_particles`
- `emit_cosmetic_particles`
- 外加 `collide_with_grid`（默认 true）

`<Reaction>` 上有 `cosmetic_particle` 字段；`VegetationComponent` 有 `is_real_pixels`（真格子）vs `is_visual`（一个格子 + 挂贴图）；`ConfigExplosion` 里有 `material_sparks_real`（"if the spark particles created are **real or fake**"）；材质表里有 `is_just_particle_fx`。整份数据里，**"real vs cosmetic" 这条线被反复画了五遍**。

> 这是对我们架构的直接佐证：Layer P（真实粒子，进状态、进哈希、参与落格）与 Channel B（表现层碎屑，Godot 自己玩）的分离不是我们的洁癖，是这类引擎的通用解法。Noita 没有确定性要求都要分，我们更必须分。
>
> 反过来也给了一条边界：`grid::CosmeticParticleConfig` 有 `render_on_grid` 字段——装饰粒子可以"看起来在网格上"但不参与模拟。表现层做水花、火星、烟尘时，**默认应该走装饰粒子**，只有需要被后续物理消费的才升级成 Layer P 真实粒子。我们现在 `Op::Emit` 产出的全是真实粒子，将来加表现层特效时别走这条路。

---

## 4. 反应引擎不止跑在网格上——瓶子里也跑

`MaterialInventoryComponent`（药瓶/粉袋的容器组件）有这么几个字段：

| 字段 | 默认 | 注释原文 |
|---|---|---|
| `do_reactions` | 0（0–100） | "if > 0, will do CellReactions between the materials" |
| `reaction_speed` | 5 | "how many pixels of material do we convert at one time (5-10) seems like a nice speed" |
| `do_reactions_explosions` | false | 允许反应触发爆炸 |
| `do_reactions_entities` | false | 允许反应生成实体 |
| `reactions_shaking_speeds_up` | true | 摇瓶子让反应更快 |
| `count_per_material_type` | — | "Count of each material indexed by material type ID" |
| `max_capacity` | -1 | < 0 = 无限 |

即：**同一张反应表既作用于网格的相邻格对，也作用于容器里的材质多重集**（容器内容是 `材质 id → 数量` 的表，不是像素）。容器反应还能被"摇晃"加速——一个纯玩法的输入通道接进了同一个反应引擎。

对我们的启示：反应表的接口应当定义在"**一对接触中的材质**"上，而"接触从哪来"是可插拔的（网格邻格 / 容器内容 / 未来的法术混合）。M2 实现反应 pass 时，把匹配与产物计算做成不依赖网格坐标的纯函数，网格 pass 只负责提供 (matA, matB) 与写回位置——这样 M4 做法术调合时能直接复用。

另外 `MaterialSuckerComponent.material_type` 用 `0=liquid / 1=sand / 2=gas` 这个三值枚举来分类可吸取的材质——注意它**把 sand 单列了**，与 `cell_type` 的四值枚举不是一套。说明即使 Noita 内部，"cell_type + liquid_sand 开关"这个设计在下游也要反复重新拼类别。我们的显式 `Category` 枚举（`material.rs:29`）在这一点上更省事。

---

## 5. PRNG 事故史：Noita 踩过的坑正是我们红线要防的

wiki 的 `Documentation: PRNG Quirks`（重定向自 `Technical: Noita PRNG`）是本轮最意外的收获。

### 5.1 它的模型

- 用 **Lewis-Goodman-Miller / Park-Miller LCG**（乘子 16807，31 位状态）。
- 一切随机 = `SetRandomSeed(x, y)` 设状态 + `Next()` 取值。
- 播种用**世界种子 + 一对坐标**，"每个随机事件既依赖世界种子也依赖事件发生的位置"。不依赖位置的事件（下雨、天赋牌堆）用硬编码的假坐标，还有些用帧号当坐标。
- 可以把整个 RNG 想成一个立方体：`SetRandomSeed(x,y)` 跳到 `(x,y,0)`，每次 `Next()` 沿 z 轴前进一格。

**这个模型和我们的 `hash(tick, x, y, salt/stream)` 是同一个思路的两种实现**：Noita 是"位置播种 + 有状态序列"，我们是"位置 + 用途直接哈希、无状态"。它已经在一个出货游戏上验证了"位置播种"这条路的可行性。

### 5.2 事故一：RNG Overlap —— 正是我们翻案第 4 条防的东西

> 同一对坐标上二次调用 `SetRandomSeed(x, y)`，z 轴归零，后续 `Next()` **重放同一串值**。

具体事故（2023 年 3 月才修）：额外血量点位的宝箱，生成时用 `(x,y,0)` 判定"值落在 0.3–0.7 之间才生成宝箱"；**开箱时又调了一次 `SetRandomSeed(x,y)`**，于是战利品表的第一次掷骰又拿到了那个**已知落在 0.3–0.7 的值**。后果：需要 `< 0.07` 的炸弹、需要 `0.94–0.95` 的心之拟态，**永远不可能出现**——将近一半的战利品表是死的，持续了好几年。

> **这就是总纲 §11 翻案记录第 4 条的原话所指**："salt/stream 必须区分同帧同格的多次掷骰"。我们那条是 2026-06-06 外部评审提出的理论隐患；现在有了一个出货游戏踩进去、坑了数年才发现的实例。
>
> 记两条操作性结论：
> 1. **这类 bug 不会崩、不会分叉、不会被测试抓到**——它只是让某些结果永远不出现。我们的 SyncTest 抓不到它（两端一样地错）。唯一的防线是**设计期把 salt/stream 维度写死**，以及对概率分支做分布回归测试（我们已有先例：`splash_probability_is_per_cell_not_all_or_nothing`）。
> 2. Noita 的补救是"在少数需要二次播种的地方加偏移量"——这正是 salt 的手工版。我们把它做成 API 强制参数，比它安全。

### 5.3 事故二：LCG 的尾部相关性

> 因为选的是 16807 乘子的 LCG，**一个非常接近 0 或 1 的采样，会限制下一个采样的取值范围**——距端点不会超过 16807 倍距离。

具体事故：大宝箱的 Sampo 需要 `> 0.99999`，命中后下一个采样"实测从不低于 ~0.883"。于是本该 1/1000 的真知之球，实际是 **1/168，比设计值高近 6 倍**。

> 对我们的意义：这是"**别用顺序消费的 RNG 流**"（总纲 §6）的第二个理由——除了执行顺序耦合，低质量流本身还有相关性。我们用哈希而非 LCG 流，天然免疫；但它反过来提出一个要求：**哈希函数的雪崩质量本身是确定性红线的一部分**。将来若有人为了性能提议"每格存一个廉价 LCG 状态往前推"，这一条就是现成的反例。

### 5.4 事故三：坐标经过浮点序列化被截断

`PositionSeedComponent` 存宝箱出生坐标（让宝箱搬走后战利品不变），但"由于一个有问题的序列化函数，只保留 6 位有效十进制数字"——X = 1234567 会以 1234570 参与播种。

> 对我们：**逻辑坐标绝不能经过浮点序列化往返**。我们的坐标是整数 + `Fx` 定点，snapshot/replay 走整数编码，这条已经被架构堵死；但 harness 场景文件是 RON 十进制小数，加载期量化——**量化必须发生在加载期一次、结果进哈希**，这一点现有实现（`quantize_fx` / `quantize_splash_chance` 等）已经做对了，值得在此记一笔为什么必须这么做。

---

## 6. 组件更新顺序 = 公开协议

`Modding: Component Update Order` 页面把 **~130 个系统的每帧更新顺序逐条列了出来**，开场白是"这些更新例程的运行顺序在做 mod 时有时很重要"。

顺序里能读出的分层（截取）：输入/变换 → 角色移动与碰撞 → AI → **网格操作类（`CellEaterComponent` → `DamageModelComponent` → `ExplosionComponent` → `LooseGroundComponent` → `MagicConvertMaterialComponent` → `MaterialAreaChecker` → `MaterialInventory` → `MaterialSucker`）** → 物理（`PhysicsBody2` / `PhysicsBody` / `PhysicsJoint`）→ 弹体 → 粒子发射 → Lua → Verlet → IK → 渲染。

> 我们总纲 §5 红线第 7 条"tick 管线顺序 = 协议，改 `step()` 内部阶段顺序 = 改协议版本"在这里拿到一个外部佐证：**一个没有确定性联机需求的单机游戏，都必须把系统更新顺序当作对外契约公开**。我们有双端逐位一致的要求，只会更严。
>
> 附带一个可抄的组织方式：Noita 的顺序里，**所有"写网格"的系统聚在一起、且排在物理之前**。我们的规范 tick 管线（架构 §4）同样是"ops → 网格相 → 粒子相 → ..."，方向一致。

---

## 7. 零散但值得记的事实

- **`fire` 是引擎唯一硬性要求存在的材质**："如果你把所有材质都删掉，你至少还得留一个 name id 为 `fire` 的占位材质，否则游戏启动就崩。" 我们的对应物是 `MAT_AIR=0` / `MAT_WALL=1` 两个强制哨兵（`material.rs:6-7`，加载期校验）——同一类设计，但我们把它写进了校验而不是靠崩溃暴露。
- **密度相同的液体会混合且无法再分离**（Density 页）。这对"分层液体"玩法是个设计约束：想让两种液体可分离，密度就必须不同。M2 排材质数值表时值得记住。
- **反应产物同时是反应物 = 自加速链**：steel + lava → lava（钢熔成岩浆，岩浆再熔更多钢，向下自我加速）；gunpowder 着火 → 连锁爆炸直到燃料耗尽。Alchemy 页把这些当作核心玩法在教学。**这既是"环境连锁"卖点的来源，也是 M2 最该压测的性能/确定性场景**——建议 golden replay 里专门放一个自加速链场景。
- **同一反应物配不同伴侣产出不同固体**：lava+water→rock、lava+blood→volcanic rock、lava+mud→ground。反应表的表达力主要来自这种组合，不来自字段复杂度。
- **方向性反应是可感知的玩法**：Alchemy 页教学"中和毒泥**只能自上而下**起作用，所以水必须在毒泥上方"。这就是 `<Reaction direction=>` 字段（vanilla 17 条在用）的玩家侧体验。我们总纲已定的"发起方 = id 小者"约定解决的是双结算问题，与 `direction` 不是一回事——**如果 M2 想要这类"从上往下才反应"的玩法，需要单独的字段**，别指望发起方约定顺带解决。
- **实体 tag 与材质 tag 是两套系统**：`Modding: Tags System` 讲的是实体/组件 tag（每个 tag manager 上限 **512** 个、**永不回收**、耗尽实体 tag 会崩游戏）。材质 tag 是 materials.xml 里的另一套。别把两者混为一谈；不过"tag 是有限资源、不要动态生成 tag"这条工程教训对任何 tag 系统都成立。
- **`DAMAGE_TYPES` 有 22 种**（`Modding: Enums`），其中 `DAMAGE_MATERIAL` / `DAMAGE_MATERIAL_WITH_FLASH` 是"被材质伤害"的专用类型。M4 做法术伤害类型时可作参考起点。
- **材质造成的伤害不写在材质上**：写在受伤实体的 `DamageModelComponent.materials_that_damage` / `materials_how_much_damage`（两个逗号分隔字符串，按材质名索引）。即**"谁怕什么"是受害者的属性，不是材质的属性**。这个方向对 1v1 对战很有用：角色/护盾可以声明对特定材质的抗性，而不用改材质表。

---

## 8. 汇总：本轮新增的 M2 待办

在 `noita-material-schema.md` §10 那张表之外，本轮追加：

| 动作 | 内容 | 依据 |
|---|---|---|
| **补 Op 参数** | `Op::Explode` 加 `max_durability`（能打穿多硬） | §2：三个独立系统都把门槛放在操作侧 |
| **补 Op 参数** | 爆炸摧毁格按概率转 `create_cell_material`（火） | §2：M2 火层最廉价的点火源 |
| **设计约束** | 反应匹配/产物做成不依赖网格坐标的纯函数 | §4：同一张表要能跑在容器上（M4 法术调合） |
| **设计约束** | 每帧预算进 Op 参数、扫描游标进状态 | §1：预算必须是状态的纯函数 |
| **测试** | golden replay 增加"自加速链"场景（钢+岩浆式） | §7：M2 性能与确定性的最坏情况 |
| **测试** | 概率分支加分布回归测试（不只是哈希一致） | §5.2：RNG overlap 类 bug 两端一样地错，SyncTest 抓不到 |
| **文档** | 总纲 §11 翻案第 4 条补一条外部实例引用 | §5.2：Noita 宝箱战利品事故 |
| **暂缓/不抄** | `direction` 式方向性反应 | §7：M2 先不做，需要时单开字段 |

---

## 9. 引用

| 页面 | 用途 |
|---|---|
| [Documentation: PRNG Quirks](https://noita.wiki.gg/wiki/Documentation:_PRNG_Quirks)（重定向到 Technical: Noita PRNG） | §5 全部：LCG 实现、RNG overlap 事故、相关性事故、坐标序列化截断 |
| [Documentation: ConfigExplosion](https://noita.wiki.gg/wiki/Documentation:_ConfigExplosion) | §2：72 字段，`ray_energy` / `max_durability_to_destroy` / `hole_destroy_liquid` |
| [Documentation: ConfigLaser](https://noita.wiki.gg/wiki/Documentation:_ConfigLaser) / [ElectricityComponent](https://noita.wiki.gg/wiki/Documentation:_ElectricityComponent) | §2：能量射线三兄弟的另两个 |
| [Documentation: MaterialInventoryComponent](https://noita.wiki.gg/wiki/Documentation:_MaterialInventoryComponent) | §4：容器内反应、`reaction_speed` |
| [Documentation: CellEaterComponent](https://noita.wiki.gg/wiki/Documentation:_CellEaterComponent) / [MagicConvertMaterialComponent](https://noita.wiki.gg/wiki/Documentation:_MagicConvertMaterialComponent) / [LooseGroundComponent](https://noita.wiki.gg/wiki/Documentation:_LooseGroundComponent) / [MaterialSuckerComponent](https://noita.wiki.gg/wiki/Documentation:_MaterialSuckerComponent) / [MaterialAreaCheckerComponent](https://noita.wiki.gg/wiki/Documentation:_MaterialAreaCheckerComponent) / [MaterialSeaSpawnerComponent](https://noita.wiki.gg/wiki/Documentation:_MaterialSeaSpawnerComponent) / [LiquidDisplacerComponent](https://noita.wiki.gg/wiki/Documentation:_LiquidDisplacerComponent) | §1：网格写操作词汇表 |
| [Documentation: ParticleEmitterComponent](https://noita.wiki.gg/wiki/Documentation:_ParticleEmitterComponent) / [grid::CosmeticParticleConfig](https://noita.wiki.gg/wiki/Documentation:_grid::CosmeticParticleConfig) / [VegetationComponent](https://noita.wiki.gg/wiki/Documentation:_VegetationComponent) | §3：真实/装饰粒子分界 |
| [Modding: Component Update Order](https://noita.wiki.gg/wiki/Modding:_Component_Update_Order) | §6：~130 个系统的公开更新顺序 |
| [Modding: Tags System](https://noita.wiki.gg/wiki/Modding:_Tags_System) / [Modding: Enums](https://noita.wiki.gg/wiki/Modding:_Enums) | §7：实体 tag 的 512 上限、DAMAGE_TYPES |
| [Fire](https://noita.wiki.gg/wiki/Fire) / [Alchemy](https://noita.wiki.gg/wiki/Alchemy) / [Density](https://noita.wiki.gg/wiki/Density) | §7：引擎必需材质、自加速链、方向性反应的玩家侧体验、密度分层 |
| [Documentation: DamageModelComponent](https://noita.wiki.gg/wiki/Documentation:_DamageModelComponent) | §7：`materials_that_damage` 在受害者侧 |
