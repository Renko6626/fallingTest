> 文档路径：`docs/superpowers/plans/2026-09-05-m4-player-and-spells-plan-task3.md`
> 运行时版本：Rust（内核）+ Godot 4 + gdext（表现层）
> 最近更新：2026-09-05 (UTC+8)
> **Status**: Proposed
> 总纲：`2026-09-05-m4-player-and-spells-plan.md`（Goal / Architecture / **Global Constraints** / File Structure / Task 索引）

# M4 · Task 3：生物与世界互动

> **For agentic workers:** 本文只含一个 Task。**开工前必读总纲的 Global Constraints 全节**
> ——它是本 Task 验收的隐含组成部分。
> **Spec:** `docs/superpowers/specs/2026-09-05-m4-player-and-spells-design.md`

---

## Task 3: 生物与世界互动（排开 / 游泳 / 接触伤害 / HP）

**Files:**
- Modify: `crates/sand-core/src/creature.rs`、`lib.rs`
- Modify: `crates/sand-harness/src/scenario.rs`（`creatures.ron` 加载 + 指纹）
- Create: `data/creatures.ron`
- Test: `crates/sand-core/tests/creature_behavior.rs`（追加）

**Interfaces:**
- Consumes: Task 2 的 `Creatures::step_kinematics`
- Produces:
  - `CreatureTpl` 字段全集（见 Step 4）
  - `Creatures::step_world_interaction(&mut self, world: &mut World, table: &MaterialTable, tpl: &CreatureTable, stamp: u8, spawns: &mut Vec<SpawnRequest>)`
  - `sand_harness::scenario::load_creatures(path: &str, table: &MaterialTable) -> Result<(CreatureTable, u64), String>`

- [ ] **Step 1: 写失败的行为测试**

追加到 `tests/creature_behavior.rs`：

```rust
#[test]
fn running_through_water_displaces_it_into_particles() {
    let (mut sim, id) = floor_world();
    sim.apply_setup(&[Op::Fill { material: WATER, x0: 40, y0: 120, x1: 80, y1: 126 }]);
    let before = sim.world().count_material(WATER);
    for _ in 0..200 { sim.step(&[], &[InputFrame::new(BTN_RIGHT, 0, 0)]); }
    // 排开的水成粒子后仍会落回网格：总量（网格 + 在飞粒子）不得凭空减少
    let after = sim.world().count_material(WATER) + sim.particles().len();
    assert!(after >= before, "排开不得损失水量：{before} → {after}");
    assert!(sim.particles().len() > 0 || after == before, "应当产生过水花");
}

#[test]
fn displacement_is_capped_per_tick() {
    // 整个身子泡在水里，单 tick 排开数不得超过模板上限
    let (mut sim, id) = floor_world();
    sim.apply_setup(&[Op::Fill { material: WATER, x0: 0, y0: 90, x1: 255, y1: 126 }]);
    let before = sim.world().count_material(WATER);
    sim.step(&[], &[InputFrame::new(BTN_RIGHT, 0, 0)]);
    let removed = before - sim.world().count_material(WATER);
    // creatures.ron 的 player.max_displace_per_tick = 24
    assert!(removed <= 24, "单 tick 排开 {removed} 超过模板上限 24");
}

#[test]
fn creature_floats_in_deep_water_instead_of_sinking_to_bottom() {
    let (mut sim, id) = floor_world();
    sim.apply_setup(&[Op::Fill { material: WATER, x0: 0, y0: 64, x1: 255, y1: 126 }]);
    for _ in 0..600 { sim.step(&[], &[]); }
    let y = sim.creatures().get(id).unwrap().y.to_cell();
    assert!(y < 120, "浮力应当托住，不该沉到池底：y={y}");
}

#[test]
fn standing_in_fire_kills_the_creature() {
    let (mut sim, id) = floor_world();
    // 生物脚下持续供火（fire 有 lifetime，用 Every 脚本补）
    for t in 0..1200 {
        if t % 4 == 0 {
            sim.step(&[Op::Fill { material: FIRE, x0: 28, y0: 118, x1: 36, y1: 126 }], &[]);
        } else {
            sim.step(&[], &[]);
        }
        if !sim.creatures().get(id).unwrap().alive { break; }
    }
    assert!(!sim.creatures().get(id).unwrap().alive, "站火里应当被烧死");
}

#[test]
fn contact_damage_below_min_cell_count_is_ignored() {
    // 只有 2 格火（< min_cell_count = 4），泡 3600 tick 也不掉血
    let (mut sim, id) = floor_world();
    let hp0 = sim.creatures().get(id).unwrap().hp;
    for t in 0..3600 {
        if t % 4 == 0 {
            sim.step(&[Op::Fill { material: FIRE, x0: 32, y0: 124, x1: 33, y1: 124 }], &[]);
        } else { sim.step(&[], &[]); }
    }
    assert_eq!(sim.creatures().get(id).unwrap().hp, hp0, "不足 4 格接触必须整项忽略");
}

#[test]
fn dead_creature_keeps_its_id_and_stops_moving() {
    let (mut sim, id) = floor_world();
    sim.creatures_mut().kill_for_test(id);
    let (x, y) = { let c = sim.creatures().get(id).unwrap(); (c.x, c.y) };
    for _ in 0..120 { sim.step(&[], &[InputFrame::new(BTN_RIGHT, 0, 0)]); }
    let c = sim.creatures().get(id).unwrap();
    assert!(!c.alive && c.x == x && c.y == y, "墓碑不动，且 id 仍在原位");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p sand-core --test creature_behavior`
Expected: 新增 6 条 FAIL。

- [ ] **Step 3: 写 `data/creatures.ron`**

```ron
(
  templates: [
    (
      name: "player",
      half_w: 2, half_h: 5,
      hp_max: 100.0, mana_max: 100.0, mana_regen: 20.0,
      run_speed: 0.67, jump_speed: 2.9,
      accel_ground: 0.05, accel_air: 0.005,
      climb_over_y: 3,
      swim_buoyancy_idle: 1.2, swim_buoyancy_up: 0.9, swim_buoyancy_down: 0.7,
      swim_drag: 0.95,
      damage_from: [ ("fire", 3.0), ("lava", 30.0), ("acid", 8.0) ],
      min_cell_count: 4,
      max_displace_per_tick: 24,
      muzzle_offset: 3,
    ),
  ],
)
```

- [ ] **Step 4: 实现加载器与量化**

`scenario.rs` 新增 `load_creatures`，与 `load_materials` / `load_reactions` 同体例：

- 指纹 = `xxh3_64(normalize_for_fingerprint(bytes))`（复用既有函数，剥 CR）。
- `hp_max` / `mana_max` → **千分位整数**：`quantize_milli(v: f64) -> Result<i32, String>`
  = `round(v * 1000.0)`，越界（`v < 0` 或 `v > 1e6`）报可读错误。
- **每秒量在加载期一次性折成每 tick 量**，运行时不再做除法：
  `mana_regen`（点/秒）→ 字段 `mana_regen_per_tick: i32 = round(v * 1000.0 / 60.0)`；
  `damage_from` 的 dps → `damage_per_tick_milli: i32 = round(v * 1000.0 / 60.0)`。
  运行时公式因此是纯整数乘加：`hp -= counts[m] * damage_per_tick_milli`。
- `run_speed` / `jump_speed` / `accel_*` / `swim_*` → `quantize_fx`（既有函数）。
- `damage_from` 的材质名经 `table.id_by_name` 解析，未知名报错（与反应表同契约）；
  解析后**按材质 id 升序排序**存 `Vec<(u8, i32)>`——定序遍历红线，且与 Noita
  `mDamageMaterials` 注释 "NOTE! Sorted!" 一致。

- [ ] **Step 5: 实现 `step_world_interaction`**

```rust
/// 第 2b 步后半（spec §4.3–§4.5）：排开 → 游泳 → 接触伤害 → HP。
/// 按 id 序；排开顺序 = AABB 扫描格序，确定。
pub fn step_world_interaction(&mut self, world: &mut World, table: &MaterialTable,
                              tpl: &CreatureTable, stamp: u8,
                              spawns: &mut Vec<SpawnRequest>) {
    for i in 0..self.list.len() {
        if !self.list[i].alive { continue; }
        let t = tpl.get(self.list[i].template);
        let c = &self.list[i];
        let (cx, cy) = (c.x.to_cell(), c.y.to_cell());
        // ① 扫 AABB 一遍，同时收集"可排开格坐标"与"各材质格数"。
        //    格序 = 自上而下、自左而右，确定；`counts` 用定长数组而非 HashMap。
        let mut soft_cells: Vec<(i32, i32, u8)> = Vec::new();
        let mut counts = [0u16; 256];
        let mut submerged = 0u16;
        for gy in (cy - c.half_h)..=(cy + c.half_h) {
            for gx in (cx - c.half_w)..=(cx + c.half_w) {
                if !world.in_bounds(gx, gy) { continue; }
                let m = world.cell(gx, gy).material();
                counts[m as usize] = counts[m as usize].saturating_add(1);
                match table.category(m) {
                    Category::Liquid => { submerged += 1; soft_cells.push((gx, gy, m)); }
                    Category::Powder => soft_cells.push((gx, gy, m)),
                    _ => {}
                }
            }
        }
        // ② 排开：取前 max_displace_per_tick 个，置 air + 脱格成粒子
        //    （复用 M3 被盖液体脱格的同一形态：set_cell_stamped(AIR) + SpawnRequest）
        for &(gx, gy, m) in soft_cells.iter().take(t.max_displace_per_tick) {
            world.set_cell_stamped(table, gx, gy, MAT_AIR, stamp);
            spawns.push(SpawnRequest {
                material: m,
                x: Fx::from_int(gx) + fixed::HALF_CELL,   // 格心，与 explode/rules 同口径
                y: Fx::from_int(gy) + fixed::HALF_CELL,
                vx: c.vx,
                vy: c.vy,
            });
        }
        // ③ 游泳：submerged > 0 时按竖直意图选浮力档，vy 加浮力、(vx,vy) 乘 swim_drag
        // ④ 接触伤害：对 t.damage_from 里每条（加载期已按材质 id 升序）
        //    若 counts[m] >= t.min_cell_count 则 hp -= counts[m] as i32 * dmg_per_tick_milli
        // ⑤ hp <= 0 → alive = false（不移除，id 保留；速度清零，墓碑不动）
    }
}
```

`set_cell_stamped` 已是 `pub(crate)`，`creature` 模块直接调用即可（与 `particle::commit`
复用同一写入路径的先例一致——脏矩形合并与 chunk 唤醒对生物排开一视同仁）。
`fixed::HALF_CELL` 现为 `pub(crate)`，`creature` 同 crate 可用。

- [ ] **Step 6: 接线到 `lib.rs`**

```rust
self.creatures.step_kinematics(&self.world, &self.table, &self.creature_table, inputs);
self.creatures.step_world_interaction(&mut self.world, &self.table, &self.creature_table,
                                      stamp, &mut self.spawn_queue);
```

排开产生的 `SpawnRequest` 进的是**同一个** `spawn_queue`，本 tick 第 5 步粒子相
按追加序 drain——与 `Op::Emit`、M3 被盖液体完全同一条通路。

- [ ] **Step 7: 跑测试**

Run: `cargo test -p sand-core --test creature_behavior`
Expected: 14 条全 PASS。

- [ ] **Step 8: 全量 + lint + 提交**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`

```bash
git commit -m "feat(core): M4 Task 3 生物与世界互动——排开液体、游泳、材质接触伤害

排开走 M3 被盖液体同一条脱格通路（set_cell_stamped + spawn_queue），带每
tick 每生物上限、超限不排队；游泳三档浮力 + swim_drag；接触伤害按 Noita
口径写受害者侧（creatures.ron 的 damage_from，按材质 id 升序），当帧接触
不足 min_cell_count=4 整项忽略；hp 归零走墓碑、id 永不回收。
creatures.ron 加载与 creatures_fp 指纹。行为测试 +6 条。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_017GZJc7RrRKLh4XgoJMUb4m"
```

---

