> 文档路径：`docs/superpowers/plans/2026-09-05-m4-player-and-spells-plan-task4.md`
> 运行时版本：Rust（内核）+ Godot 4 + gdext（表现层）
> 最近更新：2026-09-05 (UTC+8)
> **Status**: Implemented
> 总纲：`2026-09-05-m4-player-and-spells-plan.md`（Goal / Architecture / **Global Constraints** / File Structure / Task 索引）

# M4 · Task 4：弹体载体

> **For agentic workers:** 本文只含一个 Task。**开工前必读总纲的 Global Constraints 全节**
> ——它是本 Task 验收的隐含组成部分。
> **Spec:** `docs/superpowers/specs/2026-09-05-m4-player-and-spells-design.md`

---

## Task 4: 弹体载体

**Files:**
- Modify: `crates/sand-core/src/projectile.rs`（填实）、`lib.rs`
- Test: `crates/sand-core/tests/projectile_behavior.rs`（新建）

**Interfaces:**
- Consumes: Task 2/3 的 `Creatures`（命中目标）、`material::is_solid`
- Produces:
  - `projectile::{Projectiles, MAX_PROJECTILES}`
  - `Projectiles::spawn(&mut self, spell: u8, x: Fx, y: Fx, vx: Fx, vy: Fx, life: u16, energy: u32, owner: u8, grace: u8, bounces: u8) -> bool`（`false` = 容量拒绝）
  - `Projectiles::advance(&mut self, world: &mut World, table: &MaterialTable, spells: &SpellTable, creatures: &mut Creatures, bodies: &mut Bodies, phys: &mut PhysicsWorld, stamp: u8, fseed: u32, spawns: &mut Vec<SpawnRequest>)`
  - `Projectiles::len(&self) -> usize`、`hash_into(&self) -> u64`、按下标只读访问器 `x/y/vx/vy/spell/life/energy/owner/bounces`（体例照 `particle.rs` 的 `Particles::x(i)`）
  - `Sim::projectiles(&self) -> &Projectiles`
  - `Sim::queue_projectile(&mut self, spell: u8, x: Fx, y: Fx, vx: Fx, vy: Fx, owner: u8) -> bool`
    ——`pub`，文档标注"供测试与未来的诊断工具"，体例照既有 `Sim::queue_spawn`；
    其余字段（`life`/`energy`/`grace`/`bounces`）从法术表取
  - `Creatures::first_hit_at(&self, gx: i32, gy: i32, owner: u8, grace: u8, owner_team: u8) -> Option<u8>`

  - `spell::{SpellDef, SpellKind, SpellTable}` 的**核心类型本体**（`SpellKind` 本 Task 只有 `Bolt`
    变体，`Blast`/`Spray` 由 Task 5 追加）；`SpellTable::from_defs(Vec<SpellDef>) -> SpellTable`、
    `SpellTable::get(&self, id: u8) -> &SpellDef`
    ——**类型住在 core、加载器住在 harness**，与 `MaterialTable` / `ReactionTable` 完全同分工。

本 Task 只实现**直线飞行 + 命中判定 + `Bolt` 结算**；`Blast`/`Spray` 变体、RON 加载器与
施法闸门在 Task 5，七项扩展在 Task 6。测试用的法术表由 `SpellTable::from_defs` 就地构造，
不依赖 `spells.ron`。

- [ ] **Step 1: 写失败的行为测试**

新建 `crates/sand-core/tests/projectile_behavior.rs`：

```rust
mod common;
use sand_core::{input::*, spell::*, world::Op, *};

/// 本 Task 的测试法术表：0 号 = 普通直射弹（life 长），1 号 = 短命弹（life 5）。
fn bolt_table() -> SpellTable {
    SpellTable::from_defs(vec![
        SpellDef::test_bolt("bolt", /* damage_milli */ 5_000, /* knockback */ Fx::from_int(2),
                            /* speed */ Fx::from_int(8), /* life */ 120, /* grace */ 4),
        SpellDef::test_bolt("shortlived", 5_000, Fx::ZERO, Fx::from_int(8), 5, 4),
    ])
}

/// 弹体注入：走 Sim::queue_projectile（内部就是 Projectiles::spawn），
/// 速度由调用方给格/tick，`Fx::from_ratio` 构造，不用浮点。
fn shoot(sim: &mut Sim, spell: u8, x: i32, y: i32, vx: Fx, vy: Fx, owner: u8) {
    let (px, py) = (Fx::from_int(x) + fixed::HALF_CELL, Fx::from_int(y) + fixed::HALF_CELL);
    sim.queue_projectile(spell, px, py, vx, vy, owner);
}

#[test]
fn projectile_flies_straight_and_dies_on_wall() {
    let mut sim = common::arena_wide_open(bolt_table());     // 四周 wall 的空场
    shoot(&mut sim, 0, 10, 64, Fx::from_int(8), Fx::ZERO, 255);
    for _ in 0..60 { sim.step(&[], &[]); }
    assert_eq!(sim.projectiles().len(), 0, "撞墙后必须销毁");
}

#[test]
fn projectile_damages_a_creature_it_hits() {
    let mut sim = common::arena_with_two_creatures(bolt_table());  // id 0 在左，id 1 在右
    let hp0 = sim.creatures().get(1).unwrap().hp;
    shoot(&mut sim, 0, 30, 64, Fx::from_int(8), Fx::ZERO, 0);
    for _ in 0..40 { sim.step(&[], &[]); }
    assert!(sim.creatures().get(1).unwrap().hp < hp0, "命中应当扣血");
    assert_eq!(sim.projectiles().len(), 0, "命中生物后弹体销毁");
}

#[test]
fn projectile_knockback_pushes_the_target() {
    let mut sim = common::arena_with_two_creatures(bolt_table());
    let x0 = sim.creatures().get(1).unwrap().x;
    shoot(&mut sim, 0, 30, 64, Fx::from_int(8), Fx::ZERO, 0);
    for _ in 0..40 { sim.step(&[], &[]); }
    assert!(sim.creatures().get(1).unwrap().x > x0, "击退应把目标推开");
}

#[test]
fn projectile_does_not_hit_its_owner_during_grace() {
    let mut sim = common::arena_with_two_creatures(bolt_table());
    let hp0 = sim.creatures().get(0).unwrap().hp;
    // 出生就在 owner 身上、速度指向它自己：grace = 4，前 3 tick 不得自伤
    shoot(&mut sim, 0, 20, 64, Fx::from_int(-1), Fx::ZERO, 0);
    for _ in 0..3 { sim.step(&[], &[]); }
    assert_eq!(sim.creatures().get(0).unwrap().hp, hp0, "grace 帧内不得自伤");
}

#[test]
fn projectile_skips_same_team() {
    let mut sim = common::arena_with_two_creatures_same_team(bolt_table());
    let hp0 = sim.creatures().get(1).unwrap().hp;
    shoot(&mut sim, 0, 30, 64, Fx::from_int(8), Fx::ZERO, 0);
    for _ in 0..40 { sim.step(&[], &[]); }
    assert_eq!(sim.creatures().get(1).unwrap().hp, hp0, "同队不得命中");
}

#[test]
fn projectile_dies_when_lifetime_runs_out() {
    let mut sim = common::arena_wide_open(bolt_table());
    shoot(&mut sim, 1, 100, 64, Fx::ZERO, Fx::ZERO, 255);    // 1 号：life 5，静止不撞墙
    for _ in 0..6 { sim.step(&[], &[]); }
    assert_eq!(sim.projectiles().len(), 0, "寿命耗尽即销毁");
}

#[test]
fn spawn_beyond_capacity_is_rejected_deterministically() {
    let mut sim = common::arena_wide_open(bolt_table());
    for _ in 0..MAX_PROJECTILES + 10 {
        shoot(&mut sim, 0, 100, 64, Fx::ZERO, Fx::ZERO, 255);
    }
    assert_eq!(sim.projectiles().len(), MAX_PROJECTILES, "超限必须确定性拒绝");
}

#[test]
fn projectile_moves_exactly_once_per_tick() {
    let mut sim = common::arena_wide_open(bolt_table());
    shoot(&mut sim, 0, 10, 64, Fx::from_int(8), Fx::ZERO, 255);
    let x0 = sim.projectiles().x(0);
    sim.step(&[], &[]);
    let x1 = sim.projectiles().x(0);
    assert_eq!(x1 - x0, Fx::from_int(8), "每 tick 恰好走一次速度，不多不少");
}
```

`SpellDef::test_bolt(...)` 是 core 侧的构造助手（`pub`，文档标注"测试与程序化构表用"），
其余字段取 Task 6 的中性缺省（`air_friction = 1`、`dig_power = 0`、`bounces = 0` …）。
"新生弹体本 tick 不动"这条语义由 Task 5 的施法路径覆盖（见 Task 5 测试），
本 Task 的注入路径绕过 2d，故此处只断言"每 tick 走一次"。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p sand-core --test projectile_behavior`
Expected: FAIL（编译错误）。

- [ ] **Step 3: 实现 `Projectiles` SoA**

体例完全照抄 `particle.rs`：SoA、下标即 id、`retain` 语义的保序压缩、容量拒绝计数器
**不入哈希**只供诊断。列：`x, y, vx, vy: Fx`、`spell: u8`、`life: u16`、`energy: u32`、
`owner: u8`、`grace: u8`、`bounces: u8`。

**死亡标记不额外开列**：一律 `life[i] = 0`，`compact()` 保留 `life > 0` 的项。
这样"命中而死"与"寿命耗尽"走同一条出口，也少一列进哈希。

同时补 `tests/common/mod.rs` 的三个 arena 助手（Task 2 的 `floor_world_with_creature` 之外）：

```rust
/// 256×128 空场，四周 wall 一圈；无生物。法术表由调用方给（core 侧程序化构表）。
pub fn arena_wide_open(spells: SpellTable) -> Sim { /* Fill 四边 wall */ }
/// arena_wide_open + 两个生物：id 0 在 (20,64) team 0、id 1 在 (200,64) team 1，
/// 两者 controller 均为 255（不吃输入，站着不动）。
pub fn arena_with_two_creatures(spells: SpellTable) -> Sim { /* 两条 Op::SpawnCreature */ }
/// 同上但两者 team 都是 0（同队跳过测试用）。
pub fn arena_with_two_creatures_same_team(spells: SpellTable) -> Sim { /* ... */ }
```

Task 2 的 `floor_world_with_creature` 同步改为 `floor_world_with_creature(tpl: CreatureTable,
spells: SpellTable)`；Task 2/3 的测试里包一层 `fn floor_world() -> (Sim, u8)`
传 `default_creature_table()` 与 `SpellTable::empty()`。**helper 一次定型，
后续 Task 只加参数不改语义**。

- [ ] **Step 4: 实现 `advance`**

```rust
/// 第 2c 步（spec §5.1）。按下标序（= id 序）。
pub fn advance(&mut self, world: &mut World, table: &MaterialTable, spells: &SpellTable,
               creatures: &mut Creatures, bodies: &mut Bodies, stamp: u8, fseed: u32,
               spawns: &mut Vec<SpawnRequest>) {
    for i in 0..self.len() {
        let s = spells.get(self.spell[i]);
        self.vy[i] = self.vy[i] + s.gravity;
        // Task 6 在此插入 air_friction / liquid_drag
        let mut alive = true;
        let (pos, vel) = ((self.x[i], self.y[i]), (self.vx[i], self.vy[i]));
        for (gx, gy) in dda::CellWalk::new(pos, vel) {
            if !world.in_bounds(gx, gy) { alive = false; break; }        // 出界即销毁
            // ① 生物：按 creature id 序，命中即结算
            if let Some(cid) = creatures.first_hit_at(gx, gy, self.owner[i],
                                                      self.grace[i], /* team */) {
                self.resolve_hit_creature(i, cid, s, creatures);
                alive = false; break;
            }
            // ② 硬格：Task 6 在此插入侵彻与弹跳；本 Task 直接结算
            if material::is_solid(world.cell(gx, gy), table, true) {
                self.resolve_hit_cell(i, gx, gy, s, world, bodies, stamp, fseed, spawns);
                alive = false; break;
            }
        }
        if alive {
            self.x[i] = self.x[i] + self.vx[i];
            self.y[i] = self.y[i] + self.vy[i];
            self.life[i] = self.life[i].saturating_sub(1);       // Task 6 在归零处加寿命爆炸
        } else {
            self.life[i] = 0;                                    // 死亡标记 = life 归零
        }
        self.grace[i] = self.grace[i].saturating_sub(1);
    }
    self.compact();     // retain(life > 0)，保序，与 particle.rs::compact 同形
}
```

**测试用的直接生成**（`Sim::queue_projectile`）走同一个 `Projectiles::spawn`，
不另开路径——保证测试跑的就是产品代码。

`Creatures::first_hit_at(gx, gy, owner, grace, owner_team)`：按 id 升序找第一个
`alive` 且 AABB 覆盖 `(gx, gy)` 的生物；`id == owner && grace > 0` 跳过；
同 `team` 跳过。返回 `Option<u8>`。

- [ ] **Step 5: 接线第 2c 步**

`lib.rs::step` 在 `step_world_interaction` 之后、施法之前：

```rust
// 2c. 弹体相（spec §1.1）：读本 tick 已移动的生物位置。
self.projectiles.advance(&mut self.world, &self.table, &self.spell_table,
                         &mut self.creatures, &mut self.bodies, &mut self.physics,
                         stamp, fseed, &mut self.spawn_queue);
```

`Projectiles::hash_into` 折叠全部列（含 `energy`/`grace`/`bounces`），接进 `entity_hash`。

- [ ] **Step 6: 跑测试**

Run: `cargo test -p sand-core --test projectile_behavior`
Expected: 8 条全 PASS。

- [ ] **Step 7: 全量 + lint + 提交**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`

```bash
git commit -m "feat(core): M4 Task 4 弹体载体——SoA 表、DDA 命中、Bolt 结算

弹体独立于粒子池（spec §1.2：复用 dda/fixed 模块而非 Particles），SoA 体例
完全照抄 particle.rs（下标即 id、保序压缩、容量拒绝计数不入哈希）。DDA 沿
路径先到者优先：生物按 id 序、硬格用 material::is_solid(include_bodies=true)。
grace 帧防自伤、同队跳过、出界与寿命耗尽即销毁。行为测试 8 条。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_017GZJc7RrRKLh4XgoJMUb4m"
```

---

Task 5–7 见 `2026-09-05-m4-player-and-spells-plan-2.md`。
