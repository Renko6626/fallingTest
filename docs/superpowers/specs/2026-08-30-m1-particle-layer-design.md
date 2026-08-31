# M1 粒子层（Layer P）设计

> 文档路径：`docs/superpowers/specs/2026-08-30-m1-particle-layer-design.md`
> 运行时版本：Rust（内核）+ Godot 4 + gdext（表现层）
> 最近更新：2026-08-30 (UTC+8)
> **Status**: Implemented（Task 1–7 全部完成，验收标准 §0 五项全过，见 `docs/sessions/2026-08-30-m1-particle-layer.md`）

对应总纲里程碑 **M1 粒子层**（`docs/overview/kernel-charter.md:126`）：脱格/落格闭环、
DDA 碰撞、容量限流；验收 = 瀑布与爆炸溅射场景 SyncTest 绿。Layer P 硬约束见
`kernel-charter.md:60-65`，管线位置见 `program-architecture.md:125`（第 5 步）。

---

## 0. 验收标准

1. `cargo test` 全绿（含新增单测/行为测试，见 §10）。
2. SyncTest：`waterfall.ron` 与 `explosion_splash.ron` 各 2 万 tick（翻案 5 口径）
   × {1, N 线程} × {Full, ChunkSleep, LiveRect} 六配置零分叉。
3. golden replay：旧 golden 按 §9 程序重录后全绿，新增粒子场景 golden ×2 入库。
4. render GIF 目检：瀑布喷射-落格-堆积-摊平；爆炸挖坑-溅射-回落；薄墙后完好（遮挡）。
5. bench：粒子压测对照总纲 §7 预算（2 万粒子 ≈ 0.8ms），结果落 `docs/perf/`。

## 1. 范围裁决（2026-08-30，用户批准）

**M1 走最小闭环**：粒子来源 = 显式事件（`Op::Explode` + `Op::Emit`），Layer G 语义
零改动。要点：

- **Layer G 速度积分后置为独立提案**（格内移速 ≤4、超限自然脱格——总纲 §4 原文语义）。
  它直接顶在 r≤16 并行安全论证上，须单独立项、过总纲 §11、跑 SyncTest。**分期实施，
  非翻案**。Noita 实为"网格速度积分 + 脱格粒子"双系统（官方 32px/帧上限
  `docs/reference/noita-deep-dive.md:200`；GDC 原话同文 208-210），此债记账不弃账。
- **O3 粉末惯性不入 M1**（一次只动一个语义层，M0 tick-583 教训），时点由 M2 spec 裁决。
- **接口不吃亏**：粒子生成队列是白名单通信介质（`program-architecture.md:132`），
  将来 rules.rs 超速脱格只是多一个生产者，粒子层零改动。

## 2. 定点基建 `fixed.rs`

**用户裁决（2026-08-30）**：粒子层用定点，维持总纲 §6 混合数值制原判。同二进制浮点
的四个雷区（系统 libm CPU 分派、ucrtbase 随 Windows 版本漂移、MXCSR 每线程状态、
依赖库运行时 SIMD 分派）中，最实际的一条是**开发 Linux / 产品 Windows 是两份二进制**
——定点让 golden 跨平台成立。物理引擎（M3）照总纲仍走受控浮点，不受本裁决影响。

实现：手写极简 newtype，不上 fixed crate、不搬 STG 代码（M1 运算面太小，自家可审
可测；日后要换，newtype 边界不动）。

```rust
/// Q16.16 定点。范围 ±32768 格，1280×768 世界余量充足。
pub struct Fx(pub i32);
```

运算面（全部配金值单测）：`add/sub/neg`、`mul_int(i32)`、`mul(Fx)`（i64 中间量
`>> 16`）、`from_int` / `from_ratio(num, den)`、`to_cell()`（floor 至格坐标 i32）、
`isqrt(u64) -> u32`（爆炸径向归一）。除法只有 `from_ratio` 常量构造一处，无运行时
除法链。**无三角函数**——M1 全部方向量走分量运算 + isqrt 归一，BAM 角度资产留 M4。

常量（调参项，初值）：

| 常量 | 初值 | 说明 |
|---|---|---|
| `GRAVITY` | 0.25 格/tick² | `Fx::from_ratio(1, 4)` |
| `MAX_SPEED` | 16 格/tick | 逐轴 clamp；DDA 单 tick 步数上界 |
| `MAX_PARTICLES` | 65536 | 总纲初值 64k（`kernel-charter.md:64`） |

## 3. 粒子数据模型（SoA）

`particle.rs`。SoA 布局为架构 §3 state 条目既定（`program-architecture.md:74`）：

```rust
pub struct Particles {
    x: Vec<Fx>, y: Vec<Fx>,
    vx: Vec<Fx>, vy: Vec<Fx>,
    material: Vec<u8>,
    next_id: u32,   // 单调计数，入状态哈希；不做索引
}
```

- **顺序即 id 序**：生成按入队序 append；移除用保序压缩（`retain` 语义）。
  "串行按 id 提交"= 按下标顺序遍历，无需排序。
- **容量限流**：生成队列按入队序处理，`len == MAX_PARTICLES` 时确定性拒绝
  （丢弃 + 计数器 `rejected_total`，计数器不入哈希、只供诊断；M1 不发事件）。
- **无 lifetime 字段**：重力保证要么落格要么出界；出界即确定性销毁。
- 法术 payload（M4）为扩展点，现在不留字段（见 §12）。

## 4. tick 集成与并行模型

管线插入第 5 步（`program-architecture.md:125`），既有阶段顺序不动——本变更**新增**
阶段而非重排，仍属协议版本变更，随本 spec 过 §11 决策日志：

1. **Ops 应用**（现有位置）：`Explode` / `Emit` 在此把粒子压入生成队列。
2. **网格四相 pass**：现状不动（M1 无 rules.rs 生产者）。
3. **粒子相**（`particle.rs` + `lib.rs::step`）：
   - **a. 生成**：drain 队列按入队序 append，执行容量拒绝；
   - **b. 并行积分**（rayon par_iter，与调度器同池）：每粒子纯函数——
     `vy += GRAVITY` → 逐轴 clamp → 对**本 tick 网格终态只读快照**做 DDA →
     产出 `Outcome { Land(格), Fly(新 pos), Gone }`。粒子间零交互、网格只读 →
     任意线程数/调度同结果（P4 论证与四相同型且更简单）；
   - **c. 串行提交**（按下标 = id 序）：落格/冲突消解（§5），写入走 `world.rs`
     既有路径（脏矩形合并 + chunk 唤醒复用）；
   - **d. 保序压缩**：移除 Land/Gone 者。
4. 封帧哈希含粒子层（§9）。

**实际执行位置说明**（终审观察，`crates/sand-core/src/lib.rs:109` `Sim::step`）：
上面的步骤编号是管线**语义**顺序，但代码里粒子相（3.a–3.d，`particle::advance`）
调用点在 `scheduler::step` **之后**——即网格四相跑完、`tick += 1` 已自增、且
`chunk.dirty = chunk.next_dirty.take()` 的脏矩形交换已发生（`scheduler.rs:103,105`）
之后才执行。因此粒子落格时对 `next_dirty` 的标记（`world.rs` 落格写入路径，
经 `set_cell_stamped`）落在**已经清空过的**下一轮 `next_dirty` 里，要到下一个
tick 的 `scheduler::step` 才会被合并进 `dirty` 生效——粒子落格唤醒周边网格格子
天然带一 tick 延迟，不是本 tick 内网格四相能看见的。这一时序是既定语义、
不是 bug：六套配置的 SyncTest（M1 Task 3/4/6 commit）逐 tick 哈希比对已覆盖
并锁定这一行为。~~M2 插入场层（pull 场读网格/粒子状态）时~~（**订正 2026-08-31：Layer F 已删除，见总纲 §11 翻案记录第 6 条；下句的先后关系约束改由 M2 的反应表/燃烧 pass 继承**），此处的先后关系与
一 tick 延迟是既有事实，不要按步骤编号的书写顺序假设"同 tick 生效"。

## 5. DDA 与落格

`dda.rs`（或并入 particle.rs，实现时定）：

- **DDA**：标准整数网格穿越，从 `pos` 到 `pos + vel`，按跨越边界顺序逐格检查；
  边界跨越比较用 i64 交叉相乘，无除法。
- **阻挡判定**：第一个非 air 格即阻挡——wall/sand/water 一视同仁。水粒子落水面
  成水 cell 后摊平交给网格规则；沙粒子停在水面后下沉交给网格密度规则。穿水留
  M2 评估（§12）。
- **无阻挡**：`pos += vel` 继续飞；`to_cell()` 出界 → `Gone`。
- **阻挡**：落格候选 L = 阻挡格前最后一个 air 格。
- **提交期冲突**（DDA 快照中 L 为 air，但更小 id 已占）：
  1. 按固定邻格序搜 L 的邻居：**上、左、右、左上、右上**，取第一个 air 落格；
  2. 五邻格仍全占 → 沿候选格 L **正上方**继续逐格向上搜索，取第一条 air 落格
     （候选本身若是 air，第 0 步就已在前面被接住，此处不重复判）；搜到世界顶
     仍无 air → 视为出界销毁（`Gone`），计入诊断计数器 `buried_total`（不入
     哈希）。此为 Noita 同款方案（`docs/reference/noita-deep-dive.md:226`："写回
     点被占时向上找空格"）。
  3. **不存在"继续飞行/悬浮"这条路**——`Land` 必然终止于"落格"或"出界"两态
     之一，无第三态。
  > **为何改（Task 4 评审 2026-08-30，修复轮 1，C1）**：原设计"全占 → 继续飞
  > （悬浮）：`pos = L 格中心`，速度清零"，在静态堆场景下构成**活锁**——被
  > 重置到 L 格中心的粒子下一 tick 从该格出发（起点格从不检查，语义上允许
  > 站在非 air 格里），若 L 与其五邻格此刻仍是同一批"全占"局面，DDA 会原地
  > 立即再次判定 `Blocked{land_cell = L}`，`resolve_landing` 再次全占，回到
  > 悬浮，两 tick 一个周期无限重复。评审用 40 颗同位同速沙粒复现：32 颗永久
  > 卡死在同一格，粒子池不排空。总纲 `kernel-charter.md:62` 原文"按定序邻格
  > 搜索**或继续飞行**"中的"继续飞行"分支据此判定为设计缺陷而非有效选项，
  > 改为确定终止的向上兜底搜索（有界：最坏情况扫到世界顶，一次性成本，不
  > 会重复触发同一局面）。
- **落格写入**：`Cell::new(material, 当前世代戳)`。

## 6. 爆炸：Noita 射线模型

`Op::Explode { x: i32, y: i32, r: i32, power: u32 }`，ops 阶段应用，整体串行。

查证（[Noita Wiki: Explosion](https://noita.wiki.gg/wiki/Explosion)、
[Explosion interactions](https://noita.wiki.gg/wiki/Explosion_interactions)）：Noita
爆炸 = 爆心向四周发多条射线，每条带 ray energy 预算，沿途按材质消耗，耗尽即停，
另有最大半径与 durability 硬门槛。**遮挡免费涌现**（薄墙后不隔墙爆），坑形随材质
阻力变化。采纳该模型，替代最初口头方案的圆盘扫描：

1. 对半径 r 的 Bresenham 圆周每格，从爆心发一条 DDA 射线（射线按圆周格定序遍历）；
2. 每射线初始能量 = `power`，逐格消耗 `blast_cost`：air 0、water 1、sand 2、
   **wall = ∞**（M1 简化版 durability 免疫；M2 反应表引入 durability/hardness 后替换，
   见 §12）。`blast_cost` 为材料表新字段（RON，缺省 1，wall 用哨兵值）；
3. 能量 ≥ cost 的格子被摧毁：置 air + 入生成队列。粒子速度 = 射线方向单位向量
   （isqrt 归一）× `MAX_SPEED × 剩余能量 / power`（线性衰减，天然 ≤ MAX_SPEED；调参项）
   + hash 抖动
   （`STREAM_EXPLODE`，salt 区分同格多骰）。能量耗尽或撞 wall 断线；
4. 已被前序射线炸掉的格子对后续射线按 air 计费，不重复生成粒子。

此路径同时是"挖坑 + 溅射 + 脱格"三合一，也是 M4 法术命中结算的原型。

### 6.1 近心汽化（`vaporize_threshold`，用户裁决 2026-08-30）

严格质量守恒（每个被摧毁格子都生成一颗粒子）在爆心附近观感不对：近心格子
"应该没了"，而不是原地变成一颗慢速飘散的粒子。裁决：材料表新增每材质字段
`vaporize_threshold`，射线剩余能量比例**严格超过**该阈值时格子直接汽化——
删除、不生成粒子，质量确定性蒸发。

- **数据**：RON 写 `0.0..=1.0` 十进制，缺省 `1.0`（永不汽化）。加载期经
  `sand-harness::scenario::quantize_vaporize_threshold` 一次性 `×255 round`
  量化为 `u8`；负值/超界（round 后落在 `[0,255]` 之外）报错。core
  （`MaterialDef::vaporize_threshold`）只见量化后的整数，不做取值校验
  （同 `blast_cost` 先例）。`data/materials.ron` 初值：`water 0.4`
  （量化 102）、`sand 0.7`（量化 179，沙比水更耐炸）；`air`/`wall` 不写，
  吃缺省 255（`air` 从不进入摧毁分支，`wall` 免疫爆炸，字段本就无意义）。
- **判定**（`world.rs::fire_ray`）：摧毁分支内，`remaining`（`cost` 扣减后
  的剩余能量——与紧随其后的 `speed_ratio` 用的是**同一个变量**，口径钉死
  不做区分）纯整数比较 `remaining*255 > power*threshold` 成立即汽化：置
  air、**不**入生成队列、`World::vaporized_total` 计数 +1。严格大于是关键：
  `threshold=255`（缺省 1.0）时条件退化为 `remaining > power`，而
  `remaining <= power` 恒成立，故缺省材质在任何输入下都不汽化。
- **诊断计数**：`vaporized_total` 挂在 `World`（私有字段 + `pub fn` 访问器），
  仿 `Particles::rejected_total`/`buried_total` 先例——不参与
  `hash::state_hash`（该函数只读 `tick` + `chunks`），不影响 SyncTest 哈希
  比对，但本身仍是（状态,输入）的确定性函数，测试可直接断言。
- **守恒口径变更**：原"挖坑守恒 = 摧毁格数 == 生成粒子数"改为"摧毁格数 ==
  生成粒子数 + 汽化计数"——汽化格既不生成粒子也不算遗漏，质量在此确定性
  蒸发（不返还、不进粒子池）。
- **golden 影响**：`vaporize_threshold` 进材料表 → `data/materials.ron`
  内容哈希变 → 全部 golden 的 `materials_fp` 行重录。`explosion_ci`
  额外因爆炸语义变更导致状态哈希全变（重录）；`sand_pile`/`mixed`/
  `waterfall_ci` 无爆炸，状态哈希逐位不变（重录前 diff 验证，证明改动只
  影响爆炸路径）。

### 6.2 手感迭代四项（用户目检裁决，2026-08-30）

M1 终审后针对 `explosion_splash.ron` 的多轮实机目检，落定四项调参 + 一项涨落
机制（均已过 core 102 项测试 + clippy，实现见 `crates/sand-core/src/explode.rs`
——本轮同时把 §6/§6.1 原先挂在 `world.rs` 的实现全部纯搬移到该文件，逻辑不变，
见 §13 决策记录第 8 条）：

1. **`EXPLODE_SPEED` 16→8**（"粒子更重"裁决）：溅射出射速度上限从
   `Fx::from_int(16)` 降到 `Fx::from_int(8)`——原速度手感偏"轻飘"，目检后
   裁定砍半（`explode.rs::EXPLODE_SPEED`）。与 `particle.rs::MAX_SPEED`
   （飞行 clamp 上限，管数值纪律）解耦，纯手感调参项。
2. **密度冲量缩放**（"冲量物理"裁决）：同一冲量下出射速度应与材质密度成
   反比（`v ∝ 1/density`），而非此前全材质等速。新增参考密度
   `REF_BLAST_DENSITY = 40`（取沙的密度为锚点，沙的缩放系数恒为 1.0，手感
   不动），出射速度额外乘 `mass_factor = REF_BLAST_DENSITY / density(material)`
   （`Fx::from_ratio`，整数除法）；水（密度 16）系数 2.5，会被 `clamp_speed`
   封顶（`explode.rs::fire_ray`，单测 `blast_mass_factor_golden_values`/
   `fire_ray_lighter_material_launches_faster_than_heavier`）。
3. **射线方向涨落**（"完美圆坑不自然"裁决）：此前每条射线严格按
   `power`/`r` 结算，坑形是完美 Bresenham 圆——目检认为不自然。改为每条
   射线独立掷两颗骰：能量涨落 `±25%`（`ray_fluct(power, ., sym=true)`）、
   射程涨落 `−25%..0`（`ray_fluct(r, ., sym=false)`，只缩不涨——`CellWalk`
   终点是圆周格，无法越过延长）；涨落分母 `EXPLODE_FLUCT_DIV = 4`。
   骰子锚点是射线方向 `(dx, dy)`（`circle_offsets` 保证每次 `Op::Explode`
   内唯一），`salt = op_idx` 区分同 tick 多次爆炸。
   为容纳这两颗新骰，`explode_attempt` 编码从 `emit_attempt` 的"高位
   stamp + 低 1 位骰子标号"扩为"高位 stamp + **低 2 位**骰子标号"（4 颗骰：
   vx/vy/ray_power/ray_range）——`Op::Emit`/`Op::Explode` 的 attempt 编码
   各自独立演化，本次扩位只作废爆炸场景的 RNG 序列，不牵连 Emit（瀑布）
   （`explode.rs::explode_attempt` 文档、单测
   `explode_crater_is_not_perfectly_circular`）。
4. **汽化阈值现值调整**：`data/materials.ron` 里 `sand.vaporize_threshold`
   由 0.7 上调到 **0.95**（沙更耐炸，近心汽化圈显著收窄）；`water` 维持
   **0.4** 不变。§6.1 判定逻辑本身未变，只是数据表取值随目检结果迭代。

**场景 tick 口径同步收窄**：`explosion_splash.ron` 从 `ticks: 20000` /
`script Every(from: 500, until: 19500, step: 1000)`（19 炮）改为
`ticks: 8000` / `Every(from: 500, until: 7500, step: 1000)`（8 炮）——
迭代阶段 2 万 tick 单次渲染/回放耗时过长，8000 tick 已足够覆盖"挖坑 + 溅射
+ 脱格 + 水面重新摊平"的完整观察窗口。

## 7. 发射器：`Op::Emit` + 场景 script

core 只认 Op；"发射器"完全由既有场景 script 机制表达，**harness 零新概念**：

```ron
script: [
    Every(from: 0, until: 18000, step: 2,
          op: Emit(material: "water", x: 120.0, y: 8.0,
                   vx: 0.5, vy: 2.0, count: 3, jitter: 0.8)),
]
```

- `Op::Emit { material: u8, x, y, vx, vy: Fx, count: u16, jitter: Fx }`：在 (x,y) 生成
  count 个粒子，初速 (vx,vy)，每粒子速度加 `[-jitter, +jitter]` 抖动
  （`STREAM_EMIT`，salt = 粒子序号 i——翻案 4 同帧多骰纪律）。
- 场景 RON 中速度写十进制小数，**harness 加载期一次性量化为 Q16.16**（round），
  core 边界只见 `Fx`。量化在 I/O 层，不碰核心红线；量化后数值入场景指纹。

## 8. RNG streams

沿用 `rng.rs` 的 `rng_u32(fseed, stream, x, y, salt, attempt)`。新增流常量
`STREAM_EXPLODE`、`STREAM_EMIT`（值接现有编号排下去），调用点显式编码——
charter §11 翻案 4 纪律。

## 9. 状态哈希与 golden 重录程序

- 总哈希 = `combine(网格哈希树根, 粒子层哈希)`；粒子层哈希 = xxh3 按 id 序折叠
  `(x, y, vx, vy, material)` 原始位 + `next_id` + 粒子数。
- **既有 golden 哈希值会变**（哈希格式变更，网格语义未变）。重录程序：
  1. 新代码跑旧 golden 场景，导出**网格层哈希**逐 tick 序列；
  2. 与旧版逐 tick diff，必须一字不差（证明 Layer G 零扰动）；
  3. 通过后才允许重录 golden 终态哈希，验证过程记入 session 文档。

## 10. 测试与场景

**单测**：Fx 金值（含 isqrt 边界）、DDA 穿越序与阻挡语义、邻格搜索定序、
容量拒绝确定性（第 65537 个被拒且两次运行一致）、Emit 抖动 salt 独立性。

**行为测试**（沿用 M0 风格）：粒子落格成堆（守恒：发射 N 个 = 网格新增 N 格）、
爆炸遮挡（薄墙后逐格完好）、爆炸挖坑守恒（炸掉格数 = 生成粒子数，容量内）、
双粒子同格冲突 id 小者胜。

**场景**（`data/scenarios/`）：

| 场景 | 几何 | 用途 |
|---|---|---|
| `waterfall.ron` | 高处 Emit 水 → 盆地，2 万 tick | SyncTest 验收 + golden |
| `explosion_splash.ron` | 沙丘 + 水池 + script 定期 Explode，8000 tick（8 炮） | SyncTest 验收 + golden |
| `particle_stress.ron` | 持续高 rate Emit 顶满 64k | bench 专用 |

**CI SyncTest**：小图版粒子场景并入既有六配置矩阵。

## 11. bench 计划

1. **动工前**：跑既有 harness bench（dense/sparse/sleep + acceptance），落
   `docs/perf/` 为正式 M0+O1 Rust 基线（此前只有 informal 文档）。
2. **完成后**：同场景对照（网格路径无回退）+ `particle_stress` 压测对照总纲
   0.8ms/2 万粒子预算。

## 12. 后续工作与扩展点（记账不实施）

| 项 | 内容 | 时点 |
|---|---|---|
| **Layer G 速度积分提案** | 格内移速 ≤4 + 超限自然脱格；重写 rules.rs 移动语义、复核 r≤16、Cell aux 位段占位（bits 17–31 现余 15 位，速度约需 3–5 位）、golden 作废重录、过 §11 | M1 后独立立项 |
| durability/hardness | 材料表字段化，替换爆炸 wall=∞ 简化 | M2 反应表 |
| 粒子穿水/入水减速 | 现阻挡语义一视同仁的细化 | M2 评估 |
| 法术 payload | 粒子挂 payload 复用弹道基建（`kernel-charter.md:65`） | M4 |
| O3 粉末惯性 | 静止堆免斜下判定 + inertial_resistance | M2 spec 裁决 |
| 粒子渲染表形态 | bridge MultiMesh vs 原始数组（`program-architecture.md:166`） | M1 渲染压测后 |

## 13. 决策记录

| # | 决策 | 依据 |
|---|---|---|
| 1 | M1 最小闭环，Layer G 速度后置 | §1；用户批准 2026-08-30 |
| 2 | 粒子层用定点（维持总纲 §6），手写 Q16.16 | §2；用户裁决 2026-08-30 |
| 3 | 爆炸采 Noita 射线模型（非圆盘扫描） | §6；wiki 查证 + 遮挡涌现 |
| 4 | 发射器 = script Every + Op::Emit，harness 零新概念 | §7 |
| 5 | 哈希格式变更 → golden 按 §9 两步程序重录 | §9 |
| 6 | 废除"全占转悬浮"，改五邻格全占后向上兜底搜索、搜到世界顶仍无 air 则出界 | §5；Task 4 评审 2026-08-30 修复轮 1 实证活锁（C1），悬浮路径在静态堆场景两 tick 死循环 |
| 7 | 爆炸引入近心汽化 `vaporize_threshold`（每材质字段，严格质量守恒放宽为"守恒 + 汽化计数"）：射线剩余能量比例超过材质阈值即删除、不生成粒子 | §6.1；用户裁决 2026-08-30，观感诉求"近心没了、外圈飞溅" |
| 8 | 本轮手感迭代四项：`EXPLODE_SPEED` 16→8（更重）、密度冲量缩放 `v∝1/密度`（`REF_BLAST_DENSITY=40`）、射线方向涨落 ±25%/−25%（`explode_attempt` 扩位到低 2 位，不牵连 `emit_attempt`）、`sand.vaporize_threshold` 0.7→0.95（`water` 0.4 不变）；`explosion_splash.ron` 同步收窄 20000→8000 tick（19 炮→8 炮） | §6.2；用户目检迭代裁决 2026-08-30（多轮实机渲染观察后逐项定稿） |
