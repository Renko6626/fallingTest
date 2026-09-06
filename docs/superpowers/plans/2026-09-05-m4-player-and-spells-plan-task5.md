> 文档路径：`docs/superpowers/plans/2026-09-05-m4-player-and-spells-plan-task5.md`
> 运行时版本：Rust（内核）+ Godot 4 + gdext（表现层）
> 最近更新：2026-09-05 (UTC+8)
> **Status**: Implemented
> 总纲：`2026-09-05-m4-player-and-spells-plan.md`（Goal / Architecture / **Global Constraints** / File Structure / Task 索引）

# M4 · Task 5：法术表与施法

> **For agentic workers:** 本文只含一个 Task。**开工前必读总纲的 Global Constraints 全节**
> ——它是本 Task 验收的隐含组成部分。
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

