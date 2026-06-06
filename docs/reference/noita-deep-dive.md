> 文档路径：`docs/reference/noita-deep-dive.md`
> 运行时版本：调研文档（服务 Phase 1 Python 原型 → Phase 2 Godot 4.5 + C#）
> 最近更新：2026-06-06 (UTC+8)

# Noita 深度调研：目标效果、核心算法、与朴素落沙 CA 的差距

本文综合 4 路并行网络调研（效果全景 / 运动学扩展 / 材质与火系统数据挖掘 / 刚体与多线程）与本仓库 prototype 现状对照。每个事实簇标注置信度：

- **[官方]** 开发者直述（GDC 2019 talk、80.lv 访谈、官方页面）
- **[社区]** wiki / datamine / 开源复刻作者直述
- **[通用]** 通用落沙技术，非 Noita 特有
- **[推测]** 合理推断，无来源直证

---

## 0. TL;DR — 七条最重要的结论

1. 我们 prototype 的核心循环骨架（单缓冲 in-place、自底向上、每帧左右交替、dirty flag，`prototype/core/grid.py:54-96`）与 Noita **结构一致** [官方]。基础没走偏。
2. 与 Noita 的真正差距**不在 CA 规则，而在运动学**：Noita 像素带速度、受重力积分、单帧最多位移 32px；我们一切运动都是 1 格/帧。这是"棋盘演示感"与"物理感"的分水岭（§3.1）。
3. Noita 打击感的标志性机制是 **CA↔粒子双轨**：高速像素脱离网格成弹道粒子（血溅、水花、爆炸碎屑），落点再写回网格——飞溅华丽且"落地为实"（§3.2）。
4. **Noita 没有温度场、没有热传导**（已二次验证：官方机制引语 + 游戏数据文件结构证据，§2.3）。火 = 材质静态常量比较（火源 `temperature_of_fire` ≥ 邻居 `autoignition_temperature`）+ 每帧随机方向概率点燃 + `fire_hp` 消耗。我们的 fire spec（每像素温度场 + 传导 pass）是**自创设计**，有明确性能风险——§5.3 给裁决建议。
5. Noita 的 `cell_type` 只有 4 种：solid / liquid / fire / gas。**粉末不是独立类型**——sand = liquid + `liquid_sand="1"`（粉末复用液体代码路径）；大量静态地形用 `liquid_static` 而非 solid。"魔法"全部在材质参数 + 反应表里 [社区]。
6. 玩家/敌人**不是 Box2D 刚体**，是矩形 hitbox 的自定义 kinematic 控制器，对像素网格做逐像素碰撞检测；Box2D 只管 props（矿车、油灯等）[社区，FSS 作者直述引 Noita]。
7. 既有文档 `docs/algorithms/parallel-update-strategies.md` 两处需精化：32px 约束的精确形式是"chunk + 四个正方向各 32px 的**十字写域**"；64×64 是模拟 chunk，**落盘/流式单位是 512×512**（双层结构）（§4.1、§4.5）。

---

## 1. 目标效果：Noita 实际交付了什么

> 这一节回答"我们要追的效果标杆是什么"。算法本身见 §2–§4。

### 1.1 总规模

- 官方宣称："**Every pixel in the world is simulated**, allowing you to burn, explode or melt anything, and swim in the blood of your foes!"；官方 FAQ："we run **simplified simulations of various chemical reactions, electricity, thermodynamics**" [官方]。
- 材质 **~400+ 种** [社区 wiki]：固体 ~150、普通液体 ~90、魔法液体 ~40、粉末 ~30、气体 ~15、火数种。"500+"传闻未确证。
- 低分辨率是刻意选择："doing such a **low resolution would allow you to use a lot of CPU power per pixel**" — Petri Purho [官方]。

### 1.2 五类物质的玩家可见行为

| 类别 | 行为要点 | 玩法意义 |
|---|---|---|
| 固体 | 可挖掘/酸蚀/熔化/爆破；`durability`(0–14) + `hardness` 决定哪种手段能破坏 | 地形破坏分层：低级炸药挖不动硬岩；"射碎冰砸死下方敌人"是正式战术 [官方] |
| 粉末 | 重力下落、堆叠；**爆炸把命中的固体像素转为 collapsing sand**，坑沿继续塌方 [官方] | 破坏有"二段余韵"，不是一次性删像素 |
| 液体 | 下落→斜下→横流汇池；**按密度分层不混合**（水浮在毒泥上）；蒸发→上方凝结→落回（小水循环）；可装瓶 | 分层 = 玩家可读的安全信息；液体 = 战术消耗品 |
| 气体 | 上浮、天花板下聚成"倒挂湖"；**烟雾使被包围生物窒息** [社区] | 气体有玩法后果，不只是视觉 |
| 火 | 火焰像素 lifetime 极短（flame≈5 帧），必须持续贴燃料才存续 → 火"看起来是活的"；不同材质不同燃速（flammable gas 瞬燃 / oil 中速 / coal 最慢）；着火敌人 **panic 乱跑停止攻击** | 一次点燃产生持续数秒的连锁视觉 + AI 反馈 |

### 1.3 电

金属与液体（尤其水）导电，电流在**整片水体传播**（wiki 名场面：电死圣山水池所有鱼）；对生物施加 Stun；Wet 状态使电更危险；电会震碎 flask、引爆 propane tank [社区]。玩法意义：水从"灭火工具"反转为"导电陷阱"——同一材质双重身份。

### 1.4 染色 Stains 系统（低成本玩法放大器）

液体接触实体即附着 status，**敌我同规则**（个别敌种免疫）[社区]：

| Status | 来源 | 效果 |
|---|---|---|
| Wet | 水/雪水 | 防点燃；电伤害更危险；被火逐渐烘干 |
| Oiled | 油 | **失去地面摩擦（打滑）**、更易燃、燃烧更久 |
| Bloody | 血 | **暴击率提升** + 防点燃 |
| Slimy | 史莱姆 | 减速 15–40% + 防点燃 |
| On Fire! | 火/岩浆 | 2% MaxHP/s + 移速 +15%；浸液即灭 |
| Toxic | 毒泥 | 持续掉血 |
| Frozen | 冰冻液 | 定身；冻结时近战额外 -50% HP |

组合有显式语义：Wet+Oiled = "在火里能点燃，离火立刻熄灭"。Purho 原话："if there's an enemy drenched in oil, you can set the oil on fire to kill the enemy" [官方]。染色把"液体接触史"变成战斗变量——**实现成本只是实体上一个 stain 字段 + 与材质表联动**。

### 1.5 刚体 Props 与尸体

- 确证 props 清单：矿车/独轮车/滑板（带轮可滚）、油灯（受损漏油、毁坏起火）、罐（内装火药或生物）、丙烷罐（受损乱飞最终爆炸）、boulder [社区 wiki]。
- "Each pixel in a rigid body knows that it belongs to that rigid body"；像素被毁 → 重算形状，**body 可被切成多块** [官方]。
- 尸体是持久 ragdoll（材质与生物本体对应），会坠落压坏东西、**腐烂分解成松散像素堆**、可被炸成 gib [官方+社区]。死亡是物理事件，不是消失动画。

### 1.6 打击感构成要素（对本项目最重要的一节）

1. **血是材质不是贴花**——命中喷血是真实液体像素，落地成池、染红实体（还给 Bloody 暴击加成）、战斗痕迹**永久留在地形** [官方]。
2. **尸体是物理实体**——ragdoll 坠落/腐烂/二次破坏。
3. **爆炸三联反馈**——圆坑（永久地形改变）+ collapsing sand 塌方余韵 + screen shake [官方+社区]。
4. **像素↔粒子双态飞溅**——高速激发像素短暂脱网格成自由粒子，碰撞后并回网格（§3.2）。
5. **火的表演性**——随机蔓延 + 烟上涌 + 着火敌人 panic。
6. **染色 = 命中记忆**——浇到什么就"穿"什么，可从外观读出敌人状态。
7. 设计哲学（Purho，Road to the IGF 访谈）："Having a highly-simulated world **feels much richer**... It functions as this **emergence-generating machinery**"；著名轶事：杀死头顶敌人 → 尸体砸碎油灯 → 油泼到自己身上 → 被点燃 [官方]。
8. **反面教训** [社区 HN]：有玩家批评部分武器"did not meaningfully interact with the environmental simulation"——模拟深度不会自动变成战斗深度。**对我们：东方弹幕/技能必须显式消费物理系统**（弹幕点燃油、符卡掀飞地形等）。

涌现实例速记：油浸敌人+火=处决；电击水体连锁全池；炸出的血扑灭自己身上的火（Bloody 防燃）；烟雾灌满密室窒息敌人；电流引爆背包 flask（坑自己）。

### 1.7 性能与规模数字

| 项 | 数值 | 置信度 |
|---|---|---|
| 世界尺寸（主区域） | 35840 × 73728 px | [社区 datamine] |
| 内存驻留 | 12 个 512×512 流式 chunk ≈ 314 万像素 | [社区] |
| 模拟分块 | 64×64 chunk + per-chunk dirty rect | [官方] |
| 多线程 | 4-pass 棋盘格，无锁无原子 | [官方] |
| 单帧像素位移上限 | 32 px | [官方] |
| 帧率 | 官方未给数字；"incredibly CPU sensitive"，液体重灾区在 i7 上也有卡顿报告 | [官方支持口径] |

---

## 2. 核心算法架构（确证版）

### 2.1 更新循环 = 我们已有的骨架

- **单缓冲确证**，且是多线程方案的前提而非妥协：GDC 原话 "There's no two buffers. So you need to make sure the same pixel is not updated by multiple threads" [官方]。
- 受重力材质自底向上扫描、每帧 `frameCount % 2` 交替左右——与 `prototype/core/grid.py:62-64` 完全一致。
- 防重复更新：Noita 用 flag 还是 frame-parity 计数器**未确证**；sandspiel（Rust）用 `clock: u8` 世代计数器（写入时打上当前世代，免每帧清 flag）[社区]——Phase 2 多线程迁移时推荐该方案。
- **静止优化在 chunk/dirty rect 层，不是 per-pixel**："Each chunk keeps a dirty rectangle, containing all the pixels that need to be simulated" [官方]。我们 `FLAG_STATIC`（`prototype/core/cell.py:14`）当前定义未使用——按 Noita 路线，应直接走 per-chunk dirty rect，per-pixel static 可以不做（§5.2 第 8 条）。

### 2.2 材质数据模型真相（materials.xml 挖掘）

[社区 wiki + GitHub dump 交叉验证]

- **`cell_type` 只有 4 种**：`solid` / `liquid` / `fire` / `gas`（默认 liquid）。**没有 powder**——sand 的真实定义是 `cell_type="liquid"` + `liquid_sand="1"`（可堆叠、可站立，粉末走液体代码路径上的开关分支）。
- **大量静态地形是 `liquid` + `liquid_static="1"`** 而非 solid（datamine 中 coal_static 是 liquid）；`solid` 主要留给 box2d 刚体材质。启示：类别越少，代码路径越少。
- 核心属性速览：`density`（int 1–50+：oil=1, lava=6, coal=8, wood_static=11）、`durability`(0–14)、`hardness`、`hp`、`electrical_conductivity`(0–1，非沙液体默认 1)、`lifetime`（仅 liquid/gas）、`wang_color`（世界生成颜色→材质映射，必须唯一）、`tags`（`[burnable]` `[corrodible]` `[meat]` 等供反应表匹配）、`status_effects`（材质上声明施加哪种 stain，如 water→`"WET"`）、`stainable`、`slippery`。
- 分组前缀：`liquid_gravity` / `liquid_flow_speed` / `liquid_sticks_to_ceiling` / `liquid_damping` / `liquid_viscosity`（社区实测"无明显差异"，疑似半废弃）/ `liquid_stains`(0–4)；`solid_friction` / `solid_restitution` / `solid_gravity_scale`；`gas_speed=50` / `gas_upwards_speed=100` 等。
- **继承机制**：`<CellDataChild _parent="父材质">` 覆写部分属性其余全继承；另有占位材质 `"unknown"` 容错（反应引用不存在材质时静默失败不崩溃）。
- **值得抄进我们 TOML**：`inherit` 字段（对应 `_parent`）、`status_effects` 声明在材质上、`wang_color`（Phase 2 关卡生成用）、unknown 容错。我们 `prototype/core/material.py:32-61` 的 tag 索引设计与 Noita tag 体系同构，方向正确。

### 2.3 火的真实机制：没有温度场（已二次验证）

开发者原话（2026-06-06 已对 80.lv 原文逐字核验命中）：

> "When something is on fire it will look in a **random direction** to see if it can ignite that pixel." — Petri Purho, 80.lv

"无温度场"的证据链。注意：网上流传的引语 "temperature is not part of this simulation" 经核查**并不存在**于其声称出处（80.lv），疑为搜索摘要碎片，已从本报告移除；结论改由以下硬证据支撑：

- 80.lv 全文核查：除上句点燃机制外，**零次**提及 temperature / 热模拟；
- materials.xml 原始数据核查（vexx32/noita-data dump，369KB 游戏真实数据）：全文件**只存在两个**温度相关属性——`autoignition_temperature`（×97，取值 0–99 的材质静态常量）与 `temperature_of_fire`（×198，静态常量），**不存在任何**动态温度 / heat / thermal 字段；
- lava 点火走反应表实锤：dump 第 14569 行 `input: [lava]+[burnable] → output: [lava]+fire`。

机制拆解：

1. **无逐像素动态温度、无热传导 pass**。`autoignition_temperature`(0–99) 与 `temperature_of_fire`(0–200) 都是**材质静态常量**：燃烧源每帧随机选方向采样邻居，若 `源.temperature_of_fire ≥ 邻居.autoignition_temperature` 则按概率点燃 [点燃公式细节为社区共识]。自洽锚点：nest 的 autoignition=85 → fire（温度 100）能点燃它，flame（温度 60）不能。
2. **fire 既是材质也是状态**：`cell_type="fire"` 的像素材质（fire / flame，flame 带 lifetime=5）占格存在；同时燃料像素带 burning 状态、实体带 "On Fire!" status。
3. **燃料消耗走 `fire_hp`**（-1 到 99999999，**-1 = 永燃**），耗尽后按 `on_fire_convert_to_material` 转化。
4. **烟参数化**：`generates_smoke`(0–20) 概率生成 `on_fire_smoke_material`（默认 smoke）；`generates_flames` 决定喷出 flame 像素量。fire 自己不冒烟（=0），燃料冒。
5. **连 lava 都不走温度**：lava 点燃可燃物是反应表条目 `[lava] + [burnable] → [lava] + fire`（已在 dump 第 14569 行逐字核验）；lava 固化也是接触反应（+water/+blood/+cement），**没有"随时间冷却"** [社区+数据核验]。这是"无温度场"的最强佐证。
6. 灭火：液体接触 + WET stain 即时灭；`requires_oxygen` 断氧窒息火。

对我们 fire spec 的完整裁决见 §5.3。

### 2.4 反应系统数据格式

[社区 wiki] Noita 的反应是 materials.xml 里与材质平级的 `<Reaction>` 节点：

| 字段 | 语义 |
|---|---|
| `input_cell1/2` `output_cell1/2` | 材质名或 tag（`[water]` `[corrodible]` `[any_liquid]`） |
| `input_cell3` `output_cell3` | 可选**三元反应**（例：flummoxium + blood + oil → 3× unstable polymorphine, rate 35） |
| `probability` | 0–100，即反应速率 |
| `fast_reaction` | 与竞争反应冲突时优先 |
| `blob_radius1/2` | 输出扩散半径（1 像素输入 → 一团输出） |
| `convert_all` | 接触面上的 input1 全部转化 |
| `direction` | none/top/bottom/left/right 方向性反应 |
| `entity` / `<ExplosionConfig>` | 反应点生成实体 / 触发爆炸 |

内建 tag：`[any_liquid]`（liquid 且非 sand）、`[any_powder]`（liquid_sand=1）。例子：lava+water→rock+steam (80)；acid+[corrodible]→acid+flammable_gas（**酸溶解 = 纯反应表，没有专用系统**）；draught of midas+wood→midas+gold (100)。

**对照我们 `prototype/core/reaction.py:18-59`**：tag 展开 + 概率 + 对称注册的设计与 Noita 同构 ✅。缺的修饰符（按需要补）：三元输入、`blob_radius`、`convert_all`、`direction`、`fast_reaction`、反应触发爆炸/实体。

---

## 3. 超越朴素 CA 的扩展（重点：加速运动）

> 回答"比朴素 falling sand 自动机多了什么"。按对我们的落地优先级排序。

### 3.1 速度/重力积分 ★ 最大的单笔视觉提升

**[通用，多实现交叉验证；Noita 用了多格移动为官方确证]**

朴素 CA 一切 1 格/帧 → 下落匀速、飘忽、没有"砸落感"。扩展：每个运动 cell 存 velocity，每帧 `velocity += gravity`（clamp 到 maxSpeed），本帧执行多次单格移动。

jason.today《Improved Falling Sand》参数与代码（逐字引用）：`maxSpeed = 8`、`acceleration = 0.4`、初始 velocity = 0：

```javascript
updateVelocity() {
  let v = this.velocity + this.acceleration;
  this.velocity = Math.abs(v) > this.maxSpeed ? Math.sign(v) * this.maxSpeed : v;
}
getUpdateCount() {  // 小数部分概率取整 = 零成本子像素精度
  const abs = Math.abs(this.velocity);
  const floored = Math.floor(abs);
  return floored + (Math.random() < (abs - floored) ? 1 : 0);
}
// 主循环：一帧跑多次单格更新
for (let v = 0; v < particle.getUpdateCount(); v++) {
  const newIndex = this.updatePixel(index);
  if (newIndex !== index) index = newIndex;
  else { particle.resetVelocity(); break; }  // 撞障碍：停在撞点前，速度清零
}
```

**路径遍历三流派**：

1. **逐步 step 循环**（上面）：一帧 N 格 = N 次完整单格规则判定（含斜下分支），天然处理途中撞墙。最简单，**Phase 1 推荐**。
2. **插值线 / Bresenham**（winter.dev 路线）：cell 存 `float dx, dy`，沿插值线逐点检查停在首个占用格前；移动收集进 change list，同一目标格竞争者随机取一。原文已 404，代码见 `github.com/IainWinter/IwEngine` [未确证细节]。Phase 2 二维速度时用。
3. **连续坐标粒子**（The Powder Toy 路线）：float 位置+速度映射回网格——架构上已非纯 CA，不走。

**Noita 侧锚点** [官方]："We guarantee that no pixel can be moved more than 32 pixels away"——单帧位移上限 32px，反向证明 Noita 有速度积分（等效终端速度），且该上限是多线程安全的前提（§4.1）。

成本：低-中（cell 已有 VELOCITY 字段位，改语义 + 外层 for 循环）；代价是活跃像素更新次数最多 ×maxSpeed，需配合 dirty rect。视觉收益：加速下落、瀑布冲击、高差打击感。

### 3.2 CA↔粒子双轨 ★ Noita 打击感的标志性机制

**[存在性与用途官方确证；触发/写回细节为社区逆向]**

高速/被扰动像素**脱离网格**成为独立弹道粒子（float 位置 + velocity + gravity，抛物线飞行，不参与 CA 规则），碰到占用格时把材质**写回**最近空格，重新变回 CA 像素。

GDC 原话（macuyiko 转述）："When the player jumps in the blood, it takes the surrounding pixels and **puts them in a particle simulation with velocity and gravity**."

```
on_displace(cell, impulse):            # 爆炸/实体高速穿过/排开液体时
    grid.remove(cell)
    particles.add({pos, vel: impulse, material})

update_particle(p):
    p.vel.y += gravity
    for point in line(p.pos, p.pos + p.vel):   # 逐点防穿透
        if grid[point] occupied:
            grid.write_nearest_empty(point, p.material)
            particles.remove(p); return
    p.pos += p.vel
```

触发阈值未确证；写回点被占时向上找空格（防材质损失/复制是主要坑）。靠它实现：水花、血液喷溅、爆炸碎屑、岩浆飞溅——**所有"违反 CA 一帧一格节奏"的高速视觉**。对"东方横版动作 + 打击感"目标，这是优先级最高的视觉投资。成本：中（数组 + 抛物线 + 线段碰撞，2–3 天）。

### 3.3 液体 dispersion rate（横向一帧多格）

**[通用；Noita 参数未确证]**

液体横移沿当前方向最多探测 `dispersionRate` 格，移到**最远可达空格**：

```
dir = cell.lastvel                 # 方向记忆：先试上次方向
for i in 1..dispersionRate:
    if grid[x + dir*i, y] occupied: break
    furthest = x + dir*i
move(cell, furthest) or flip dir
```

参数：水 ≈5、油更低、岩浆 1–2（粘稠）。我们**已有方向记忆**（`prototype/core/rules.py:69-77` 用 VELOCITY 字段存 ±1），只差多格探测的 for 循环——**成本最低、建议最先落地**。解决"水面抖动 + 摊平极慢"两大顽疾，且粘稠度差异（水 vs 岩浆）一个参数免费拿到。

### 3.4 粉末 inertia / free-falling 标记

**[通用，FSS 等社区方案；Noita 官方确证的静止优化在 chunk 层]**

每个 powder cell 加 `is_free_falling`：静止堆体**不参与斜下滑动判定**，只有被扰动（邻居移动/下方变空）才唤醒；落地静止时按 `inertial_resistance ∈ [0,1]` 概率唤醒左右邻居（沙 ≈0.1 易塌、土 ≈0.5、湿沙 ≈0.9 立陡壁）。

收益：堆体稳定不抖 + 爆炸/挖掘后链式塌方 + 不同粉末"安息角"差异。坑：漏唤醒 → 悬空沙。与 dirty rect 互补不冲突。

### 3.5 密度交换的速率控制

**[通用/推测]** 我们已有密度交换（`prototype/core/rules.py:31-43`），缺速率控制防"一帧穿透整池"：① 每帧按 `p = k·(ρ_heavy − ρ_light)` 概率才交换（密度差越大沉越快，典型 0.2–0.7）；② 交换后竖直 velocity 清零或折减（等效水的拖拽，与 §3.1 衔接）；③ 交换时 ±1 随机横移，下沉轨迹不笔直。成本极低。

### 3.6 杂项运动质量技巧速记

| 技巧 | 机制 | 置信度 |
|---|---|---|
| 子像素精度 | A) 概率取整（§3.1，零存储）；B) float 累加器攒满 1.0 才动 | A [官方教程] / B [通用] |
| slide chance | powder 满足斜下条件仅按概率滑（沙 0.9 / 雪灰 0.3）——免费的材质"脾气" | [通用] |
| 方向记忆 lastvel | 我们已有（`rules.py:69-77`）✅ | [社区确证] |
| 粘性液体 | 低 dispersion + 低移动概率即可，岩浆=粘+发光 | [通用] |
| 表面张力 hack | 孤立单格水滴禁横铺（邻居 <N 只准竖直），避免 1px 水膜铺满地面 | [通用/推测] |
| gas 抖动 | 上升随机横移 + 随机跳帧，烟雾自然飘散 | [通用] |

---

## 4. 工程层：并行 / 刚体 / 世界流式（Phase 2 素材，后置）

### 4.1 多线程核验（对 `docs/algorithms/parallel-update-strategies.md` 的修正）

| 既有表述 | 判定 | 精确事实 |
|---|---|---|
| 64×64 chunk | ✅ | 双源确认 [官方] |
| 4-pass 棋盘格 | ✅ | "we do this 4 times... every other 64×64 chunk" [官方] |
| "像素移动 ≤32px 故无竞争"（`parallel-update-strategies.md:39`） | ✅ 本质正确，**建议精化** | 写域 = "本 chunk 64×64 **+ 四个正方向各 32px 的十字**"（不含对角）；同 pass 被选 chunk 边距 64px，两侧 32+32 恰好相接不重叠 → 无锁无原子 [官方原话 "plus 32 pixels in each cardinal direction"] |
| Margolus 方案（`parallel-update-strategies.md:81`） | ⚠️ 保留但应注明 | **非 Noita 实践**——是论文/GPU 路线（GelamiSalami 等），作为 Phase 2+ 备选定位正确 |

线程池规格、dirty rect 扩张规则（移动时是否把 ±1 邻域并入下一帧 rect、双 rect 轮换）各来源均无 → 自行设计，社区共识做法：写像素时把该点 ±1 并入 next rect，帧末交换清空。

### 4.2 刚体桥接管线（Phase 2）

[社区，FallingSandSurvival（FSS）作者直述，自称基于 GDC talk 5:42–9:17]

```
body 像素位图 → Marching Squares（多轮廓，含洞）
            → Douglas-Peucker 简化（实测 284 三角形 → 85）
            → PolyPartition 三角化 → Box2D body（三角形 fixtures）
```

- 三角化坑位：`Triangulate_EC`（ear clipping）不自动识别洞，须先 `SetHole(true)` + `RemoveHoles()`；`Triangulate_MONO` 自动处理洞但三角形冗余。
- **每帧顺序**（FSS v1）：tick 开始把 body 像素**写入网格（solid）** → 跑落沙模拟（沙自然堆在 body 上）→ tick 末**擦掉** body 像素 → 位移交互 → Box2D step → 下帧按新 transform 重新光栅化。
- 重算是 **on-demand**（body 内像素被毁才重跑管线），无固定周期；Marching Squares 多轮廓输出天然支持 body 分裂 [官方确证 Noita 行为]。
- **已知坑**：body 旋转后写入/擦除坐标不一致 → 洞/复制像素。FSS v1 hack：±1px 容差匹配（"修了大部分"）；**FSS v2（Rust）更优方案：根本不写入网格**——对 body 内每像素按变换后坐标调 `simulate_pixel()` 回调，网格查询函数感知 body 像素。永不丢/复制像素，渲染用每 body 一张随动 image。
- 静态地形侧：solid 像素同管线生成 world mesh 喂 Box2D，**按 chunk 缓存、只在刚体附近的 chunk 生成**（world mesh 不用于玩家碰撞）。
- 反面教材（slowrush.dev）：跳过 DP 简化 → shape 生成成为性能瓶颈；水做成刚体 → 灾难。**DP 不可省；液体不进刚体管线**。
- 排开与浮力：tick 末遍历 body 像素世界坐标，遇沙/液体 → 该像素转入粒子系统（飞溅）+ 按 `velocity_at_point` 施加反向力和阻尼——**浮力是逐像素反作用力的涌现，没有显式阿基米德公式** [社区]。

### 4.3 角色控制（Phase 2/3 直接采用）

[社区，FSS 作者直述引 Noita] 四层模拟分类：**Sand（网格）/ Particles（弹道像素）/ Rigidbodies（Box2D）/ Entities（矩形 hitbox，无旋转）**。

> "the player and enemies (entities) can stand on sand, but boxes and other physics objects (rigidbodies) fall into it. ... in Noita and in my game player collision is **pixel-perfect and is a completely separate system**"

玩家 = 自定义 kinematic：手动积分速度 → 矩形碰撞盒逐像素查询网格 → 碰 solid/sand 即停（可顺带排开少量沙成粒子）。FSS 玩家另挂一个影子刚体**只**用于与 props 互推。

### 4.4 世界流式与持久化（Phase 2+）

- **双层 chunk**：模拟 64×64；**流式/落盘 512×512**（内存驻留 12 个）[社区]。
- 落盘：`save00/world/world_X_Y.png_petri`（自定义压缩 png 变体）+ `entities_NUM.bin`；实现 "everything stays where you left it" [官方+社区]。
- 程序化生成：Herringbone Wang tiles 拼预制件消除接缝感，预制件内敌人/道具 50% 概率放置 [官方]；biome 布局由 biome map png 驱动 [社区常识，半确证]。
- 摄像机周围模拟区域的确切屏数：**未确证**（建议不要在文档里写死）。

---

## 5. prototype 现状对照（gap 分析）

### 5.1 已对齐项 ✅

| 项 | 锚点 | 对照结论 |
|---|---|---|
| 更新循环骨架 | `prototype/core/grid.py:54-96` | 与 Noita 结构一致（单缓冲/自底向上/交替/防重复） |
| 反应表设计 | `prototype/core/reaction.py:18-59` | tag 展开 + 概率 + 对称注册，与 Noita `<Reaction>` 同构 |
| 液体方向记忆 | `prototype/core/rules.py:69-77` | 即 lastvel 技巧，已有 |
| 数据驱动材质 | `prototype/data/materials.toml` + `material.py:32-61` | tag 体系与 Noita 同路线 |

### 5.2 Gap 清单（按打击感/视觉收益排序）

| # | Gap | 现状 | Noita/参考对应 | 优先级 | 预估 |
|---|---|---|---|---|---|
| 1 | 速度/重力积分 | 全部 1 格/帧（`rules.py:46-110`） | §3.1，32px 上限 | **P0** | 1 天 |
| 2 | 液体 dispersion rate | 横移 1 格/帧（`rules.py:71-74`） | §3.3 | **P0** | 半天 |
| 3 | CA↔粒子双轨 | 无 | §3.2 | **P0** | 2–3 天 |
| 4 | 火系统落地方式 | spec 已写未实施，需按 §5.3 裁决修订 | §2.3 | **P1** | 修订半天 + 实施 |
| 5 | 粉末 inertia | 无（堆体每帧重算） | §3.4 | P1 | 1 天 |
| 6 | 密度交换速率控制 | 即时交换（`rules.py:31-43`） | §3.5 | P1 | 半天 |
| 7 | 爆炸（径向破坏 + collapsing sand + 粒子弹射） | 完全没有 | §1.2 / §3.2 | P1 | 2 天 |
| 8 | chunk + dirty rect | `FLAG_STATIC` 定义未用（`cell.py:14`） | §2.1：走 per-chunk dirty rect，**不做 per-pixel static** | P2 | benchmark 后 |
| 9 | stains / `status_effects` 字段 | 无 | §1.4 | P3（Phase 3 游戏层；TOML 可先留字段） | — |

### 5.3 对 fire spec 的裁决建议

对照 `docs/superpowers/specs/2026-05-26-fire-system-design.md`：

| spec 设计 | Noita 对照 | 结论 |
|---|---|---|
| `fire_hp` 燃烧消耗 | `fire_hp`（-1 永燃） | ✅ **完全一致，连名字都一样**；建议补 -1 永燃语义 |
| `requires_oxygen` 表面燃烧 | `requires_oxygen` | ✅ 一致 |
| `burn_to` 燃尽转化 | `on_fire_convert_to_material` | ✅ 一致；建议加抄 `on_fire_smoke_material` + `generates_smoke`（烟参数化） |
| `autoignition_temp` | `autoignition_temperature` | ⚠️ 属性同名但语义不同：Noita 是与火源 `temperature_of_fire` 的**静态比较**，不是动态温度过阈值 |
| **每像素温度场 + 热传导 pass** | **不存在**（§2.3 已二次验证） | ❌ 自创设计，风险见下 |

温度场方案的风险：

1. **与休眠优化正面冲突（最大风险）**：热传导是全场 diffusion，温度梯度永远在变 → 像素永不静止、chunk 永不休眠，直接对冲 CLAUDE.md §5.3 优先级 1–3 的全部优化。Noita 方案天然兼容 dirty rect。
2. **调参面大**：传导率 × 阈值 × 消耗三参数耦合，比 Noita 单概率难调（扩散慢 → 蔓延肉眼可见地滞后；快 → 全图烧）。
3. **内存**：每像素多一个通道（Noita 为 0）。

**建议**：spec 修订为 **Noita 式优先**——保留 fire_hp / requires_oxygen / burn_to 三件套（已与 Noita 一致），点燃改为"火源 `temperature_of_fire` vs 邻居 `autoignition_temp` 静态比较 + 随机方向采样 + 概率"；温度场+传导**降级为可选实验分支**，仅当需要"岩浆随时间冷却 / 热水沸腾"这类 Noita 做不到的差异化效果时再开，且必须先过 benchmark + 设计好休眠条件（温差 < ε 即夹断停更）。

另注：CLAUDE.md §5.2 的示例反应 `(Lava, t>300) → [Rock]`（按温度冷却）在 Noita 中不存在——lava 固化全部由接触反应触发。我们仍可以做，但要明确这是超出 Noita 的自选动作。

---

## 6. Phase 1 行动队列建议（按性价比）

> **过时标注（2026-06-06）**：本队列的优先顺序已被 `docs/proposals/2026-06-06-deterministic-parallel-and-netcode.md` §5/§7 取代——现行顺序为 **M0 → M0.5 → 本队列**。本节保留作各项收益的论证依据。

1. **液体 dispersionRate**（半天）——成本最低收益立竿见影
2. **竖直 velocity + 重力积分**（1 天，jason.today 方案）——"加速运动"本体
3. **fire spec 修订为 Noita 式**（半天）→ 实施火系统
4. **粉末 free-falling + inertial_resistance**（1 天）
5. **最小粒子双轨 + 简单爆炸**（2–3 天）→ 打击感验证里程碑：「炸开沙墙、血/水溅上墙再流下来」demo
6. **benchmark + per-chunk dirty rect**（量化以上各项的性能账，按 CLAUDE.md §5.3 落 `docs/perf/`）

每步完成落 `docs/CHANGELOG.md`；新材质/规则跑 benchmark。

---

## 7. 来源索引

**官方/开发者直述**

| 来源 | 内容 |
|---|---|
| [GDC 2019: Exploring the Tech and Design of Noita](https://www.youtube.com/watch?v=prXuyMCgbTc)（[GDC Vault](https://www.gdcvault.com/play/1025695/Exploring-the-Tech-and-Design)） | 核心架构；刚体段 5:42–9:17；粒子演示约 23–30 min |
| [80.lv 访谈](https://80.lv/articles/noita-a-game-based-on-falling-sand-simulation) | 火机制原话、4-pass 十字写域原话、collapsing sand、Wang tiles |
| [Road to the IGF 访谈](https://www.gamedeveloper.com/game-platforms/road-to-the-igf-nolla-games-i-noita-i-) | game feel 设计哲学引语 |
| [Steam 页](https://store.steampowered.com/app/881100/Noita/) / [官网](https://noitagame.com/) | 官方宣称 |

**社区 wiki / datamine**

| 来源 | 内容 |
|---|---|
| [noita.wiki.gg: Modding materials](https://noita.wiki.gg/wiki/Modding:_Making_a_custom_material) | CellData 全属性表（§2.2 主来源） |
| [noita.wiki.gg: Reaction 文档](https://noita.wiki.gg/wiki/Documentation:_Reaction) | 反应格式（§2.4 主来源） |
| [vexx32/noita-data materials.xml dump](https://raw.githubusercontent.com/vexx32/noita-data/main/materials.xml) | 原始数值锚点（fire/flame/nest/coal） |
| noita.wiki.gg: [Fire](https://noita.wiki.gg/wiki/Fire) / [Stains](https://noita.wiki.gg/wiki/Stains) / [Materials](https://noita.wiki.gg/wiki/Materials) / [Props](https://noita.wiki.gg/wiki/Props) / [Save](https://noita.wiki.gg/wiki/Save) | gameplay 层 + 存档结构 |
| [Dadido3/noita-mapcap](https://github.com/Dadido3/noita-mapcap/blob/master/AREAS.md) | 世界尺寸 datamine |
| [HN: every pixel simulated](https://news.ycombinator.com/item?id=24394259) | 官方 FAQ 引用 + 武器/模拟脱节批评 |

**开源复刻 / 教程**

| 来源 | 内容 |
|---|---|
| [macuyiko Part 4](https://blog.macuyiko.com/post/2020/an-exploration-of-cellular-automata-and-graph-based-game-systems-part-4.html) | GDC 逐段复盘 + 概念复现（注意：是概念教程，非 Noita 逆向——CLAUDE.md 索引对它的描述略高估） |
| [jason.today/falling-sand](https://jason.today/falling-sand) / [falling-improved](https://jason.today/falling-improved) | 速度积分逐字代码（§3.1 主来源） |
| [braindump GDC 笔记](https://braindump.jethro.dev/posts/gdc_vault_exploring_the_tech_and_design_of_noita/) | 64×64 + per-chunk dirty rect |
| [FallingSandSurvival](https://github.com/PieKing1215/FallingSandSurvival)（[Issue #3](https://github.com/PieKing1215/FallingSandSurvival/issues/3) / [#4](https://github.com/PieKing1215/FallingSandSurvival/issues/4)、[Tech wiki](https://github.com/PieKing1215/FallingSandSurvival/wiki/Tech-&-Integrations)） | 刚体桥接完整管线 + 角色控制 + v2 改进（§4.2–4.3 主来源） |
| [FallingSandJava](https://github.com/DavidMcLaughlin208/FallingSandJava) | 同管线独立佐证 |
| [slowrush.dev: Bridging Physics Worlds](https://www.slowrush.dev/news/bridging-physics-worlds/) | 桥接踩坑（DP 不可省） |
| [sandspiel](https://github.com/MaxBittker/sandspiel) | clock 世代计数器参考（`crate/src/universe.rs`） |
| winter.dev "Making Games with Falling Sand" | ⚠️ 原文 404，查 Wayback 或 [IwEngine](https://github.com/IainWinter/IwEngine)；插值移动 + change list 的最佳参考 |

**抽查验证记录（2026-06-06，主会话对一手来源直接核字）**

- ✅ **80.lv**：random direction 点燃、"plus 32 pixels in each cardinal direction" + "4 times"、collapsing sand "good candidates"、刚体像素归属与 body 分裂、per-chunk dirty rect——全部逐字命中。
- ✅ **macuyiko Part 4**："There's no two buffers"、"We guarantee that no pixel can be moved more than 32 pixels away"、跳血泊→粒子模拟——逐字命中；文章性质确认为"前半 GDC 转述 + 后半作者自做概念教程"。
- ✅ **jason.today/falling-improved**：`maxSpeed=8`、`acceleration=0.4`、`getUpdateCount()` 概率取整、受阻 `resetVelocity` 循环——逐字命中。
- ✅ **FSS Issue #3/#4**：MS→DP→PolyPartition 管线、world mesh 不用于玩家碰撞（"in Noita and in my game player collision is pixel-perfect and is a completely separate system"）、tick 写入/擦除顺序、`velocity_at_point` 阻尼、v2 `simulate_pixel` 方案、"the player and enemies (entities) can stand on sand"——逐字命中。
- ✅ **materials.xml dump 直查**：cell_type 仅 4 种取值（fire×4 / gas×15 / liquid×147 / solid×42，按显式标注计，children 继承不计入）、sand = `cell_type="liquid"` + `liquid_sand="1"`、`fire_hp="-1"` 存在（×4）、`CellDataChild` ×478、温度相关属性仅 2 个静态常量字段。
- ❌ **删除 1 条伪引语**："temperature is not part of this simulation" 在其声称出处（80.lv）中不存在，疑为搜索摘要碎片混入；"无温度场"结论不变，但已改为结构证据支撑（§2.3）。
- ⚠️ braindump.jethro.dev 笔记本轮抓取失败（socket error）；其 64×64 chunk + dirty rect 表述已由 80.lv 原文独立双重确认。
- 未复核（维持 wiki 级置信度，均非决策承重项）：世界尺寸 datamine、12×512² reality bubble、stains 效果数值表、props 清单细节、材质总数 ~400+、fire/flame lifetime 帧数。

**主要未确证项清单**：Noita 防重复更新具体机制（flag vs parity）；粒子弹出精确阈值；点燃概率公式精确形式；电的传播算法；线程池规格；dirty rect 扩张/唤醒规则；模拟区域屏数；Noita 浮力实现；三角化具体算法。
