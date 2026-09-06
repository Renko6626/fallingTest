> 文档路径：`docs/superpowers/plans/2026-09-05-m4-player-and-spells-plan-task2.md`
> 运行时版本：Rust（内核）+ Godot 4 + gdext（表现层）
> 最近更新：2026-09-05 (UTC+8)
> **Status**: Implemented
> 总纲：`2026-09-05-m4-player-and-spells-plan.md`（Goal / Architecture / **Global Constraints** / File Structure / Task 索引）

# M4 · Task 2：生物本体与运动学

> **For agentic workers:** 本文只含一个 Task。**开工前必读总纲的 Global Constraints 全节**
> ——它是本 Task 验收的隐含组成部分。
> **Spec:** `docs/superpowers/specs/2026-09-05-m4-player-and-spells-design.md`

---

## Task 2: 生物本体与运动学

**Files:**
- Modify: `crates/sand-core/src/creature.rs`（填实）、`material.rs`、`body.rs`、`world.rs`、`lib.rs`
- Test: `crates/sand-core/tests/creature_behavior.rs`（新建）

**Interfaces:**
- Consumes: Task 1 的 `InputFrame`、`fixed::dir_of`、`Sim` 签名
- Produces:
  - `material::is_solid(cell: Cell, table: &MaterialTable, include_bodies: bool) -> bool`
  - `creature::{Creature, Creatures, MAX_CREATURES}`
  - `Creatures::spawn(&mut self, tpl: &CreatureTable, template: u8, x: i32, y: i32, team: u8, controller: u8, loadout: [u8; MAX_SLOTS]) -> Option<u8>`
  - `Creatures::step_kinematics(&mut self, world: &World, table: &MaterialTable, tpl: &CreatureTable, inputs: &[InputFrame])`
  - `Creatures::get(&self, id: u8) -> Option<&Creature>`、`Creatures::len(&self) -> usize`
  - `Creatures::input_of(&self, i: usize, inputs: &[InputFrame]) -> InputFrame`（`controller == 255` 或越界 → `InputFrame::default()`，即全键松开）
  - `Creatures::hash_into(&self) -> u64`
  - `Sim::creatures(&self) -> &Creatures`、`Sim::creatures_mut(&mut self) -> &mut Creatures`
    （后者与既有 `Sim::queue_spawn` 同体例：`pub`，文档标注"供测试与诊断"）
  - `Creatures::set_hp(&mut self, id: u8, hp: i32)`、`Creatures::set_mana(&mut self, id: u8, mana: i32)`（同上，供测试）
  - `Op::SpawnCreature { x: i32, y: i32, template: u8, team: u8, controller: u8, loadout: [u8; MAX_SLOTS] }`

- [ ] **Step 1: 抽出共用硬格谓词（纯搬移 + 参数化）**

`material.rs` 追加：

```rust
use crate::cell::Cell;

/// 硬格判定，刚体与生物共用（spec §2）。
///
/// `include_bodies`：刚体自己做地形缓存时传 `false`（body 不与自身碰撞）；
/// **生物传 `true`**——刚体盖章格对生物就是可站立平台，这是 M3 木箱免费
/// 变地形的来源。`body_passable` 语义两侧共享（能让刚体穿过的软材质，
/// 生物同样穿过）。
pub fn is_solid(cell: Cell, table: &MaterialTable, include_bodies: bool) -> bool {
    let m = cell.material();
    m != MAT_AIR
        && (include_bodies || !cell.is_body())
        && !matches!(table.category(m), Category::Gas | Category::Liquid)
        && !table.body_passable(m)
}
```

`body.rs` 的 `fn is_hard(cell, table)` 改为 `material::is_solid(cell, table, false)` 的薄包装
（保留名字，调用点一行不改）。

Run: `cargo test -p sand-core` — 既有 body 测试必须原样绿（纯搬移）。

- [ ] **Step 2: 写失败的行为测试**

新建 `crates/sand-core/tests/creature_behavior.rs`：

```rust
mod common;
use sand_core::{input::*, world::Op, *};

/// 造一个 4×2 chunk 的世界，底部一行 wall 当地板。
fn floor_world() -> (Sim, u8) { /* common 里的构造助手，见下方 Step 4 */ }

#[test]
fn creature_falls_and_lands_on_floor() {
    let (mut sim, id) = floor_world();
    for _ in 0..120 { sim.step(&[], &[]); }
    let c = sim.creatures().get(id).unwrap();
    assert!(c.on_ground, "该落地了");
    assert!(c.vy == Fx::ZERO, "落地后竖直速度清零");
}

#[test]
fn creature_walks_right_when_right_is_held() {
    let (mut sim, id) = floor_world();
    let x0 = sim.creatures().get(id).unwrap().x;
    for _ in 0..60 { sim.step(&[], &[InputFrame::new(BTN_RIGHT, 0, 0)]); }
    assert!(sim.creatures().get(id).unwrap().x > x0, "按右应该往右走");
}

#[test]
fn creature_is_blocked_by_a_wall_column() {
    // 右侧竖一道 wall，走 300 tick 也不能穿过去
    let (mut sim, id) = floor_world();
    sim.apply_setup(&[Op::Fill { material: MAT_WALL, x0: 40, y0: 0, x1: 40, y1: 127 }]);
    for _ in 0..300 { sim.step(&[], &[InputFrame::new(BTN_RIGHT, 0, 0)]); }
    assert!(sim.creatures().get(id).unwrap().x.to_cell() < 40, "撞墙不得穿过");
}

#[test]
fn creature_climbs_over_a_three_cell_step_but_not_four() {
    // climb_over_y = 3：3 格台阶自动跨上去，4 格挡住
    for (h, should_pass) in [(3i32, true), (4, false)] {
        let (mut sim, id) = floor_world();
        let top = 126 - h;
        sim.apply_setup(&[Op::Fill { material: MAT_WALL, x0: 40, y0: top, x1: 41, y1: 126 }]);
        for _ in 0..300 { sim.step(&[], &[InputFrame::new(BTN_RIGHT, 0, 0)]); }
        let passed = sim.creatures().get(id).unwrap().x.to_cell() > 41;
        assert_eq!(passed, should_pass, "台阶高 {h} 的跨越结果不符");
    }
}

#[test]
fn jump_only_works_on_ground() {
    let (mut sim, id) = floor_world();
    for _ in 0..120 { sim.step(&[], &[]); }              // 先落地
    sim.step(&[], &[InputFrame::new(BTN_JUMP, 0, 0)]);
    assert!(sim.creatures().get(id).unwrap().vy < Fx::ZERO, "起跳应有向上速度");
    let vy_air = sim.creatures().get(id).unwrap().vy;
    sim.step(&[], &[InputFrame::new(BTN_JUMP, 0, 0)]);   // 空中再按
    assert!(sim.creatures().get(id).unwrap().vy > vy_air, "空中按跳不得二段跳");
}

#[test]
fn creature_stands_on_a_stamped_rigid_body() {
    // M3 木箱盖章格对生物就是地形（spec §4.2）
    let (mut sim, id) = floor_world();
    sim.apply_setup(&[Op::SpawnBody { material: /* wood */ 3, x: 30, y: 100, w: 16, h: 16, angle_deg: 0 }]);
    // 生物出生在箱子正上方，落下后应停在箱顶而非穿过去
    for _ in 0..200 { sim.step(&[], &[]); }
    let c = sim.creatures().get(id).unwrap();
    assert!(c.on_ground && c.y.to_cell() < 100, "应踩在箱顶上");
}

#[test]
fn creature_id_is_stable_and_never_recycled() {
    let (mut sim, _) = floor_world();
    let n = sim.creatures().len();
    sim.step(&[Op::SpawnCreature { x: 10, y: 10, template: 0, team: 1, controller: 255,
                                   loadout: [255; MAX_SLOTS] }], &[]);
    assert_eq!(sim.creatures().len(), n + 1, "新生物追加在末尾，id = 下标");
}

#[test]
fn spawn_beyond_capacity_is_rejected_deterministically() {
    let (mut sim, _) = floor_world();
    for _ in 0..MAX_CREATURES + 5 {
        sim.step(&[Op::SpawnCreature { x: 10, y: 10, template: 0, team: 1, controller: 255,
                                       loadout: [255; MAX_SLOTS] }], &[]);
    }
    assert_eq!(sim.creatures().len(), MAX_CREATURES, "超限必须确定性拒绝");
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p sand-core --test creature_behavior`
Expected: FAIL（编译错误，`Creature` 字段与方法未定义）。

- [ ] **Step 4: 补 `tests/common` 的世界构造助手**

`crates/sand-core/tests/common/mod.rs` 追加（沿用既有 helper 风格）：

```rust
/// 4×2 chunk（256×128）的世界，底行 wall；在 (32, 100) 放一个 controller 0 的生物。
pub fn floor_world_with_creature(tbl: CreatureTable) -> (Sim, u8) {
    let cfg = InitConfig { width_chunks: 4, height_chunks: 2, seed: 42,
                           threads: 1, scan: ScanMode::LiveRect };
    let mut sim = Sim::new(&cfg, materials(), ReactionTable::empty(&materials()),
                           tbl, SpellTable::empty()).unwrap();
    sim.apply_setup(&[
        Op::Fill { material: MAT_WALL, x0: 0, y0: 127, x1: 255, y1: 127 },
        Op::SpawnCreature { x: 32, y: 100, template: 0, team: 0, controller: 0,
                            loadout: [255; MAX_SLOTS] },
    ]);
    (sim, 0)
}
```

- [ ] **Step 5: 实现 `Creature` / `Creatures` / 运动学**

`creature.rs` 关键实现要点（完整代码由实施者写，以下为承重逻辑）：

```rust
pub const MAX_CREATURES: usize = 16;

pub struct Creature {
    pub x: Fx, pub y: Fx, pub vx: Fx, pub vy: Fx,
    pub half_w: i32, pub half_h: i32,
    pub hp: i32, pub mana: i32,
    pub cooldowns: [u16; MAX_SLOTS], pub loadout: [u8; MAX_SLOTS],
    pub aim: Bam, pub team: u8, pub controller: u8, pub template: u8,
    pub on_ground: bool, pub facing_right: bool, pub alive: bool,
}

impl Creatures {
    pub fn step_kinematics(&mut self, world: &World, table: &MaterialTable,
                           tpl: &CreatureTable, inputs: &[InputFrame]) {
        for i in 0..self.list.len() {              // 按 id 序，不得用迭代器打乱
            if !self.list[i].alive { continue; }
            let inp = self.input_of(i, inputs);    // controller == 255 → 全松开
            let t = tpl.get(self.list[i].template);
            // ① 水平意图 → 加速/减速（地面 accel_ground、空中 accel_air）
            // ② 重力：vy += G（与网格同源）；起跳只在 on_ground 时生效
            // ③ 逐轴扫掠：先 x 后 y（顺序即协议）
            self.sweep_x(i, world, table, t);
            self.sweep_y(i, world, table, t);
            self.list[i].aim = inp.aim;
        }
    }
}
```

`sweep_x` 的承重细节：

```rust
/// 单 tick 单轴最大整格步数（防高速穿透 + 界定最坏成本）。
pub const CREATURE_MAX_STEP: i32 = 8;

/// AABB 内是否有硬格：逐格扫 [cx-hw, cx+hw] × [cy-hh, cy+hh]。
/// `include_bodies = true`——刚体盖章格对生物就是地形。
fn aabb_blocked(world: &World, table: &MaterialTable, x: Fx, y: Fx, hw: i32, hh: i32) -> bool {
    let (cx, cy) = (x.to_cell(), y.to_cell());
    for gy in (cy - hh)..=(cy + hh) {
        for gx in (cx - hw)..=(cx + hw) {
            if material::is_solid(world.cell(gx, gy), table, true) {
                return true;
            }
        }
    }
    false
}

/// 沿 x 轴按整格推进，撞硬格即停并清零该轴速度；被挡时尝试抬高
/// 1..=climb_over_y 格重试（Noita climb_over_y）。抬高判定按固定升序、无掷骰。
///
/// **小数部分**：不足一格的余量直接累加进坐标、不再做碰撞判定——半格位移不可能
/// 跨格，下一 tick 的整格步进会覆盖。这样避免为"亚格穿透"引入半格采样。
fn sweep_x(c: &mut Creature, world: &World, table: &MaterialTable, t: &CreatureTpl) {
    let (hw, hh) = (c.half_w, c.half_h);
    let dir = if c.vx.0 > 0 { 1 } else if c.vx.0 < 0 { -1 } else { 0 };
    if dir == 0 {
        return;
    }
    let steps = ((c.vx.0.abs() >> 16) as i32).min(CREATURE_MAX_STEP);
    for _ in 0..steps {
        let nx = c.x + Fx::from_int(dir);
        if !aabb_blocked(world, table, nx, c.y, hw, hh) {
            c.x = nx;
            continue;
        }
        let mut climbed = false;
        for up in 1..=t.climb_over_y {
            let ny = c.y - Fx::from_int(up);
            if !aabb_blocked(world, table, nx, ny, hw, hh) {
                c.x = nx;
                c.y = ny;
                climbed = true;
                break;
            }
        }
        if !climbed {
            c.vx = Fx::ZERO;
            return;                              // 撞停：小数余量一并丢弃
        }
    }
    // 小数余量（保号）
    let frac = c.vx.0.abs() & 0xFFFF;
    c.x = c.x + Fx(frac * dir);
    c.facing_right = dir > 0;
}
```

`sweep_y` 同形（无跨台阶分支）：撞到即 `c.vy = Fx::ZERO`；**向下**撞停时
`c.on_ground = true`，其余情况（含向上撞顶、未撞）置 `false`。
两者必须**先 x 后 y** 调用——顺序即协议（spec §4.2）。

- [ ] **Step 6: `Op::SpawnCreature` 接线**

`world.rs` 的 `Op` 加变体 + `apply_op` 的 `unreachable!("必须由 Sim 路由到 Creatures")` 分支
（与 `Op::SpawnBody` 完全同体例）；`lib.rs::Sim::apply_one` 加路由分支。

`lib.rs::step` 接第 2 步的 2a/2b：

```rust
// 2a + 2b. 实体相前半（架构 §4 第 2 步，M4 spec §1.1）：输入应用 + 生物运动学。
//     读本 tick 起始网格——刚体相(3)与网格四相(4)都在其后。
self.creatures.step_kinematics(&self.world, &self.table, &self.creature_table, inputs);
```

`Creatures::hash_into` 实现为按 id 序折叠全字段（含 `cooldowns`/`mana`/`hp`/`aim`）。

- [ ] **Step 7: 跑行为测试**

Run: `cargo test -p sand-core --test creature_behavior`
Expected: 8 条全 PASS。

- [ ] **Step 8: 全量与 lint，golden 重录**

Run:
```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
既有 golden **不需重录**（无生物的场景里 `Creatures::hash_into()` 仍为空表折叠值，
与 Task 1 一致）——若 diff 非空说明空表折叠值被改动，停下排查。

- [ ] **Step 9: 提交**

```bash
git add -A
git commit -m "feat(core): M4 Task 2 生物本体与运动学——逐轴扫掠、跨台阶、踩得住刚体

硬格谓词从 body.rs 抽到 material::is_solid（include_bodies 参数化）：刚体
盖章格对生物即地形，M3 木箱免费变平台。逐轴分离扫掠先 x 后 y（顺序即协议），
climb_over_y=3 自动跨台阶，起跳只在 on_ground 生效。Op::SpawnCreature 与
SpawnBody 同体例由 Sim 路由；生物 id 永不回收（InputFrame 按 id 索引）。
行为测试 8 条。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_017GZJc7RrRKLh4XgoJMUb4m"
```

---

