> 文档路径：`docs/superpowers/specs/2026-09-05-m4-player-and-spells-design.md`
> 运行时版本：Rust（内核）+ Godot 4 + gdext（表现层）
> 最近更新：2026-09-05 (UTC+8)
> **Status**: Proposed

# M4 玩家与法术 · 实现级设计

真源约束：`docs/overview/kernel-charter.md`（宪法）、`docs/overview/program-architecture.md`（架构）。
外部依据：`docs/reference/noita-deep-dive.md` §4.3、`docs/reference/noita-grid-api-and-rng.md` §1/§2、
以及本轮新查的 wiki 组件文档（`ProjectileComponent` / `VelocityComponent` / `CharacterDataComponent` /
`CharacterPlatformingComponent` / `DamageModelComponent` / `ConfigGunActionInfo`，见 §9 引用）。

---

## 0. 范围与验收

### 0.1 M4 做什么

总纲 §11 的 M4 定义是"玩家实体、loadout 数据模型、法术执行原语（弹体/喷射/爆炸/状态效果）"。
**本 spec 按用户裁决收窄**（brainstorm 2026-09-05）：M4 = **会动的生物 + 会飞的投射物**。

- **做**：生物实体（运动学、网格碰撞、排开液体、游泳、HP、材质接触伤害）；弹体子系统
  （积分、DDA 命中、侵彻、弹跳、刚体冲量）；三条法术原语（Bolt / Blast / Spray）；
  loadout + cooldown + mana 双闸门；InputFrame 正式编码。
- **不做**：Noita 式施法状态机（牌堆抽取 / 多重施法 / trigger / timer / shuffle / 法杖构筑）——
  整块推后；**stain 状态效果**（总纲 M4 定义的第四类原语）——推后，理由见 §8 第 3 条。
- **不触发**总纲 §11 翻案记录第 6 条的复议条件：M4 法术里没有"加热/冷却"这类可叠加可读数的
  **连续温度量**，火就是火材质、伤害走材质接触表。Layer F 保持删除。

### 0.2 验收标准

1. `data/scenarios/duel.ron`：两个生物按输入时间线跑跳互射，全程 SyncTest 六配置 2 万 tick 零分叉。
2. 三原语 + 侵彻 + 弹跳 + 刚体冲量各有行为测试（§7.2 九条）。
3. 材质接触伤害能致死（站火里烧死）。
4. 6 个既有 golden 重录（先 `--grid-only` 取证网格哈希流逐位不变）；线程数 1/8/16 逐位相同；
   散布角分布回归绿。
5. bench 落 `docs/perf/2026-09-05-m4-player-and-spells.md`，无预算外回退。
6. GIF 目检经用户签收。

---

## 1. 管线与协议变更

### 1.1 第 2 步从占位变生效

架构 §4 的规范 tick 管线第 2 步"实体与法术"自立宪起就是空占位。M4 让它生效，**属协议版本变更**，
按 M1 粒子相（§11 实施期决策第 1 条）与 M3 刚体相（第 8 条）的先例记入决策日志。

```
1.  ops 应用                （含新增 Op::SpawnCreature）
2a. 输入应用                InputFrame[controller] → Creature 意图（按 creature id 序）
2b. 生物运动学              扫掠碰撞 / 排开液体 / 浮力 / 材质接触伤害 / HP（按 id 序）
2c. 弹体积分                DDA 走位 → 命中结算（按弹体下标序）
2d. 施法结算                cooldown + mana 闸门 → 产弹体（Bolt/Blast）或喷射（Spray，按 creature id 序）
3.  刚体相                  （M3，不变）
4.  网格四相                （M0–M2，不变）
5.  粒子相                  （M1，不变）
7.  刚体对账 + 限额重提取    （M3，不变）
7'. 爆炸冲量                （M3，Blast 与 Op::Explode 共用此出口）
```

三条子步骤定序理由，全部承重：

- **2b 在 2c 之前**：弹体命中的是生物**本 tick 移动后**的位置。反过来会让"跑进弹道"这一帧的语义
  依赖上一 tick 的陈旧位置。
- **2c 在 2d 之前**：新生弹体本 tick 不积分、下一 tick 起飞，杜绝"出生即穿墙"（弹体出生点在
  生物身上，若当帧就走 DDA，第一段路径起点在生物 AABB 内部）。
- **整个第 2 步在第 3/4 步之前**：生物与弹体读同一份"本 tick 起始网格"，与架构 §4 原文
  "玩家运动学（采样本 tick 起始网格）"一致；弹体炸出的洞在同 tick 的网格四相里就被消化。

### 1.2 总纲 §4 措辞澄清（非翻案）

总纲 §4 Layer P 末句："法术弹体复用同一套弹道基建（挂 payload 的粒子）"。M4 落地为
**独立弹体表**，复用的是 `dda.rs` 与 `fixed.rs` 两个模块，而非 `Particles` 池本体。

判据：两者语义不同——粒子是**材质搬运器**，落格即变 cell、无 lifetime（`particle.rs:9` 明文）；
弹体是**事件载体**，命中即结算、有寿命与归属。塞进同一个 65536 容量的 SoA 会让每颗粒子多背
三列，且 `commit` 里长出"这颗是材料还是弹体"的分支，污染 M1 已验收的落格与保序压缩论证。

这是措辞澄清而非推翻裁决（弹道基建仍然复用），随实施期决策一并落档。

### 1.3 哈希结构

`state_hash` 由 `combine3` 变 **`combine4`**（网格 / 粒子 / 刚体 / 实体）。6 个既有 golden
（`sand_pile` / `waterfall_ci` / `mixed` / `explosion_ci` / `fire_oil_chain` / `crate_yard`）
全部重录，按 M3 先例先用 `--grid-only` 取证网格哈希流逐位不变。

---

## 2. 模块划分

| 新模块 | 职责 | 读 | 写 |
|---|---|---|---|
| `input.rs` | `InputFrame` 编解码；BAM 角类型 | — | — |
| `creature.rs` | 生物表：运动学、扫掠碰撞、排开、游泳、接触伤害、HP | cells、`InputFrame`、生物模板表 | 生物状态、cells（排开）、`spawn_queue` |
| `projectile.rs` | 弹体表：积分、DDA、命中结算、侵彻、弹跳 | cells、生物 AABB、法术表 | 弹体状态、cells（侵彻删格）、`spawn_queue`、`pending_blasts`、生物 hp/速度 |
| `spell.rs` | 法术表 + loadout + 施法结算（cooldown/mana 闸门） | 生物意图、法术表 | 弹体表、`spawn_queue`（Spray）、爆炸出口 |

BAM → 方向向量的 **1024 项 `Fx` 查表**并进已有的 `fixed.rs`（数值模块），不单开文件。

**既有代码的一处整理**（限于服务本目标）：M3 的硬地形谓词（非 air/Gas/Liquid、非 `body_passable`）
现藏在 `body.rs` 的地形缓存里，生物碰撞要用同一个。抽到共用位置（`material.rs` 或 `cell.rs` 旁），
两侧共享——**保证刚体盖章格对生物就是可站立平台**，M3 的木箱免费变成地形。

依赖方向：`spell.rs → projectile.rs → creature.rs → input.rs`，均单向，不成环。

---

## 3. 数据模型

### 3.1 InputFrame（`input.rs`，4 字节）

```rust
pub struct InputFrame {
    pub buttons: u8,  // bit0 左 / bit1 右 / bit2 跳 / bit3 开火 / bit4 下蹲 / bit5-7 留白
    pub aim: u16,     // BAM 无符号 16 位角（65536 = 360°）
    pub slot: u8,     // 选中法术槽 0..MAX_SLOTS-1
}
```

架构 §3 `bridge-input` 条目定的就是"位打包按键 + BAM 定点瞄准角，约 8 字节"。M4 把它定死，
**生物控制器只吃 `InputFrame`**——这让 P2 铁律"Godot → 核心唯一写入路径 = InputFrame"
在 M4 就获得类型级担保，而不必等 bridge 落地。

`Sim::step` 签名扩展为 `step(&mut self, ops: &[Op], inputs: &[InputFrame])`。`inputs` 按
**controller 序号**索引（不是 creature id）；生物的 `controller` 字段是它的下标，`255` = 不吃输入。

### 3.2 Creature（`creature.rs`，AoS）

数量个位数，AoS 比 SoA 清楚。**id = 下标，永不回收**——`InputFrame` 与 loadout 按 id 关联，
压缩会错位；死亡走 `alive = false` 墓碑。

```rust
pub struct Creature {
    pub x: Fx, pub y: Fx,              // AABB 中心
    pub vx: Fx, pub vy: Fx,
    pub half_w: i32, pub half_h: i32,  // 半宽高（格）
    pub hp: i32,                       // 千分之一点，整数
    pub mana: i32, pub mana_max: i32, pub mana_regen: i32,  // 同为千分之一单位
    pub cooldowns: [u16; MAX_SLOTS],   // 剩余帧
    pub loadout: [u8; MAX_SLOTS],      // 法术 id，255 = 空槽
    pub aim: u16,                      // BAM
    pub team: u8,
    pub controller: u8,                // 255 = 不吃输入
    pub template: u8,                  // 指回 creatures.ron
    pub on_ground: bool,
    pub facing_right: bool,
    pub alive: bool,
}
```

`hp` / `mana` 用整数千分位：RON 里写十进制小数，**加载期一次性量化**——沿用既有体例
（`quantize_fx` / `quantize_splash_chance`），理由见 `noita-grid-api-and-rng.md` §5.4
（逻辑量绝不能经浮点序列化往返）。

`MAX_CREATURES = 16`，超限 `Op::SpawnCreature` 确定性拒绝。

### 3.3 Projectile（`projectile.rs`，SoA）

下标即 id、保序压缩，与 `Particles` 完全同体例（`particle.rs` 那套论证直接适用）。

```rust
x, y, vx, vy: Fx     // 位置与速度
spell: u8            // 指回法术表
life: u16            // 剩余帧
energy: u32          // 剩余侵彻能量池
owner: u8            // 发射者 creature id
grace: u8            // 防自伤宽限剩余帧
bounces: u8          // 剩余弹跳次数
```

`MAX_PROJECTILES = 4096`，满则确定性拒绝（同 M1 粒子池的口径，计数器不入哈希、只供诊断）。

### 3.4 `data/spells.ron`

扁平记录，无引用、无递归（用户裁决："只三原语"）。

```ron
(
  spells: [
    (
      name: "spark_bolt",
      kind: Bolt( damage: 5.0, knockback: 2.0 ),
      // ── 施法闸门 ──
      mana: 10.0, cooldown: 12,
      // ── 出射 ──
      speed: 8.0, life: 120, gravity: 0.0, spread_deg: 2.0, grace: 4,
      // ── 侵彻（Bolt / Blast 公共）──
      dig_power: 0, max_durability: 10,
      // ── 运动修饰 ──
      air_friction: 1.0, liquid_drag: 0.9, pass_through: ["gas"],   // Category 名列表 → 加载期编译成掩码
      displace_liquid: true,
      bounces: 0, bounce_energy: 0.5,
      // ── 与刚体 ──
      physics_impulse: 0.0,
      // ── 死亡 ──
      on_lifetime_out_explode: false,
    ),
  ],
)
```

| `kind` | 产弹体 | 结算 |
|---|---|---|
| `Bolt { damage, knockback }` | 是 | 命中生物：扣血 + 给受击者速度；命中硬格（能量耗尽或门槛免疫）：消失 |
| `Blast { power, radius, max_durability }` | 是 | 命中即走**现有** `explode::apply_explode` 全套 + 追加 `bodies.pending_blasts`，第 7' 步统一施冲量——与 `Op::Explode` 完全同一口径 |
| `Spray { material, count, speed, jitter }` | **否** | 施法当帧直接走**现有** `emit::apply_emit` 路径塞 `spawn_queue` |

`pass_through` 是 **`Category` 掩码**（不是 Noita 的 per-material `go_through_this_material`）——
更贴我们的显式 `Category` 体系，一个 u8 表达"穿气体 / 穿液体"。缺省穿 Gas。

### 3.5 `data/creatures.ron`

```ron
(
  templates: [
    (
      name: "player",
      half_w: 2, half_h: 5,
      hp_max: 100.0, mana_max: 100.0, mana_regen: 20.0,   // mana_regen 单位：点/秒
      run_speed: 0.67, jump_speed: 2.9,                    // 格/tick
      accel_ground: 0.05, accel_air: 0.005,                // 格/tick²，起步猜测，按目检标定
      climb_over_y: 3,
      // 受害者侧伤害表（Noita 口径：谁怕什么是受害者的属性，不是材质的属性）
      damage_from: [ ("fire", 3.0), ("lava", 30.0), ("acid", 8.0) ],  // 点/秒
      min_cell_count: 4,
      max_displace_per_tick: 24,
    ),
  ],
)
```

`spells_fp` / `creatures_fp` 与 `materials_fp` / `reactions_fp` 同等待遇入握手指纹（P5）。
哈希前走 `normalize_for_fingerprint` 剥 CR（§11 实施期决策第 4 条的既有卫生）。

### 3.6 新增 Op

```rust
Op::SpawnCreature { x: i32, y: i32, template: u8, team: u8, controller: u8, loadout: [u8; MAX_SLOTS] }
```

与 `Op::SpawnBody` 同体例：由 `Sim::apply_one` 截走路由到 `Creatures`，`World` 不持有生物。

---

## 4. 生物：运动学与世界互动

### 4.1 运动

全 Q16.16 定点。数值以 Noita 为起点换算到 格/tick（60Hz）：`run_velocity` 40 px/s → 0.67、
`jump_velocity_y` −175 px/s → −2.9、`climb_over_y = 3`、`check_collision_max_size = 5×5`。
加速度（`accel_x` 地面 1 / 空中 0.1）的单位在 Noita 侧不明确，**RON 里的值是起步猜测、
按目检标定**。全部进 `docs/tuning-knobs.md` 的 **A 类**（手感）。

重力与网格同源，取 `cell::G_ACCEL` 一致的口径（不再单列一个会与网格脱节的常量）。

### 4.2 碰撞

**AABB 逐轴分离扫掠，先 x 后 y**——顺序即协议，改它进决策日志。每轴按整格步进，单 tick 位移
上限 `CREATURE_MAX_STEP`（起步 8 格）。硬格谓词 = §2 抽出来的共用谓词（非 air/Gas/Liquid、
非 `body_passable`），故**刚体盖章格天然可站立**。

**自动跨台阶**：水平被挡时依次试抬高 1..`climb_over_y` 格重试，成功即接受（Noita `climb_over_y`）。
抬高判定按固定升序，无掷骰。

### 4.3 排开液体与粉末

扫掠经过的液体/粉末格 → 置 air + 脱格成粒子，**复用 M3 盖章那条已验证的通路**
（`set_cell_stamped` + `spawn_queue`，被盖液体脱格的同一段代码）。带每 tick 每生物上限
`max_displace_per_tick`，超限即**不排开、不排队**——排队需要跨 tick 状态，会把限流变成状态机
（同 M1 溅射限流第 ② 条的先例）。排开顺序 = 扫掠格序，确定。

### 4.4 游泳

AABB 内液体格计数 → 三档浮力系数（idle 1.2 / up 0.9 / down 0.7，Noita `swim_*_buoyancy_coeff`）
+ `swim_drag = 0.95`。档位由本 tick 的竖直意图（上/下/无）选取。

**已知限制**：Noita 的 `liquid_velocity_coeff = 9`（水流推着人走）依赖它的液体**速度场**；
我们的网格只有无符号竖直速度位、**没有水平速度场**——这正是 `docs/tuning-knobs.md` §6 缺口 #2
挂着的那条债。故 M4 的水只做**阻力 + 浮力**，"急流冲走玩家"留到有水平速度场之后，
**不在 M4 里为它开位段**。

### 4.5 材质接触伤害与死亡

碰撞扫掠时顺手统计 AABB 内各材质格数（几乎零成本，那些格本来就要读）。判定：

1. 某材质当帧接触格数 `< min_cell_count`（4）→ **整项忽略**（Noita `material_damage_min_cell_count`）。
2. 否则查**受害者模板**的 `damage_from`，伤害 = 格数 × dps / 60。

方向性来自 Noita 一手（`DamageModelComponent.materials_that_damage` 在受害者身上，
见 `noita-grid-api-and-rng.md` §7）：**抗性/易伤写在生物侧，不动材质表**——这对 1v1 的
角色/护盾差异化很关键。

`hp <= 0` → `alive = false` + 一条 Channel B 事件。不做 ragdoll、不做尸体、不掉落。

---

## 5. 弹体

### 5.1 每 tick 流程（按下标序）

```
vy += spell.gravity
(vx, vy) *= spell.air_friction              // Noita air_friction，一次 Fx 乘法
若起点在液体格内: (vx, vy) *= spell.liquid_drag
沿 dda::cell_walk 逐格推进，按“先到者优先”判定：
  ① 命中格   → §5.2 侵彻判定
  ② 命中生物 AABB（按 creature id 序）→ §5.3 结算 → 死亡
路径走完 → life -= 1；life == 0 → 死亡（若 on_lifetime_out_explode 则先炸）
```

`pass_through` 掩码里的 `Category` 不算"命中格"，直接穿过。

### 5.2 侵彻（Noita `ground_penetration_*`）

`noita-grid-api-and-rng.md` §2 把它钉死过：爆炸/激光/闪电三兄弟都是"带能量预算的射线 +
durability 门槛"。**弹体侵彻是第四个同构用例**，射线方向由弹体速度给、能量池跟着弹体走。
因此复用 `explode::fire_ray` 的逐格消能逻辑，不新写一套：

1. 该格 `durability > spell.max_durability` → **门槛免疫**，按 kind 结算 → 死亡。
2. `energy == 0` → 按 kind 结算 → 死亡。
3. 否则按材质 `hp` 扣 `energy`，删格（盖**当前** stamp，按既有 `vaporize_threshold` 决定
   汽化还是溅射成粒子），**继续沿路径飞**。

三种法术形态由此自然长出：`dig_power = 0` 的普通弹（撞墙即停）、大 `dig_power` 小 `damage` 的
**挖掘弹**、中等 `dig_power` + `Blast` 的**钻进去再炸**。

**不违反 P4**：第 2 步是**串行**阶段，`window.rs::MAX_WRITE_RADIUS = 12` 只约束网格四相 pass 内
每个 cell 的影响半径。弹体一 tick 最多走 `MAX_SPEED = 16` 格、删 16 格，写在四相之前，
与 `Op::Explode`（半径 20，第 1 步）同一相位区间。此句写入 spec 是为了防日后误判。

### 5.3 命中结算

| kind | 命中生物 | 命中格（能量耗尽/门槛免疫） |
|---|---|---|
| `Bolt` | `hp -= damage`；受击者速度 += 弹体方向 × `knockback` | 消失 |
| `Blast` | 同上 + 在命中点触发爆炸 | 在命中点触发爆炸 |

爆炸走**现有出口**：`explode::apply_explode`（网格射线 + 溅射）+ 追加 `bodies.pending_blasts`，
第 7' 步与 `Op::Explode` 一起统一施刚体冲量。零新增通路。

**防自伤**：`owner` 在 `grace` 帧内跳过；同 `team` 跳过。不做 `penetrate_entities`（穿透生物），
命中生物即死。

### 5.4 弹跳（Noita `bounces_left` / `bounce_energy`）

本批唯一有真实实现量的一条。撞硬格且 `bounces > 0` 时：法线取自 **DDA 的撞击轴**
（`cell_walk` 天然知道是 x 还是 y 先跨格，法线是纯整数、免费），对应轴速度取反并乘
`bounce_energy`，`bounces -= 1`，继续本 tick 剩余路径。`bounces == 0` 时走 §5.3 结算。

不做 `bounce_always` / `bounce_at_any_angle`（Noita 为特殊角度打的补丁）。

### 5.5 排开液体与刚体冲量

- `displace_liquid`：飞行路径上的液体格脱格成粒子，**复用 §4.3 同一条通路**，近乎零成本。
- `physics_impulse`：命中刚体盖章格时给该 body 一个**单点冲量**（Noita `physics_impulse_coeff`：
  `Impulse = coeff × velocity`）。`Bodies` 新增单点冲量 API（约 20 行，改造 `apply_blast`
  已有的"施于受击像素加权中心"那套）。修复 M3 留下的说不通的缺口——箱子能被炸飞却不能被射中。

### 5.6 明确不做（Noita 有、我们 M4 不要）

`lifetime_randomness` / `die_on_liquid_collision` / `die_on_low_velocity` /
`on_death_duplicate_remaining` / `on_death_emit_particle` / `spawn_entity` /
`speed_min`+`speed_max` / `friction` / `mass` / `direction_nonrandom_rad`（扇形均布）/
`lob` / `HomingComponent` 全组 / `collide_with_world` / `penetrate_world` /
`collect_materials_to_shooter` / `penetrate_entities` / `damage_every_x_frames` /
`damage_scaled_by_speed` / `ConfigDamagesByType`（15 类伤害矩阵）/ `damage_critical` /
`damage_game_effect_entities`（= stain）/ `never_hit_player` / `explosion_dont_damage_shooter` /
`penetrate_world_velocity_coeff`（侵彻减速——能量池已限制深度，不再加耦合旋钮）。

表现层字段（`shoot_light_flash_*` / `muzzle_flash` / `shell_casing` / `velocity_sets_*` /
`angular_velocity` / `bounce_fx` / `camera_shake` / `screenshake` / `recoil` 视觉部分 /
`blood_*` / `gore_particles` / `ragdoll_*` / `hit_particle_force_multiplier` / `light`）
**一律不进核心**，需要时走 Channel B。这条线 Noita 自己也画了五遍
（`noita-grid-api-and-rng.md` §3：real vs cosmetic）。

---

## 6. 施法

### 6.1 双闸门

loadout = 赛前构筑的 `MAX_SLOTS`（起步 4）条法术。`InputFrame.slot` 选槽、`buttons` bit3 开火。

```
若 slot 空 / cooldowns[slot] > 0 / mana < spell.mana → 不出，无副作用
否则：mana -= spell.mana；cooldowns[slot] = spell.cooldown；按 kind 产出
每 tick 收尾：cooldowns 全体饱和递减 1；mana = min(mana_max, mana + mana_regen/60)
```

mana 回复照抄 Noita `ManaReloaderComponent` 的形态（每帧加 `mana_charge_speed/60`），
只是我们用整数千分位而非浮点。

### 6.2 出射方向与散布

方向 = `InputFrame.aim`（BAM）经 1024 项 `Fx` 查表得 `(cos, sin)`。
`spread_deg > 0` 时掷一次骰在 ±spread 内偏转。出生点 = 生物 AABB 中心沿出射方向偏移
`muzzle_offset`（模板字段，起步 = half_w + 1），保证不在自己身体里出生。

---

## 7. 确定性执法与测试

### 7.1 确定性

- **定序**：生物按 id（墓碑制、id 永不回收）；弹体按下标；施法按 creature id；
  弹体命中生物的遍历按 creature id。
- **随机**：全 M4 只有一处掷骰——散布。新增 `STREAM_SPREAD = 9`，
  key = `(tick, creature_id)`、salt = 槽位、attempt = 本 tick 第几发。严格照总纲 §11
  翻案记录第 4 条区分同帧同源的多次掷骰（Noita 宝箱事故的教训）。
- **数值**：全 `Fx`。BAM → 方向走 1024 项 `Fx` 常量表（核心禁系统超越函数，总纲 §6），
  金值测试钉死表内容。弹跳法线取自 DDA 撞击轴，纯整数。
- **容器**：数组与 SoA，无 HashMap。
- **写域**：第 2 步是串行阶段，不受 `MAX_WRITE_RADIUS` 约束（§5.2 已论证）。
  弹体删格盖**当前** stamp，与 ops 同口径。
- **限流**（两端必须一致）：`MAX_PROJECTILES = 4096` / `MAX_CREATURES = 16` /
  `max_displace_per_tick`，全部确定性拒绝、不排队。
- **哈希**：`combine4`（网格 / 粒子 / 刚体 / 实体）。实体层 = 生物全字段（含 `cooldowns`、
  `mana`、`hp`、`aim`）+ 弹体 SoA 全列。
- **指纹**：`spells_fp` / `creatures_fp` 入握手。

### 7.2 测试

**单测**：逐轴扫掠 / 跨台阶 1–3 格 / 排开上限 / 浮力三档 / 接触伤害 4 格门槛 /
hp 归零墓碑且 id 不回收 / DDA 先到者优先（格 vs 生物）/ grace 帧 / team 跳过 /
侵彻扣能量与 durability 门槛 / 弹跳次数与衰减 / `air_friction`·`liquid_drag`·`pass_through` /
容量拒绝 / cooldown + mana 双闸门 / InputFrame 编解码往返 / BAM 查表金值。

**行为测试（9 条）**

1. 跳上沙堆并站住；刚体盖章格同样可站立。
2. 跑过水面溅起水花，排开成粒子且水量守恒。
3. 站在火里掉血直至死亡。
4. `Blast` 炸穿石墙并把木箱推走。
5. `Bolt` 命中对手扣血 + 击退。
6. 挖掘弹钻进石头 N 格后能量耗尽停住；`wall`（durability 15）挡得住（门槛免疫）。
7. **`Spray` 浇油 → `Bolt` 点燃 → 连锁**——跨 M2 反应表的端到端，"环境连锁"卖点的第一个可测形态。
8. 弹跳弹弹 3 次后死亡。
9. 弹体射中木箱把它推走（`physics_impulse`）。

**分布回归**（M2 §7.2 立的规矩：概率分支必须验分布，SyncTest 抓不到 RNG 维度缺失类 bug）：
散布角在 ±spread 上均匀（分腔 + 4σ）。

**执法**：`duel.ron` + 既有场景 SyncTest 六配置 2 万 tick 零分叉；线程数 1/8/16 逐位相同；
6 个既有 golden 重录；bench 落 `docs/perf/2026-09-05-m4-player-and-spells.md`。

### 7.3 新场景

`data/scenarios/duel.ron`：两个生物 + 输入时间线 + 一池水、一堆油、一面石墙。
golden 回放与 GIF 目检共用。

场景 RON 新增 `inputs` 字段：`tick → [InputFrame]` 的稀疏时间线（缺省沿用上一条，
避免逐帧铺满）。harness 加载期编译成 `Vec<Vec<InputFrame>>`，与既有 `grid` 字段
（地图编辑器那条）同体例——core 零改动。

---

## 8. 决策记录

1. **弹体独立于粒子池**（用户裁决 A）。总纲 §4 "挂 payload 的粒子"措辞澄清为"复用 `dda`/`fixed`
   模块"，非翻案。判据见 §1.2。
2. **玩家对世界有写能力**（用户裁决 B）：碰撞 + 排开液体 + 游泳。不做"与刚体互推的影子刚体"
   （M4 场景里本来就没几个箱子可推），刚体盖章格可站立是免费副产品。
3. **stain 状态效果推后**。总纲 M4 定义的第四类原语，本次不做——用户选了"只三原语"，
   且 stain 的价值（湿了防燃、油了易燃）要等燃烧与法术都成体系才显现。记为 **M4 范围收窄**，
   不是翻案：总纲 M4 条目的措辞不改，在决策日志里注明"状态效果原语顺延"。
4. **不触发翻案第 6 条复议**（Layer F 温度场）。M4 法术无连续温度量，宪法级开关保持关闭。
5. **法术表扁平、无脚本**。总纲 §11 待决项"法术表达力是否升级为受限确定性脚本 VM"
   **本轮判定为不升级**，起步纯数据表——Noita 走到成品形态也是 `ConfigGunActionInfo`
   那张 ~70 字段的扁平表，无脚本。该待决项判定时点顺延到施法状态机落地后重评。
6. **施法节流 = cooldown + mana 双闸门**（用户裁决 B）。
7. **弹体七项扩展**（用户全收）：`displace_liquid` / `on_lifetime_out_explode` /
   `air_friction` / `liquid_drag` / `pass_through`（改造为 `Category` 掩码）/
   `physics_impulse`（推翻本轮设计中途"不给刚体冲量"的判断）/ `bounces` + `bounce_energy`。
8. **`liquid_velocity_coeff` 降级**：水流推人做不了（无水平速度场），M4 只做阻力 + 浮力。
   不为它开位段——那是 `tuning-knobs.md` §6 缺口 #2 的既有债，与 M4 无关。
9. **`Op::Explode` 的 `create_cell_material` 缺口不在 M4 补**（`noita-grid-api-and-rng.md` §8
   列的 M2 待办之一：被摧毁格按概率变火）。`Spray` + `Bolt` 已经给出更直接的点火路径。

## 9. 引用

| 来源 | 用途 |
|---|---|
| `docs/overview/kernel-charter.md` §2/§4/§5/§11 | 第一性原则、两层内核、确定性法典、里程碑与决策日志 |
| `docs/overview/program-architecture.md` §3/§4/§5/§8 | 子系统清单、规范 tick 管线、通信白名单、"玩家不是物理 body" |
| `docs/reference/noita-deep-dive.md` §4.3 | 四层模拟分类；玩家碰撞是逐像素的独立系统 |
| `docs/reference/noita-grid-api-and-rng.md` §1/§2/§3/§5/§7 | 网格操作词汇、能量射线三兄弟、real vs cosmetic、PRNG 事故、伤害在受害者侧 |
| [Documentation: ProjectileComponent](https://noita.wiki.gg/wiki/Documentation:_ProjectileComponent) | §5 字段全表（本 spec §5.6 的取舍依据） |
| [Documentation: VelocityComponent](https://noita.wiki.gg/wiki/Documentation:_VelocityComponent) | `air_friction` / `liquid_drag` / `displace_liquid` / `terminal_velocity` |
| [Documentation: CharacterDataComponent](https://noita.wiki.gg/wiki/Documentation:_CharacterDataComponent) / [CharacterPlatformingComponent](https://noita.wiki.gg/wiki/Documentation:_CharacterPlatformingComponent) | §4.1/§4.4 数值起点：`pixel_gravity` / `run_velocity` / `jump_velocity_y` / `climb_over_y` / `swim_*` |
| [Documentation: DamageModelComponent](https://noita.wiki.gg/wiki/Documentation:_DamageModelComponent) | §4.5：`materials_that_damage` / `material_damage_min_cell_count` |
| [Documentation: ConfigGunActionInfo](https://noita.wiki.gg/wiki/Documentation:_ConfigGunActionInfo) | §3.4 法术表形状；`action_mana_drain` / `fire_rate_wait` / `spread_degrees` |
| [Documentation: ManaReloaderComponent](https://noita.wiki.gg/wiki/Documentation:_ManaReloaderComponent) | §6.1 mana 每帧回复形态 |
| [Guide: Wand Mechanics](https://noita.wiki.gg/wiki/Guide:_Wand_Mechanics) / [Wands](https://noita.wiki.gg/wiki/Wands) | 被推后的施法状态机全貌（牌堆抽取、wrap、cast delay 与 recharge 并行不相加） |
