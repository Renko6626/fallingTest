# M2 反应表与燃烧实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> 文档路径：`docs/superpowers/plans/2026-08-31-m2-reactions-and-fire-plan.md`
> 最近更新：2026-08-31 (UTC+8)
> **Status**: Proposed

**Goal:** 落地 M2——气体 Category、数据驱动反应表、Noita 式燃烧（counter 燃料池/寿命），全部塞进 `Ctx::eval`、零新增 pass。

**Architecture:** 按 spec §1.5 方案 1：不改 tick 管线（非协议变更），新增写入全部 r ≤ 1。三个实施 Task 串行（气体 → 反应表 → 燃烧），数据层（spec Task 0）并入第一个 Task；末尾一个收口 Task（bench + u64 对照 + 总纲同步）。

**Tech Stack:** Rust（cargo workspace），RON 数据，xxh3 指纹，rayon 四相调度。

**Spec:** `docs/superpowers/specs/2026-08-31-m2-reactions-and-fire-design.md`（含审阅补漏 4+3 处，读代码前先通读）

## Global Constraints

- 确定性红线全文见 CLAUDE.md §5 与总纲 §2/§6：核心纯整数、hash 派生随机、禁默认 hasher、遍历定序。
- RNG 新流接续编号：`STREAM_REACT = 6`、`STREAM_IGNITE = 7`（现有 0–5，`rng.rs:23-100`）。
- 新增写入半径全部 ≤ 1，`window.rs:43` 的 `MAX_WRITE_RADIUS = 12` 断言不动；每处新写入点要有一行写域注释（spec §6.1"显式论证"要求）。
- 写回纪律（休眠红线）：静止且无 counter 的格子零写入（`rules.rs:141-147` 文档、spec §5.6）。
- 概率分支必须验分布，不只验哈希（spec §7.2）。
- 每个 Task 收尾：`cargo test --workspace` + `cargo clippy --workspace -- -D warnings` 全绿、golden 处置完成、spec §实施进度表打勾、`docs/CHANGELOG.md` 落账、单独 commit。
- 概率字段量化一律照 `splash_chance` 体例：RON 写 `0.0..=1.0`，加载期 `×255 round` 为 u8（`scenario.rs:176-187`）。
- Godot / godot CLI 一律不碰（CLAUDE.md §2.4）；GIF 目检由用户手动执行。

---

### Task 1: 数据层 + 气体（spec Task 0 + Task 1）

**Files:**
- Modify: `crates/sand-core/src/material.rs`（`Category::Gas` + 新字段与访问器）
- Modify: `crates/sand-core/src/cell.rs`（`CellRepr` 别名 + 堵 `pub u32` 缺口；counter 位段留 Task 3）
- Modify: `crates/sand-core/src/hash.rs:16`（`cell.0` → `cell.raw()`）、`crates/sand-core/src/world.rs:11`（`WALL_SENTINEL` 改用 const `Cell::pack`）
- Modify: `crates/sand-core/src/rules.rs`（`eval` 加 Gas 分支、`displace` 密度梯度按运动方向、`side` 提出 `reach` 参数）
- Modify: `crates/sand-harness/src/scenario.rs`（`MatSpec` 新字段 + 加载期校验/量化）
- Modify: `data/materials.ron`（新增 oil/wood/fire/smoke 四材质 + 新字段）
- Modify: `crates/sand-core/src/particle.rs`（落格 P→G 写速度位仅对非 Gas 材质）
- Test: `crates/sand-core/tests/rules_behavior.rs`、`material.rs`/`cell.rs`/`scenario.rs` 内嵌单测

**Interfaces（后续 Task 依赖）:**
- `Category::Gas`（enum 第四值；`eval` 的 match 因此强制穷尽处理）。
- `MaterialDef` 新字段（全部缺省安全，spec §2.1 表）：`tags: Vec<String>`、`ignition_temp: u8`(缺省 100)、`fire_temp: u8`(缺省 10)、`fire_hp: u8`(缺省 0)、`lifetime: u8`(缺省 0)、`decay_to: u8`(加载期由名字解析成 id，缺省 air=0)、`requires_oxygen: bool`(缺省 true)、`extinguisher: bool`(缺省 false)、`fire_chance: u8`(缺省 0，×255 量化——spec §5.3 的产火概率，对应 Noita `generates_flames`；spec §2.1 表漏列，落地时在 spec 表补一行)。访问器与现有同名体例（`table.fire_hp(id)` 等）。
- `MaterialTable` 新增 `tags_of(id) -> &[String]`（供 harness tag 展开）。
- `type CellRepr = u32;`（`cell.rs`）+ `Cell::raw() -> CellRepr`；`Cell(pub u32)` 的字段转私有，`Cell::pack` 改 `const fn`。
- `rules::side` 签名加 `reach: i32` 参数（liquid 传 `dispersion.min(DISPERSION_MAX)`，gas 传 1）。

**要点（实施顺序即步骤）:**

- [ ] **加载期契约先行（TDD）**：scenario.rs 单测——① `fire_hp` 与 `lifetime` 同时声明 ⇒ 报错（spec §2.1 校验）；② Gas 材质声明 `dispersion` ⇒ 报错（spec §3.1 审阅补漏）；③ `decay_to` 引用不存在材质 ⇒ 报错；④ 未声明新字段的旧表加载结果与现值逐字段相同（缺省安全）。跑红。
- [ ] **material.rs + scenario.rs 落实字段**。`decay_to` 在 RON 里写材质名字符串，`load_materials` 二遍解析（先建名→id 映射再回填），core 侧只见 u8 id（"core 不出现字符串"，spec §2.4 契约 2 的同一纪律）。跑绿。
- [ ] **Cell 封装**：`CellRepr` 别名、字段私有化、`pack` 转 const、`hash.rs`/`world.rs` 两个消费点改 `raw()`/const `pack`。全 workspace 编译过即是验证（缺口堵没堵编译器说了算）。
- [ ] **气体行为测试（TDD，spec §7.1）**：`rules_behavior.rs` 新增——① `gas_rises_straight_up`（镜像 `sand_falls_straight_down`，rules_behavior.rs:16 体例）；② `gas_rises_one_cell_per_tick_no_chain`（一列烟静置一 tick 后每格恰升 1，钉死 §3.3 的 stamp 防连锁）；③ `gas_bubbles_up_through_liquid`（水下烟上浮置换，§3.1 密度梯度反转）；④ `trapped_gas_lets_chunk_sleep`（天花板下困住的烟入睡——写回纪律）。跑红。
- [ ] **rules.rs 实现**：`eval` 的 category match 加 `Category::Gas => return self.gas_eval(x, y, c)`（在速度积分之前分流——气体不进 substeps、不碰速度位段，spec §3.2）。`gas_eval` = 单次 `gas_step`：上 → 两个斜上（`diag_side` 复用，k=0）→ 水平 `side(reach=1)`；`displace` 改为沿运动方向的密度梯度：`let rising = ny < y;` 上浮要求目标更重、下沉要求目标更轻，`tm == MAT_AIR` 快路径不变。跑绿。
- [ ] **particle.rs 一行守卫**：落格写速度位（P→G）跳过 Gas 材质——气体不读速度位段，写入只会在哈希里留死重量。
- [ ] **materials.ron 加四材质**（id 4–7：oil/wood/fire/smoke，字段值按 spec §1.3；oil 密度低于 water 的 16、`tags: ["burnable"]`；fire/smoke 是 Gas，本 Task 先不给 `lifetime`——衰变是 Task 3 的事，给了也没人递减，反而让"Task 1 golden 只变 fp"的断言复杂化）。
- [ ] **golden 处置**：四个 golden 场景不引用新材质 ⇒ 状态哈希应逐位不变。先 `cargo run -p sand-harness -- hashrun <scenario> --hash-every 1` 对改动前后 diff 取证（照 spec §7.3 既有做法），再 `replay --write-golden` 重录（只有 materials_fp 行变）。
- [ ] **收尾**：全测试 + clippy 绿；spec 进度表 Task 0/1 打勾；CHANGELOG 落账；commit `feat(core): M2 Task 1——数据层字段 + Category::Gas 上浮`。

---

### Task 2: 反应表 + durability 替换（spec Task 2）

**Files:**
- Create: `crates/sand-core/src/reaction.rs`
- Create: `data/reactions.ron`
- Modify: `crates/sand-core/src/lib.rs`（导出 + `Sim` 持有 `ReactionTable`）、`rng.rs`（`STREAM_REACT = 6`）、`rules.rs`（eval 准入重构 + 反应结算）、`material.rs`（`hp`/`durability` 字段替换 `blast_cost`）、`world.rs`（`Op::Explode` 加 `max_durability`）、`explode.rs`（双层破坏 + 哨兵退役）、`dda.rs`/`emit.rs`/`particle.rs` 测试表同步
- Modify: `crates/sand-harness/src/scenario.rs`（`load_reactions` + tag 展开 + `hp`/`durability`/`max_durability` 表面格式）、`runner.rs`（报告头加 `reactions_fp` 行）、`main.rs`（加载 reactions.ron）
- Modify: `data/materials.ron`（`blast_cost` → `hp` + `durability`，wall `durability: 15`）、`data/scenarios/*.ron`（Explode 不动——`max_durability` RON 缺省 10）
- Test: 各文件内嵌单测 + `rules_behavior.rs` + `synctest.rs`

**Interfaces:**
- `reaction.rs`：
  ```rust
  pub struct ReactionRule { pub a: u8, pub b: u8, pub out_a: u8, pub out_b: u8, pub threshold: u8 }
  pub struct ReactionTable { /* index: Vec<u16> (n×n, 0=无, else 1+rules偏移), rules: Vec<ReactionRule>, initiates: Vec<bool> */ }
  impl ReactionTable {
      pub fn empty(n_materials: usize) -> ReactionTable;                    // 既有测试的零迁移路径
      pub fn new(n_materials: usize, rules: Vec<ReactionRule>) -> Result<ReactionTable, String>; // 入参已展开/已规范化
      pub fn get(&self, a: u8, b: u8) -> &[ReactionRule];  // 要求 a<b；同对条目按加载序连续存放
      pub fn initiates(&self, a: u8) -> bool;              // 加载期预计算：是否存在以 a 为 id_a 的条目
  }
  ```
  规范化（`new` 校验，展开在 harness）：全部条目 `a < b`、按 `(a, b, 加载序)` 排序后同对连续；`a == b` 或引用越界 id ⇒ Err。
- `Sim::new(cfg, table, reactions: ReactionTable)`——所有既有调用点传 `ReactionTable::empty(table.len())`。
- harness：`load_reactions(path, &MaterialTable) -> Result<(ReactionTable, u64), String>`。RON 面：`(reactions: [(input: ["water","fire"], output: ["water","smoke"], probability: 0.80)])`；input 项可写 `[tag]`，output 只许具体材质名。tag 展开：成员按 id 升序、笛卡尔积、`id_a > id_b` 时连 output 一起换位、`id_a == id_b` 静默跳过（tag 自交预期内）而显式同名对报错；引用不存在材质/tag ⇒ Err（spec §2.4 契约 1，与 Noita 反着抄）。指纹 = 归一化字节 xxh3（`normalize_for_fingerprint` 复用）。
- `Op::Explode { x, y, r, power, max_durability: u8 }`；RON 面 `Explode` 加可选 `max_durability` 字段，serde 缺省 10。
- `MaterialDef`：`blast_cost: u32` → `hp: u32`（RON 字段名 `hp`，缺省 1），新增 `durability: u8`（缺省 0）。`BLAST_COST_INFINITE` 常量删除。

**要点:**

- [ ] **reaction.rs（TDD）**：单测——`get` 返回同对全部条目且按加载序；`initiates` 只对 id_a 为真；`empty` 全空；`new` 拒绝 a==b 与越界 id。实现按上述接口。
- [ ] **harness 加载（TDD）**：单测——tag 展开成员定序；正反只注册一次（对照 `archive/prototype-python/core/reaction.py:44-46` 的反例，spec §2.4 契约 3）；未知材质/tag 报错；probability 量化（0.80 → 204）。实现 `load_reactions` + `main.rs` 接线 + `runner.rs` 报告头 `reactions_fp` 行（挨着 materials_fp，进 golden 与握手指纹语义，P5）。
- [ ] **eval 准入重构（spec §2.6）**：`rules.rs:151` 的 `is_static || stamp` 早退改为：stamp 检查 → `needs_eval`（本 Task = `!is_static(m) ∨ initiates(m)`，Task 3 再加 `counter > 0` 项）→ 运动分支挂 `!is_static` 条件。`is_static`/`initiates` 合并进单个 per-material flags 字节一次载入（spec §2.6 审阅补记）。**先跑一遍四 golden 的逐 tick hashrun diff 存底**：本重构必须零行为变化。
- [ ] **反应结算（TDD）**：行为测试——① `initiator_convention_prevents_double_settlement`（water+fire 相邻恰各转化一次）；② `reaction_skips_stamped_neighbors`（spec §4.5 审阅补漏：同格一 tick 不二次转化）；③ 分布测试 `reaction_rate_matches_declared_probability`（大样本触发率贴近 0.80，体例照 rules_behavior.rs:521 的 splash 分布测试；spec §7.2 新规矩）。实现：`eval` 运动之后在落点上，4 邻域编译期常量序 `[(0,-1),(0,1),(-1,0),(1,0)]`，跳过已盖当前戳的邻居，`m < nm` 才查 `get(m, nm)`，逐条 `rng_u32(fseed, STREAM_REACT, x, y, salt=方向索引, attempt=条目序) % 255 < threshold`，首中即写两格产物 + 盖戳 + 速度清零，整段 break（spec §4.4 salt 纪律——rng.rs 文档注释按既有体例写满论证）。
- [ ] **durability 替换**：`fire_ray`（`explode.rs:199-201`）改 `durability > max_durability ⇒ break`、否则按 `hp` 扣能；wall 改 `durability: 15, hp: 100`；air `hp: 0`；哨兵与其全部引用（`dda.rs:184`、`emit.rs:158`、`particle.rs:385,496` 等测试表）清理。explode.rs 既有"撞哨兵断线"单测改为"durability 超限断线"。
- [ ] **场景与执法**：`data/reactions.ron` 落 `water+fire → water+smoke @0.80`（本轮内容稀薄是预期，spec §4.6）。新场景 `data/scenarios/fire_oil_chain.ron`（油池 + 木块 + 定点 Brush fire，2 万 tick——本 Task 火不衰变不点燃，场景先建好跑通 SyncTest，行为在 Task 3 长出来）；`synctest.rs` 加 `fire_oil_chain_six_configs_zero_divergence`。
- [ ] **golden 处置**：`materials_fp`/`reactions_fp`/报告头全变 ⇒ 四 golden 全重录；`explosion_ci` 状态哈希预期内改变（durability 门槛语义），其余三个先 `hashrun --hash-every 1 --grid-only` diff 取证逐位不变再重录（spec §7.3）。
- [ ] **收尾**：全测试 + clippy；spec 进度表；CHANGELOG；commit `feat(core): M2 Task 2——数据驱动反应表 + hp/durability 双层破坏`。

---

### Task 3: 燃烧（spec Task 3）

**Files:**
- Modify: `crates/sand-core/src/cell.rs`（counter 位段 24–31）、`material.rs`（若有零散补漏）、`rng.rs`（`STREAM_IGNITE = 7`）、`rules.rs`（burn 阶段）、`world.rs`（`set_cell_stamped` 装填 lifetime）
- Modify: `data/materials.ron`（fire/smoke 的 `lifetime`/`decay_to`、oil/wood 的 `fire_hp`/`fire_chance`/`ignition_temp`、fire 的 `fire_temp`、water 的 `extinguisher: true`、wood 的 `requires_oxygen: true`）
- Test: `cell.rs` 单测、`rules_behavior.rs`、`synctest.rs`

**Interfaces:**
- `cell.rs`：`COUNTER_SHIFT = 24`、8 位掩码；`Cell::counter() -> u8`、`Cell::with_counter(u8) -> Cell`。位段表注释更新（23 仍留白）。
- `rules.rs` 内部：burn 阶段函数 `fn burn(&self, x: i32, y: i32)`，在反应结算之后、以落点现值重读为准；`needs_eval` 加 `counter > 0` 项。
- RNG：`STREAM_IGNITE`，key=源格坐标，salt=0，attempt 常量 `IGNITE_ROLL_DIR = 0`（点燃方向骰）、`FIRE_ROLL_TRIGGER = 1`（产火触发骰）、`FIRE_ROLL_DIR = 2`（产火方向骰）——spec §5.8 的"不同 attempt"落到三个常量。

**要点:**

- [ ] **counter 位段（TDD）**：`cell.rs` 单测照 `vel_roundtrip_does_not_disturb_other_fields`（cell.rs:134）体例——counter 往返 + 与 material/stamp/dir/vel 互不污染 + **`with_stamp`/`with_vel`/`with_dir` 不清 counter**（spec §5.9 点名）。
- [ ] **装填时机**：`world::set_cell_stamped` 对 `lifetime > 0` 的材质写入时 `with_counter(lifetime)`（出生即装填——Brush/Fill/反应产物/粒子落格全走这条路径或在写入点各自装填；`displace` 搬整字自动跟随，spec §5.9）。反应产物写入点（Task 2 代码）补装填。
- [ ] **燃烧行为测试（TDD）**：① `ignition_needs_burning_source`（冷油贴木不点燃——spec §5.2 审阅补漏的门）；② `fire_ignites_adjacent_oil_and_chain_burns`（火油连锁端到端：油被点燃 → 产火 → 火衰变烟 → 烟衰变空气，材质计数随时间迁移）；③ `wood_burns_outside_in`（大块 wood 中心格在表面烧完前 counter 恒 0——含 §5.2 氧气前置后，内部格根本不被装填）；④ `water_extinguishes_burning_fuel`（extinguisher 清零）；⑤ `resting_wood_lets_chunk_sleep`（执法，照 rules_behavior.rs:393 体例）；⑥ 分布测试 `ignition_direction_roll_is_uniform`（四向点燃各 ≈25%，spec §7.2）。跑红。
- [ ] **burn 阶段实现**（顺序定死并写进代码注释，spec 审阅意见"阶段内顺序即语义"）：落点重读 cell；`counter == 0 ⇒ 立即返回（零写入红线）`；燃料格（`fire_hp > 0`）：①邻接 extinguisher ⇒ 清零返回 ②`requires_oxygen` 且四邻无 air ⇒ 清零返回（闷熄）③递减，归零 ⇒ 转 `decay_to`（含产物 lifetime 装填）返回 ④产火骰：`FIRE_ROLL_TRIGGER % 255 < fire_chance` 且 `FIRE_ROLL_DIR % 4` 指向 air ⇒ 写 fire+lifetime+戳 ⑤点燃骰：`IGNITE_ROLL_DIR % 4` 选邻居，按 spec §5.2 四条件（源 counter>0 已隐含、温度比较、目标可燃未燃、目标氧气前置）⇒ 装填目标 + 盖戳。寿命格（`lifetime > 0`）：递减/衰变同 ③，然后仅执行 ⑤（fire 靠高 `fire_temp` 点燃，smoke 靠低 `fire_temp` 天然失败——materials.ron 里 smoke 不声明 `fire_temp` 吃缺省 10）。`needs_eval` 加 `counter > 0`。
- [ ] **materials.ron 调参初值**（目检前的第一版，量级对齐 spec §5.2 的 Noita 锚点）：fire `fire_temp: 100, lifetime: 40, decay_to: "smoke"`；smoke `lifetime: 200, decay_to: "air"`；oil `ignition_temp: 40, fire_hp: 90, fire_chance: 0.6, decay_to: "air"`；wood `ignition_temp: 80, fire_hp: 250, fire_chance: 0.3, requires_oxygen: true, decay_to: "air"`；water `extinguisher: true`。
- [ ] **执法与 golden**：`fire_oil_chain` SyncTest 2 万 tick 六配置零分叉；四旧 golden 无火 ⇒ 逐位不变取证后重录 fp 行；新录 `fire_oil_chain.golden`。线程数不变性由 synctest 六配置覆盖（1/2/4 线程 × 扫描模式，沿用 runner::synctest 既有矩阵）。
- [ ] **收尾**：全测试 + clippy；spec 进度表；CHANGELOG；commit `feat(core): M2 Task 3——counter 燃烧链（点燃/产火/衰变/闷熄）`。GIF 目检项（火油连锁、烟上升、由外向内烧）留给用户，harness 渲染即可出图。

---

### Task 4: 收口——bench、u64 对照、总纲同步

**Files:**
- Modify: `crates/sand-core/Cargo.toml` + `cell.rs`（`cell-u64` feature：`CellRepr = u64`，掩码常量随别名走——2.3 的"随时可扩"就在这里兑现并顺手验证）
- Create: `docs/perf/2026-08-31-m2-reactions-and-fire.md`
- Modify: `docs/overview/kernel-charter.md` §11（实施期决策：M2 落地补记 + 修正翻案 6"延迟点燃队列"措辞——spec §5.7 落地待办）、spec（Status: Proposed → Implemented、进度表全勾）、`docs/CHANGELOG.md`、`docs/README.md` 优先队列

**要点:**

- [ ] **bench**：照 `docs/perf/2026-08-30-m0-rust-informal.md` 的口径（hashrun 计时），对比 M2 前后 `sand_pile`/`waterfall_ci`（无火基线，验"无回退"）+ `fire_oil_chain`（新增成本）；`--features cell-u64` 再跑一轮全活跃场景做 u64 对照（spec §2.3 用户裁决第 5 条）。数据落 perf 文档。
- [ ] **u64 feature 正确性**：`cargo test --workspace --features sand-core/cell-u64` 全绿（哈希与 golden 会变——feature 只用于对照测量，文档写明绝不进产品构建，体例照 `zero-gravity`，cell.rs:44-49）。
- [ ] **总纲 §11 实施期决策补记**：① M2 落地（三 Task、零新增 pass、非协议变更的论证归档）；② 修正翻案 6 中"复用 fire spec v2（尤其延迟点燃队列）"为"延迟点燃队列不移植，stamp 机制已覆盖，见 M2 spec §5.7"。
- [ ] **收尾**：spec Status 翻 Implemented；CHANGELOG；README 优先队列指向 M3；commit `docs+perf: M2 收口——bench 入档、u64 对照、总纲 §11 同步`。

---

## Self-Review 记录

- spec 覆盖：§2 全部字段/契约/查找结构/准入 → Task 1+2；§3 → Task 1；§4 → Task 2；§5 → Task 3；§6 写域注释与 §6.5 睡眠语义 → 各写入点注释；§7 测试矩阵 → 各 Task 的 TDD 步骤；§2.3 u64 与 §7.4 bench → Task 4；审阅补漏 4 条分别落在 Task 3（源门/氧气）、Task 2（戳跳过）、Task 4（总纲同步）、Task 1（gas dispersion 校验）。
- 类型一致性：`ReactionTable::get/initiates/empty`、`Sim::new` 三参、`side(reach)`、`Cell::counter/with_counter` 在各 Task 间签名一致。
- 已知折衷（非遗漏）：goldens 每 Task 重录一次 fp（三次），换来每 Task 独立的"状态哈希逐位不变"取证——与 Layer G 逐 Task 重录先例一致。
