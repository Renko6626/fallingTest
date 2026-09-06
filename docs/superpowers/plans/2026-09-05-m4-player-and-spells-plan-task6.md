> 文档路径：`docs/superpowers/plans/2026-09-05-m4-player-and-spells-plan-task6.md`
> 运行时版本：Rust（内核）+ Godot 4 + gdext（表现层）
> 最近更新：2026-09-05 (UTC+8)
> **Status**: Implemented
> 总纲：`2026-09-05-m4-player-and-spells-plan.md`（Goal / Architecture / **Global Constraints** / File Structure / Task 索引）

# M4 · Task 6：弹体七项扩展

> **For agentic workers:** 本文只含一个 Task。**开工前必读总纲的 Global Constraints 全节**
> ——它是本 Task 验收的隐含组成部分。
> **Spec:** `docs/superpowers/specs/2026-09-05-m4-player-and-spells-design.md`

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

