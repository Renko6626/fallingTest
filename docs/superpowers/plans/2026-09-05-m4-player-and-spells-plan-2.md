> 文档路径：`docs/superpowers/plans/2026-09-05-m4-player-and-spells-plan-2.md`
> 运行时版本：Rust（内核）+ Godot 4 + gdext（表现层）
> 最近更新：2026-09-05 (UTC+8)
> **Status**: Proposed
> 前篇：`2026-09-05-m4-player-and-spells-plan.md`（Header / Global Constraints / File Structure / Task 1–4）

# M4 玩家与法术 · 实施计划（Task 5–7）

> **For agentic workers:** 本文是前篇的续，**Global Constraints 与 File Structure 见前篇**，
> 每个 Task 的验收隐含包含前篇的 Global Constraints 全节。
> **Spec:** `docs/superpowers/specs/2026-09-05-m4-player-and-spells-design.md`

---

## Task 5: 法术表与施法

**Files:**
- Modify: `crates/sand-core/src/spell.rs`（填实）、`projectile.rs`、`rng.rs`、`lib.rs`
- Modify: `crates/sand-harness/src/scenario.rs`（`spells.ron` 加载 + 指纹）
- Create: `data/spells.ron`
- Test: `crates/sand-core/tests/projectile_behavior.rs`（追加）

**Interfaces:**
- Consumes: Task 4 的 `Projectiles::spawn`、Task 3 的 `Creature.mana`/`cooldowns`/`loadout`
- Produces:
  - `SpellKind::{Blast, Spray}` 两个变体（`Bolt` 与 `SpellTable` 本体由 Task 4 建立）
  - `spell::SPELL_NONE: u8 = 255`（空槽哨兵，故法术数上限 255）
  - `SpellTable::id_by_name(&self, name: &str) -> Option<u8>`、`Sim::spell_id(&self, name: &str) -> u8`（测试用）
  - `spell::cast_all(creatures: &mut Creatures, projectiles: &mut Projectiles, world: &mut World, table: &MaterialTable, spells: &SpellTable, tpl: &CreatureTable, bodies: &mut Bodies, inputs: &[InputFrame], stamp: u8, fseed: u32, spawns: &mut Vec<SpawnRequest>)`
  - `rng::STREAM_SPREAD: u32 = 9`
  - `sand_harness::scenario::load_spells(path: &str, table: &MaterialTable) -> Result<(SpellTable, u64), String>`

- [ ] **Step 1: 加 `STREAM_SPREAD` 并写它的纪律注释**

`rng.rs` 追加（体例照 `STREAM_GASSTAY = 8`）：

```rust
/// 施法散布骰（M4 spec §7.1）。key = 施法者格坐标（tick 内每生物唯一——生物
/// AABB 互不重叠不成立，故**改用 creature id 编码进 x**：`x = id as i32`、`y = 0`），
/// salt = 槽位，attempt = 本 tick 该生物第几发。
///
/// 三个维度缺一不可：无 salt 则同 tick 换槽位掷同值；无 attempt 则同槽位
/// 连发全打同一方向——正是总纲 §11 翻案第 4 条点名、Noita 宝箱事故实证过的
/// RNG overlap（`docs/reference/noita-grid-api-and-rng.md` §5.2）。
pub const STREAM_SPREAD: u32 = 9;
```

- [ ] **Step 2: 写失败的行为测试**

追加到 `tests/projectile_behavior.rs`：

```rust
#[test]
fn firing_consumes_mana_and_sets_cooldown() {
    let mut sim = arena_with_loadout(&["spark_bolt"]);
    let m0 = sim.creatures().get(0).unwrap().mana;
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    let c = sim.creatures().get(0).unwrap();
    assert!(c.mana < m0, "施法必须扣 mana");
    assert!(c.cooldowns[0] > 0, "施法必须置冷却");
    assert_eq!(sim.projectiles().len(), 1, "应当出一发");
}

#[test]
fn cooldown_gate_blocks_a_second_shot() {
    let mut sim = arena_with_loadout(&["spark_bolt"]);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    assert_eq!(sim.projectiles().len(), 1, "冷却未好不得再出");
}

#[test]
fn mana_gate_blocks_when_insufficient_and_costs_nothing() {
    let mut sim = arena_with_loadout(&["expensive_bolt"]);
    sim.creatures_mut().set_mana_for_test(0, 0);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    assert_eq!(sim.projectiles().len(), 0, "mana 不足不得出");
    assert_eq!(sim.creatures().get(0).unwrap().mana, 0, "不出就不得扣费");
    assert_eq!(sim.creatures().get(0).unwrap().cooldowns[0], 0, "不出就不得置冷却");
}

#[test]
fn empty_slot_is_a_no_op() {
    let mut sim = arena_with_loadout(&[]);                   // 全空槽
    let m0 = sim.creatures().get(0).unwrap().mana;
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    assert_eq!(sim.projectiles().len(), 0);
    assert_eq!(sim.creatures().get(0).unwrap().mana, m0, "空槽无任何副作用");
}

#[test]
fn mana_regenerates_up_to_max() {
    let mut sim = arena_with_loadout(&["spark_bolt"]);
    sim.creatures_mut().set_mana_for_test(0, 0);
    for _ in 0..600 { sim.step(&[], &[]); }
    let c = sim.creatures().get(0).unwrap();
    assert_eq!(c.mana, c.mana_max, "10 秒后应当回满且不越界");
}

#[test]
fn slot_selects_which_spell_is_cast() {
    let mut sim = arena_with_loadout(&["spark_bolt", "bomb"]);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, /* slot */ 1)]);
    assert_eq!(sim.projectiles().spell(0), sim.spell_id("bomb"), "应当放 1 号槽的法术");
}

#[test]
fn aim_determines_launch_direction() {
    let mut sim = arena_with_loadout(&["spark_bolt"]);        // spread = 0 的法术
    sim.step(&[], &[InputFrame::new(BTN_FIRE, /* 90° 向下 */ 16384, 0)]);
    assert!(sim.projectiles().vy(0) > Fx::ZERO && sim.projectiles().vx(0) == Fx::ZERO);
}

#[test]
fn blast_spell_explodes_on_impact_and_carves_terrain() {
    let mut sim = arena_with_loadout(&["bomb"]);
    sim.apply_setup(&[Op::Fill { material: STONE, x0: 60, y0: 40, x1: 70, y1: 90 }]);
    let before = sim.world().count_material(STONE);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    for _ in 0..90 { sim.step(&[], &[]); }
    assert!(sim.world().count_material(STONE) < before, "Blast 必须炸出洞");
}

#[test]
fn spray_spell_emits_particles_without_creating_a_projectile() {
    let mut sim = arena_with_loadout(&["oil_spray"]);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    assert_eq!(sim.projectiles().len(), 0, "Spray 不产生弹体");
    assert!(sim.particles().len() > 0, "Spray 当帧就应产出粒子");
}

#[test]
fn projectile_spawns_outside_the_shooter_hitbox() {
    // muzzle_offset 保证不在自己身体里出生（否则第一帧就自撞）
    let mut sim = arena_with_loadout(&["spark_bolt"]);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    let c = sim.creatures().get(0).unwrap();
    let dx = (sim.projectiles().x(0).to_cell() - c.x.to_cell()).abs();
    assert!(dx > c.half_w, "出生点必须在自身 AABB 之外");
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p sand-core --test projectile_behavior`
Expected: 新增 10 条 FAIL。

- [ ] **Step 4: 写 `data/spells.ron`**

```ron
(
  spells: [
    (
      name: "spark_bolt",
      kind: Bolt( damage: 5.0, knockback: 2.0 ),
      mana: 10.0, cooldown: 12,
      speed: 8.0, life: 120, gravity: 0.0, spread_deg: 2.0, grace: 4,
      dig_power: 0, max_durability: 10,
      air_friction: 1.0, liquid_drag: 0.9, pass_through: ["gas"],
      displace_liquid: false,
      bounces: 0, bounce_energy: 0.5,
      physics_impulse: 0.0,
      on_lifetime_out_explode: false,
    ),
    (
      name: "bomb",
      kind: Blast( power: 1200, radius: 12, max_durability: 10 ),
      mana: 35.0, cooldown: 60,
      speed: 5.0, life: 180, gravity: 0.25, spread_deg: 0.0, grace: 20,
      dig_power: 0, max_durability: 10,
      air_friction: 1.0, liquid_drag: 0.8, pass_through: ["gas"],
      displace_liquid: true,
      bounces: 2, bounce_energy: 0.4,
      physics_impulse: 0.0,
      on_lifetime_out_explode: true,
    ),
    (
      name: "oil_spray",
      kind: Spray( material: "oil", count: 12, speed: 4.0, jitter: 0.6 ),
      mana: 8.0, cooldown: 6,
      speed: 0.0, life: 0, gravity: 0.0, spread_deg: 0.0, grace: 0,
      dig_power: 0, max_durability: 10,
      air_friction: 1.0, liquid_drag: 1.0, pass_through: [],
      displace_liquid: false,
      bounces: 0, bounce_energy: 0.0,
      physics_impulse: 0.0,
      on_lifetime_out_explode: false,
    ),
    (
      name: "digger",
      kind: Bolt( damage: 1.0, knockback: 0.0 ),
      mana: 15.0, cooldown: 20,
      speed: 6.0, life: 90, gravity: 0.0, spread_deg: 0.0, grace: 4,
      dig_power: 900, max_durability: 12,
      air_friction: 1.0, liquid_drag: 1.0, pass_through: ["gas", "liquid"],
      displace_liquid: false,
      bounces: 0, bounce_energy: 0.0,
      physics_impulse: 0.0,
      on_lifetime_out_explode: false,
    ),
    (
      name: "expensive_bolt",
      kind: Bolt( damage: 30.0, knockback: 6.0 ),
      mana: 90.0, cooldown: 90,
      speed: 10.0, life: 120, gravity: 0.0, spread_deg: 0.0, grace: 4,
      dig_power: 0, max_durability: 10,
      air_friction: 1.0, liquid_drag: 0.9, pass_through: ["gas"],
      displace_liquid: false,
      bounces: 0, bounce_energy: 0.0,
      physics_impulse: 20.0,
      on_lifetime_out_explode: false,
    ),
  ],
)
```

- [ ] **Step 5: 实现加载器**

`scenario.rs::load_spells`，与 `load_reactions` 同体例：

- 指纹 = `xxh3_64(normalize_for_fingerprint(bytes))`。
- `damage` / `knockback` / `mana` / `physics_impulse` → 千分位整数（`quantize_milli`，Task 3 已有）。
- `speed` / `gravity` / `air_friction` / `liquid_drag` / `bounce_energy` / Spray 的 `speed`/`jitter` → `quantize_fx`。
- `spread_deg` → BAM（`quantize_bam`，Task 1 已有），并**校验 `0 ≤ spread_deg ≤ 180`**，
  越界报可读错误（散布是双边的，> 180 无意义）。
- `pass_through` 的 `Category` 名 → 位掩码 `u8`；未知名报错。
- `Spray.material` 与 `Bolt/Blast` 无材质引用；`material` 名经 `table.id_by_name` 解析，未知即报错。
- **加载期契约**：`name` 不得重复（自定义 `MapAccess` 或加载后排序检查——ron 0.8 对重复键静默覆盖，
  同 `GridSpec.legend` 那次的教训）；法术数不得超过 255（id 是 u8，255 保留为"空槽"哨兵）。

- [ ] **Step 6: 实现 `spell::cast_all`**

```rust
/// 第 2d 步（spec §6）。按 creature id 序；每生物每 tick 至多一发。
pub fn cast_all(/* ... */) {
    for i in 0..creatures.len() {
        let c = &mut creatures.list[i];
        if !c.alive { continue; }
        // ① 收尾类更新先做，保证"本 tick 冷却好了就能放"
        for cd in c.cooldowns.iter_mut() { *cd = cd.saturating_sub(1); }
        c.mana = (c.mana + tpl.get(c.template).mana_regen_per_tick).min(c.mana_max);

        let inp = creatures.input_of(i, inputs);
        if !inp.held(BTN_FIRE) { continue; }
        let slot = inp.slot as usize;
        let sid = c.loadout[slot];
        if sid == SPELL_NONE { continue; }
        let s = spells.get(sid);
        if c.cooldowns[slot] > 0 || c.mana < s.mana { continue; }   // 双闸门，无副作用
        c.mana -= s.mana;
        c.cooldowns[slot] = s.cooldown;

        // ② 方向：aim + 散布骰（spread == 0 时**不掷骰**，保证零散布法术完全可预测）
        let mut a = c.aim;
        if s.spread_bam > 0 {
            let r = rng::rng_u32(fseed, rng::STREAM_SPREAD, i as i32, 0,
                                 slot as u32, shots_this_tick);
            a = a.wrapping_add(bam_in_range(r, s.spread_bam));   // 均匀落在 ±spread
        }
        let (dx, dy) = fixed::dir_of(a);

        // ③ 派发
        match s.kind {
            SpellKind::Spray { material, count, speed, jitter } => {
                // 不产生弹体：直接走既有 emit 通路，与 Op::Emit 同一队列同一语义
                emit::apply_emit(material, muzzle_x, muzzle_y,
                                 dx.mul(speed), dy.mul(speed), count, jitter,
                                 stamp, fseed, /* op_idx */ SPRAY_OP_IDX_BASE + i,
                                 spawns);
            }
            _ => {
                projectiles.spawn(sid, muzzle_x, muzzle_y,
                                  dx.mul(s.speed), dy.mul(s.speed),
                                  s.life, s.dig_power, i as u8, s.grace, s.bounces);
            }
        }
    }
}
```

`bam_in_range(r: u32, half: Bam) -> Bam`：把 32 位随机数**均匀**映射到
`[-half, +half]`——用 `((r as u64 * (2*half as u64 + 1)) >> 32) as i64 - half as i64`
（乘法-右移法，无取模偏置），再 `as u16`（二补码环绕即负角）。

`muzzle_x/y` = 生物中心沿 `(dx, dy)` 偏移 `tpl.muzzle_offset` 格。

- [ ] **Step 7: 实现 `Blast` 的命中结算**

`projectile.rs::resolve_hit_cell` / `resolve_hit_creature` 里补 `Blast` 分支：

```rust
SpellKind::Blast { power, radius, max_durability } => {
    // 走**现有**出口，零新增通路（spec §5.3）
    explode::apply_explode(world, table, gx, gy, radius, power, max_durability,
                           stamp, fseed, BLAST_OP_IDX_BASE + i, spawns);
    bodies.pending_blasts.push((gx, gy, radius));
}
```

`BLAST_OP_IDX_BASE` / `SPRAY_OP_IDX_BASE`：与 ops 阶段的 `op_idx` 值域**不得重叠**——
两者共用 `STREAM_EXPLODE` / `STREAM_EMIT` 的 salt 维度，重叠即掷出同值（RNG overlap）。
取 `BLAST_OP_IDX_BASE = 1 << 20`、`SPRAY_OP_IDX_BASE = 1 << 21`，并加单测断言
`ops` 阶段 `op_idx` 上界（场景 ops 数）远小于 `1 << 20`。

- [ ] **Step 8: 接线第 2d 步 + harness 加载**

`lib.rs::step` 在 `projectiles.advance` 之后：

```rust
// 2d. 施法结算（spec §6）：新弹体本 tick 不积分，下 tick 起飞。
spell::cast_all(&mut self.creatures, &mut self.projectiles, &mut self.world,
                &self.table, &self.spell_table, &self.creature_table,
                &mut self.bodies, inputs, stamp, fseed, &mut self.spawn_queue);
```

`main.rs` / `runner.rs` / `render.rs`：加载 `data/creatures.ron` 与 `data/spells.ron`，
两个指纹与 `materials_fp` / `reactions_fp` 一并进握手指纹的打印与比对。

场景 RON 的 `SpawnCreature` OpSpec 里 `loadout` 写法术名列表，加载期解析成 id 数组，
未知名报错、超过 `MAX_SLOTS` 报错。

- [ ] **Step 9: 跑测试**

Run: `cargo test -p sand-core --test projectile_behavior && cargo test -p sand-harness`
Expected: 18 条全 PASS。

- [ ] **Step 10: 全量 + lint + golden 重录 + 提交**

本 Task 把 `creatures_fp` / `spells_fp` 加进 `hashrun` 的指纹输出行（Task 1 刻意推迟到此，
避免录两次），故 **6 个 golden 需再重录一次**——这是本次唯一的合法重录理由，
仿真哈希列必须逐字不变：

```bash
S="sand_pile waterfall_ci mixed explosion_ci fire_oil_chain crate_yard"
for s in $S; do cargo run -q -p sand-harness --release -- hashrun data/scenarios/$s.ron \
  > /tmp/m4t5-$s.new; done
# 与旧 golden 逐行比对：只允许指纹行有差异，哈希行必须一致
for s in $S; do diff <(grep -v '_fp' crates/sand-harness/tests/golden/$s.golden) \
                     <(grep -v '_fp' /tmp/m4t5-$s.new) || echo "HASH DIFF $s"; done
for s in $S; do cargo run -q -p sand-harness --release -- hashrun data/scenarios/$s.ron \
  --write-golden crates/sand-harness/tests/golden/$s.golden; done
```
Expected: 无 `HASH DIFF` 输出。

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`

```bash
git commit -m "feat(core): M4 Task 5 法术表与施法——三原语、cooldown+mana 双闸门

spells.ron 扁平记录、无引用无递归（spec §3.4）；Bolt/Blast 产弹体，Spray 走
现有 emit 通路当帧出粒子。双闸门不通过时零副作用（不扣费、不置冷却）。
散布骰新增 STREAM_SPREAD=9，key=creature id、salt=槽位、attempt=本 tick 第几发
（翻案第 4 条纪律）；spread=0 时不掷骰。Blast 走现有 apply_explode +
pending_blasts，op_idx 值域与 ops 阶段隔离防 RNG overlap。行为测试 +10 条。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_017GZJc7RrRKLh4XgoJMUb4m"
```

---

## Task 6: 弹体七项扩展

七条互相独立，**逐条 TDD**：先写该条的失败测试 → 实现 → 跑绿 → 下一条。全部做完再提交一次。

**Files:**
- Modify: `crates/sand-core/src/projectile.rs`、`body.rs`（单点冲量 API）、`docs/tuning-knobs.md`
- Test: `crates/sand-core/tests/projectile_behavior.rs`（追加）

**Interfaces:**
- Consumes: Task 5 的 `SpellDef` 全字段
- Produces: `Bodies::apply_point_impulse(&mut self, phys: &mut PhysicsWorld, x: i32, y: i32, jx: f32, jy: f32)`

- [ ] **Step 1: `displace_liquid`（最便宜的先做）**

测试：

```rust
#[test]
fn displacing_projectile_pushes_liquid_out_of_its_path() {
    let mut sim = arena_with_loadout(&["bomb"]);              // displace_liquid: true
    sim.apply_setup(&[Op::Fill { material: WATER, x0: 50, y0: 60, x1: 60, y1: 70 }]);
    let before = sim.world().count_material(WATER);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    for _ in 0..30 { sim.step(&[], &[]); }
    assert!(sim.world().count_material(WATER) < before || sim.particles().len() > 0,
            "飞过水面应当把水推成粒子");
}
```

实现：`advance` 的 DDA 循环里，格材质为 `Liquid`/`Powder` 且 `s.displace_liquid` 时，
`set_cell_stamped(AIR)` + `spawns.push(SpawnRequest{ material, 格心, 弹体速度 })`——
与 Task 3 生物排开**同一条通路**。

- [ ] **Step 2: `pass_through` 掩码**

```rust
#[test]
fn pass_through_liquid_lets_the_projectile_cross_a_pool() {
    let mut sim = arena_with_loadout(&["digger"]);            // pass_through: gas + liquid
    sim.apply_setup(&[Op::Fill { material: WATER, x0: 40, y0: 55, x1: 45, y1: 75 }]);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    for _ in 0..20 { sim.step(&[], &[]); }
    assert!(sim.projectiles().len() == 1 && sim.projectiles().x(0).to_cell() > 45,
            "穿液体的弹体应当越过水池");
}
```

实现：DDA 循环里，若该格 `Category` 在掩码内 → `continue`（不算命中，也不做排开）。
注意判定顺序：`pass_through` 优先于 `displace_liquid`（穿过去就不推开）。

- [ ] **Step 3: `air_friction` / `liquid_drag`**

```rust
#[test]
fn air_friction_below_one_decelerates_the_projectile() {
    let mut sim = arena_with_loadout(&["slow_bolt"]);          // air_friction: 0.9
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    sim.step(&[], &[]);
    let v1 = sim.projectiles().vx(0);
    sim.step(&[], &[]);
    assert!(sim.projectiles().vx(0) < v1, "air_friction < 1 应当减速");
}

#[test]
fn liquid_drag_slows_a_projectile_inside_water_more_than_in_air() {
    // 两发独立开局，避免互相干扰：同法术同初速，一发穿空气、一发穿水池
    let travel = |flood: bool| {
        let mut sim = common::arena_wide_open(spell_table());
        if flood {
            sim.apply_setup(&[Op::Fill { material: WATER, x0: 10, y0: 40, x1: 240, y1: 90 }]);
        }
        shoot(&mut sim, sim.spell_id("wet_bolt"), 12, 64, Fx::from_int(6), Fx::ZERO, 255);
        let x0 = sim.projectiles().x(0);
        for _ in 0..20 { sim.step(&[], &[]); }
        assert_eq!(sim.projectiles().len(), 1, "20 tick 内不该撞到东西");
        sim.projectiles().x(0) - x0
    };
    assert!(travel(true) < travel(false), "水里应当飞得更近");
}
```

`spells.ron` 追加两条测试用法术：`slow_bolt`（`air_friction: 0.9`、`liquid_drag: 1.0`）
与 `wet_bolt`（`air_friction: 1.0`、`liquid_drag: 0.7`、`pass_through: ["gas","liquid"]`
——**必须穿液体**，否则它会在入水第一格就命中而不是被减速）。

实现：`advance` 开头，`vy += gravity` 之后：
`(vx, vy) *= air_friction`；若**起点格**是 `Liquid` 则再 `*= liquid_drag`。
`spells.ron` 追加 `slow_bolt` 条目（`air_friction: 0.9`）供测试用。

- [ ] **Step 4: `on_lifetime_out_explode`**

```rust
#[test]
fn timed_blast_explodes_when_lifetime_runs_out_even_without_hitting() {
    let mut sim = arena_wide_open_with_loadout(&["bomb"]);     // on_lifetime_out_explode: true
    sim.apply_setup(&[Op::Fill { material: STONE, x0: 100, y0: 60, x1: 110, y1: 70 }]);
    let before = sim.world().count_material(STONE);
    // 让它在石块附近寿命耗尽（bomb 有重力，会落在石块上方空中耗尽）
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    for _ in 0..200 { sim.step(&[], &[]); }
    assert!(sim.world().count_material(STONE) < before, "寿命耗尽也要炸");
}
```

实现：`life` 归零分支里，若 `s.on_lifetime_out_explode` 且 `kind` 是 `Blast`，
在当前格触发与 §5.3 完全相同的爆炸结算。

- [ ] **Step 5: 侵彻（`dig_power` + `max_durability`）**

```rust
#[test]
fn digger_bores_into_stone_and_stops_when_energy_is_spent() {
    let mut sim = arena_with_loadout(&["digger"]);
    sim.apply_setup(&[Op::Fill { material: STONE, x0: 50, y0: 0, x1: 90, y1: 127 }]);
    let before = sim.world().count_material(STONE);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    for _ in 0..60 { sim.step(&[], &[]); }
    let dug = before - sim.world().count_material(STONE);
    assert!(dug > 0, "挖掘弹必须挖穿一段");
    assert!(sim.world().cell(89, 64).material() == STONE, "能量有限，不得挖穿整堵墙");
    assert_eq!(sim.projectiles().len(), 0, "能量耗尽即销毁");
}

#[test]
fn wall_durability_gate_stops_the_digger_immediately() {
    // wall durability 15 > digger 的 max_durability 12 ⇒ 一格都挖不动
    let mut sim = arena_with_loadout(&["digger"]);
    sim.apply_setup(&[Op::Fill { material: MAT_WALL, x0: 50, y0: 0, x1: 60, y1: 127 }]);
    let before = sim.world().count_material(MAT_WALL);
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    for _ in 0..60 { sim.step(&[], &[]); }
    assert_eq!(sim.world().count_material(MAT_WALL), before, "门槛免疫，一格都不掉");
}
```

实现：DDA 循环命中硬格分支改为：

```rust
let m = world.cell(gx, gy).material();
if table.durability(m) > s.max_durability || self.energy[i] == 0 {
    self.resolve_hit_cell(...); alive = false; break;
}
let cost = table.hp(m);
if (self.energy[i] as u64) < cost as u64 {
    self.resolve_hit_cell(...); alive = false; break;
}
self.energy[i] -= cost;
// 删格：盖**当前** stamp，按 vaporize_threshold 决定汽化还是溅射成粒子
//（复用 explode.rs 里同一段判定，抽成 pub(crate) fn destroy_cell(...) 供两处共享）
destroy_cell(world, table, gx, gy, m, stamp, remaining_ratio, spawns);
// 继续沿路径飞
```

`destroy_cell` 从 `explode::fire_ray` 里抽出（纯搬移），两个调用方共享——避免
"能量射线三兄弟"的第四个用例重写一遍删格语义。

- [ ] **Step 6: 弹跳（`bounces` + `bounce_energy`）**

```rust
#[test]
fn bouncing_projectile_reflects_off_the_floor_and_dies_after_its_last_bounce() {
    let mut sim = arena_with_loadout(&["bomb"]);              // bounces: 2
    sim.step(&[], &[InputFrame::new(BTN_FIRE, /* 45° 下斜 */ 8192, 0)]);
    let mut sign_flips = 0;
    let mut prev = sim.projectiles().vy(0);
    for _ in 0..400 {
        sim.step(&[], &[]);
        if sim.projectiles().len() == 0 { break; }
        let v = sim.projectiles().vy(0);
        if prev > Fx::ZERO && v < Fx::ZERO { sign_flips += 1; }
        prev = v;
    }
    assert_eq!(sign_flips, 2, "应当恰好弹 2 次");
    assert_eq!(sim.projectiles().len(), 0, "弹完即销毁（bomb 会在此炸开）");
}

#[test]
fn bounce_energy_reduces_speed_each_time() {
    // 每次反弹后该轴速度大小 ≈ 前一次 × bounce_energy（容差 1/16 格）
    let mut sim = common::arena_wide_open(spell_table());
    shoot(&mut sim, sim.spell_id("bomb"), 40, 20, Fx::ZERO, Fx::from_int(4), 255);
    let mut speeds = Vec::new();
    let mut prev = sim.projectiles().vy(0);
    for _ in 0..400 {
        sim.step(&[], &[]);
        if sim.projectiles().len() == 0 { break; }
        let v = sim.projectiles().vy(0);
        if prev > Fx::ZERO && v < Fx::ZERO {
            speeds.push((prev, v));            // (撞前向下速度, 反弹后向上速度)
        }
        prev = v;
    }
    assert_eq!(speeds.len(), 2, "bomb 的 bounces = 2");
    let e = Fx::from_ratio(4, 10);             // bounce_energy 0.4
    let tol = Fx::from_ratio(1, 16);
    for (before, after) in speeds {
        let want = before.mul(e);
        let got = Fx(-after.0);                // 取绝对值（反弹后是负的）
        assert!((got - want).0.abs() < tol.0, "反弹衰减不符：{got:?} vs {want:?}");
    }
}
```

实现：`CellWalk` 需要额外吐出"这一步跨的是哪根轴"。给 `dda.rs` 加
`pub(crate) enum Axis { X, Y }` 与 `CellWalk::last_axis(&self) -> Axis`（`next()` 里记录），
**不改动既有 `Iterator` 语义**，粒子与爆炸两个既有调用方一行不改。

命中硬格且 `bounces > 0` 时：按 `last_axis` 取反该轴速度并乘 `bounce_energy`，
`bounces -= 1`，用**新速度**从当前位置重开一次 `CellWalk` 走完本 tick 剩余位移
（剩余量 = 原位移 − 已走部分；实现取"从撞击格前一格重新起步，剩余速度按比例"，
比例用整数：`remaining = total_cells_left / total_cells`，避免除法链——直接
用"本 tick 最多重开 `MAX_BOUNCE_RESTARTS = 4` 次、每次消耗剩余步数"的循环）。

- [ ] **Step 7: 刚体单点冲量（`physics_impulse`）**

```rust
#[test]
fn projectile_pushes_a_rigid_body_it_hits() {
    let mut sim = arena_with_loadout(&["expensive_bolt"]);    // physics_impulse: 20
    sim.apply_setup(&[Op::SpawnBody { material: WOOD, x: 60, y: 60, w: 12, h: 12, angle_deg: 0 }]);
    let x0 = sim.body_state(0).unwrap().0.0;
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
    for _ in 0..60 { sim.step(&[], &[]); }
    assert!(sim.body_state(0).unwrap().0.0 > x0, "射中的箱子应当被推走");
}
```

实现：`body.rs` 加

```rust
/// 单点冲量（M4 spec §5.5，Noita `physics_impulse_coeff`：Impulse = coeff × velocity）。
/// 与 `apply_blast` 共用 `phys.apply_impulse_at`，区别只是作用点由调用方给定、
/// 不做半径加权。命中格属于哪个 body 由盖章清单反查。
pub(crate) fn apply_point_impulse(&mut self, phys: &mut PhysicsWorld,
                                  x: i32, y: i32, jx: f32, jy: f32) {
    if let Some(bi) = self.body_index_at(x, y) {
        phys.apply_impulse_at(self.list[bi].handle, (jx, jy), (x as f32 + 0.5, y as f32 + 0.5));
    }
}
```

`body_index_at(x, y)`：按 body id 序查盖章清单（线性即可，body 数以十计）。
弹体侧：命中格 `cell.is_body()` 且 `s.physics_impulse > 0` 时调用，冲量 =
`physics_impulse × 弹体速度`（浮点转换只发生在 `physics` 适配层边界，与
`apply_blast` 同一处理）。

- [ ] **Step 8: 跑全部弹体测试 + 更新 tuning-knobs**

Run: `cargo test -p sand-core --test projectile_behavior`
Expected: 全绿。

`docs/tuning-knobs.md` 新增 **§8 M4 生物与法术旋钮**：`CREATURE_MAX_STEP`、
`MAX_CREATURES`、`MAX_PROJECTILES`、`MAX_BOUNCE_RESTARTS`、以及 `creatures.ron` /
`spells.ron` 的每个字段（现值、拧它的后果、A/B/C 分类）。表格体例照 §6 M3 那节。

- [ ] **Step 9: 全量 + lint + 提交**

```bash
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
git commit -m "feat(core): M4 Task 6 弹体七项扩展——侵彻/弹跳/阻力/穿透/排开/冲量/定时爆

侵彻是「能量射线三兄弟」的第四个同构用例：durability 门槛 + hp 能量池，
删格逻辑从 explode::fire_ray 抽出 destroy_cell 两处共享。弹跳法线取自 DDA
撞击轴（dda 加 last_axis，既有两个调用方零改动）。刚体单点冲量补上 M3
「箱子能被炸飞却不能被射中」的缺口。tuning-knobs 新增 §8。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_017GZJc7RrRKLh4XgoJMUb4m"
```

---

## Task 7: 收口

**Files:**
- Create: `data/scenarios/duel.ron`、`docs/perf/2026-09-05-m4-player-and-spells.md`
- Modify: `crates/sand-core/tests/synctest_ci.rs`、`docs/overview/kernel-charter.md`、
  `docs/overview/program-architecture.md`、`docs/README.md`、`docs/CHANGELOG.md`、
  `docs/superpowers/specs/2026-09-05-m4-player-and-spells-design.md`（Status → Implemented）

- [ ] **Step 1: 写 `data/scenarios/duel.ron`**

骨架（spec §7.3）——地形用既有 `grid` 字段（地图编辑器那条通路）或 `setup` 的 `Fill` 均可：

```ron
(
  name: "duel",
  world: (4, 2),          // 256×128
  seed: 20260905,
  ticks: 3000,
  setup: [
    Fill( material: "wall",  x0: 0,   y0: 127, x1: 255, y1: 127 ),  // 地板
    Fill( material: "wall",  x0: 0,   y0: 0,   x1: 0,   y1: 127 ),  // 左墙
    Fill( material: "wall",  x0: 255, y0: 0,   x1: 255, y1: 127 ),  // 右墙
    Fill( material: "stone", x0: 120, y0: 80,  x1: 135, y1: 126 ),  // 中央石墙（可炸/可钻）
    Fill( material: "water", x0: 20,  y0: 118, x1: 70,  y1: 126 ),  // 左侧水池
    Fill( material: "oil",   x0: 180, y0: 122, x1: 230, y1: 126 ),  // 右侧油滩
    SpawnCreature( x: 30,  y: 110, template: "player", team: 0, controller: 0,
                   loadout: ["spark_bolt", "bomb", "oil_spray", "digger"] ),
    SpawnCreature( x: 220, y: 110, template: "player", team: 1, controller: 1,
                   loadout: ["spark_bolt", "bomb", "oil_spray", "digger"] ),
  ],
  script: [],
  inputs: [
    // tick, [controller 0 的帧, controller 1 的帧]
    ( tick: 0,    frames: [ (right: true),                       (left: true) ] ),
    ( tick: 120,  frames: [ (right: true, jump: true),           (left: true) ] ),
    // ① 0 号趟过水池（tick 0–300 一路向右）
    ( tick: 300,  frames: [ (fire: true, slot: 3, aim_deg: 0.0), (fire: true, slot: 1, aim_deg: 180.0) ] ),
    // ③ 挖掘弹钻石墙 / 炸弹砸过来
    ( tick: 900,  frames: [ (fire: true, slot: 1, aim_deg: 350.0), () ] ),
    // ④ 1 号往自己脚下浇油、0 号打火弹点燃
    ( tick: 1500, frames: [ (),                                  (fire: true, slot: 2, aim_deg: 90.0) ] ),
    ( tick: 1900, frames: [ (fire: true, slot: 0, aim_deg: 5.0), () ] ),
    // ⑤ 收尾：0 号持续射击直至 1 号死亡
    ( tick: 2200, frames: [ (fire: true, slot: 0, aim_deg: 0.0), () ] ),
    ( tick: 2900, frames: [ (),                                  () ] ),
  ],
)
```

实施者按实际手感调 tick 与角度，**只要五项都在 3000 tick 内被覆盖**：
① 走过水面 ② 炸墙 ③ 挖掘弹钻石头 ④ 浇油点燃连锁 ⑤ 一方被打死。
`Op::SpawnCreature` 的 `template`/`loadout` 在场景 RON 里写**名字**，加载期解析成 id。

- [ ] **Step 2: 端到端行为测试（验收 §7.2 第 7 条）**

`tests/projectile_behavior.rs` 追加——这条是"环境连锁"卖点的第一个可测形态：

```rust
#[test]
fn oil_spray_then_bolt_ignites_a_chain() {
    let mut sim = arena_with_loadout(&["oil_spray", "fire_bolt"]);
    // ① 往地上浇一大片油
    for _ in 0..40 { sim.step(&[], &[InputFrame::new(BTN_FIRE, /* 略向下 */ 4096, 0)]); }
    for _ in 0..120 { sim.step(&[], &[]); }                  // 让油落地摊开
    let oil_before = sim.world().count_material(OIL);
    assert!(oil_before > 50, "应当先铺出一片油");
    // ② 打一发火弹点燃
    sim.step(&[], &[InputFrame::new(BTN_FIRE, 4096, 1)]);
    for _ in 0..600 { sim.step(&[], &[]); }
    assert!(sim.world().count_material(OIL) < oil_before / 2, "油应当被连锁烧掉大半");
}
```

（`fire_bolt` = `spells.ron` 追加的一条 `Bolt`，命中格触发一小半径 `Blast`
或直接把命中格换成 fire——按 §3.4 三原语约束，用 `Blast{ power 小, radius 2 }` 表达。）

- [ ] **Step 3: 散布角分布回归（新规矩，spec §7.2）**

```rust
/// RNG salt/attempt 维度缺失类 bug 两端一样地错，SyncTest 抓不到——本测试是
/// 唯一防线（Noita 宝箱事故先例：`noita-grid-api-and-rng.md` §5.2）。
#[test]
fn spread_angle_is_uniform_within_the_declared_cone() {
    const BINS: usize = 10;
    const SHOTS: usize = 5000;
    let spread: i32 = 30;                                    // 度，spells.ron 的 scatter_bolt
    let half_bam = (spread as i64 * 65536 / 360) as i32;     // ±half_bam
    let mut hist = [0u32; BINS];
    let mut sim = common::arena_wide_open_with_shooter(spell_table());
    let sid = sim.spell_id("scatter_bolt");
    let mut fired = 0usize;
    let mut t = 0u64;
    while fired < SHOTS {
        // scatter_bolt 的 cooldown 设为 1，故每 tick 出一发；aim 恒 0（+x）
        sim.step(&[], &[InputFrame::new(BTN_FIRE, 0, 0)]);
        t += 1;
        assert!(t < 200_000, "取样太慢，检查 cooldown 配置");
        for i in 0..sim.projectiles().len() {
            if sim.projectiles().spell(i) != sid { continue; }
            // 出射角 = atan2 的替代：直接用 vy/|v| 的符号与比例落腔太糙，
            // 改为记录**弹体出生 tick 的速度分量**，按 vy 相对 vx 的比例落腔。
            // 由于 |角| ≤ 30°，vx > 0 恒成立，vy/vx 单调映射角度。
            let (vx, vy) = (sim.projectiles().vx(i), sim.projectiles().vy(i));
            let ratio = (vy.0 as i64) * 32768 / (vx.0 as i64);   // 定点比例
            let lo = -(half_bam as i64) * 32768 / 65536 * 2;     // 近似边界
            let b = (((ratio - lo) * BINS as i64) / (-2 * lo)).clamp(0, BINS as i64 - 1);
            hist[b as usize] += 1;
            fired += 1;
        }
        // 每 tick 清空弹体池，避免重复计数：让 scatter_bolt 的 life = 1
    }
    let n = SHOTS as f64;
    let p = 1.0 / BINS as f64;
    let (mu, sigma) = (n * p, (n * p * (1.0 - p)).sqrt());
    for (i, &c) in hist.iter().enumerate() {
        assert!((c as f64 - mu).abs() < 4.0 * sigma,
                "第 {i} 腔 {c} 偏离均匀分布（期望 {mu:.0} ± {:.0}）", 4.0 * sigma);
    }
}
```

`spells.ron` 追加 `scatter_bolt`：`spread_deg: 30`、`cooldown: 1`、`mana: 0`、`life: 1`
（出生即计数、下一 tick 即销毁，避免重复计入）。**这条法术只服务本测试**，
在 `spells.ron` 里加注释说明，`duel.ron` 不用它。

- [ ] **Step 4: SyncTest 与线程不变性**

`tests/synctest_ci.rs` 把 `duel` 加入场景清单（六配置 × 2 万 tick）。

Run:
```bash
cargo test -p sand-core --test synctest_ci --release
cargo run -q -p sand-harness --release -- hashrun data/scenarios/duel.ron --threads 1  > /tmp/duel.t1
cargo run -q -p sand-harness --release -- hashrun data/scenarios/duel.ron --threads 8  > /tmp/duel.t8
cargo run -q -p sand-harness --release -- hashrun data/scenarios/duel.ron --threads 16 > /tmp/duel.t16
diff /tmp/duel.t1 /tmp/duel.t8 && diff /tmp/duel.t1 /tmp/duel.t16
```
Expected: 零分叉、三份哈希流逐字相同。

- [ ] **Step 5: golden 与 bench**

**没有 `bench` 子命令**——性能数字取自 `hashrun` 收尾打印的 `tick 耗时 avg / max`
（既有 perf 文档同源）。

```bash
# duel 的 golden（Task 5 已把 creatures_fp / spells_fp 加进输出行，此处一次录全）
cargo run -q -p sand-harness --release -- hashrun data/scenarios/duel.ron \
  --write-golden crates/sand-harness/tests/golden/duel.golden
# 全部场景重录（Task 5 改了指纹输出行）
for s in sand_pile waterfall_ci mixed explosion_ci fire_oil_chain crate_yard duel; do
  cargo run -q -p sand-harness --release -- hashrun data/scenarios/$s.ron \
    --write-golden crates/sand-harness/tests/golden/$s.golden
done
# 性能：每个场景跑 3 次取中位，记 avg/max
for s in sand_pile mixed crate_yard duel; do
  for i in 1 2 3; do
    cargo run -q -p sand-harness --release -- hashrun data/scenarios/$s.ron 2>&1 >/dev/null \
      | grep "tick 耗时"
  done
done
```

结果落 `docs/perf/2026-09-05-m4-player-and-spells.md`，对照口径照
`docs/perf/2026-09-02-m3-rigid-body.md`：每场景 M4 前 / 后的 avg·max ms/tick。
**既有场景不得回退**——无生物无弹体的场景里第 2 步是两个空循环，
若 ms/tick 有可测上升，停下排查而不是记一笔了事。

- [ ] **Step 6: 文档同步**

- `kernel-charter.md` §11 新增**实施期决策第 18 条**：M4 管线第 2 步生效（协议版本变更）；
  `combine3 → combine4`；总纲 §4 "挂 payload 的粒子"措辞澄清；M4 范围收窄（stain 顺延）；
  待决项"法术表达力是否升级为脚本 VM"本轮判定不升级、判定时点顺延；
  明确记载**未触发**翻案第 6 条复议。
- `program-architecture.md` §3 子系统清单的 `entities & spells` 行改为已落地并给锚点；
  §4 管线第 2 步补四个子步骤。
- `docs/README.md` 优先队列：新增 `5.` 条 M4 完成记录，"下一步 = M5 联机对局"。
- `docs/CHANGELOG.md` 顶部 2026-09-05 块补 `Added` 条目（逐 Task 一行 + 受影响文件路径）。
- spec Status → **Implemented**；两份 plan Status → **Implemented**。

- [ ] **Step 7: 最终验证与提交**

Run:
```bash
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
```
Expected: 全绿。

```bash
git commit -m "feat(core): M4 收口——duel 场景、SyncTest 六配置、油火连锁端到端

duel.ron（两生物 + 输入时间线 + 水/油/石墙，3000 tick）入 golden 与 SyncTest；
油火连锁端到端测试是「环境连锁」卖点的第一个可测形态；散布角分布回归 10 腔
4σ（RNG 维度缺失类 bug 的唯一防线）。线程 1/8/16 逐位相同。总纲 §11 实施期
决策第 18 条、架构 §3/§4、README 优先队列、tuning-knobs §8、perf 全部同步。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_017GZJc7RrRKLh4XgoJMUb4m"
```

- [ ] **Step 8: 交付目检**

用 `sand-harness render data/scenarios/duel.ron` 出 GIF，交用户目检签收（验收第 6 项）。
**subagent 不得在终端调 Godot**——GIF 走 harness 的 PPM/GIF 渲染路径，与既有场景同法。
