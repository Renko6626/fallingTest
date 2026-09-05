> 文档路径：`docs/superpowers/plans/2026-09-05-m4-player-and-spells-plan.md`
> 运行时版本：Rust（内核）+ Godot 4 + gdext（表现层）
> 最近更新：2026-09-05 (UTC+8)
> **Status**: Proposed
> 续篇：`2026-09-05-m4-player-and-spells-plan-2.md`（Task 5–7）

# M4 玩家与法术 · 实施计划（Task 1–4）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让落沙世界里出现会动的生物和会飞的投射物——生物能跑跳、踩得住刚体、排开水、被火烧死；三条法术原语（直射 / 爆炸 / 喷射）能打出去并改变世界。

**Architecture:** 架构 §4 规范 tick 管线的第 2 步"实体与法术"从空占位变生效，内部分四个子步骤（输入 → 生物运动学 → 弹体 → 施法），插在 ops 与刚体相之间。弹体独立于粒子池，复用 `dda.rs`/`fixed.rs` 两个模块。全部整数/定点，零浮点、零超越函数。

**Tech Stack:** Rust（`sand-core` 纯库 / `sand-harness` CLI）、RON 数据表、xxHash 状态哈希、rayon 有界并行（本计划新增代码全部串行）。

**Spec:** `docs/superpowers/specs/2026-09-05-m4-player-and-spells-design.md`

---

## Global Constraints

摘自总纲 §2/§6 与 spec §7.1，**每个 Task 的验收隐含包含本节**：

- `sand-core` 不依赖 gdext / 网络 / 文件系统 / `std::time`。世界演化 = （状态，输入）的纯函数。
- 网格逻辑纯整数；自研运动学用 `Fx`（Q16.16）定点。**核心禁用系统数学库超越函数**——BAM 角 → 方向向量必须查表。
- 一切逻辑随机 = `rng::rng_u32(fseed, stream, x, y, salt, attempt)`。禁全局顺序消费的 RNG 流。同帧同源的多次掷骰必须靠 `salt`/`attempt` 区分（总纲 §11 翻案第 4 条）。
- 禁 std `HashMap`/`HashSet` 默认 hasher（clippy `disallowed_types` 执法）。一切影响状态的遍历必须定序。
- 数据驱动：法术/生物走 RON 表，禁 if-else 硬编码。RON 写十进制小数，**加载期一次性量化**为整数或 `Fx`（沿用 `quantize_fx` / `quantize_splash_chance` 体例）。
- 一切算术走 `wrapping_*`（`fixed.rs` 已有纪律），保证 dev/release profile 位级一致。
- 限流常量两端必须一致，超限**确定性拒绝、不排队**：`MAX_CREATURES = 16`、`MAX_PROJECTILES = 4096`、`max_displace_per_tick`（模板字段）。
- `MAX_SLOTS = 4`（loadout 槽位数）。
- 完成任何"已通过 / 已修复"断言前必须先跑命令验证（`cargo test` / `cargo clippy`），不得凭推断。
- 每个 Task 结束时提交一次，commit message 结尾附：
  ```
  Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_017GZJc7RrRKLh4XgoJMUb4m
  ```

---

## File Structure

| 文件 | 动作 | 职责 |
|---|---|---|
| `crates/sand-core/src/input.rs` | 新建 | `InputFrame` 定义与位打包编解码 |
| `crates/sand-core/src/fixed.rs` | 改 | 追加 `Bam` 类型 + 1024 项 sin 表 + `dir_of` |
| `crates/sand-core/src/sin_table.rs` | 新建（生成物） | 1024 个 Q16.16 sin 值字面量，`include!` 进 `fixed.rs` |
| `crates/sand-core/src/creature.rs` | 新建 | 生物表：模板、运动学、扫掠碰撞、排开、游泳、接触伤害、HP |
| `crates/sand-core/src/projectile.rs` | 新建 | 弹体表：积分、DDA、命中结算、侵彻、弹跳 |
| `crates/sand-core/src/spell.rs` | 新建 | 法术表 + loadout + 施法闸门 + 三原语派发 |
| `crates/sand-core/src/material.rs` | 改 | 抽出共用硬格谓词 `is_solid` |
| `crates/sand-core/src/body.rs` | 改 | `is_hard` 改为调用 `material::is_solid(.., false)`；新增单点冲量 API |
| `crates/sand-core/src/hash.rs` | 改 | 新增 `combine4` |
| `crates/sand-core/src/lib.rs` | 改 | `Sim` 持新表、`step` 签名扩展、第 2 步四子步骤接线 |
| `crates/sand-core/src/world.rs` | 改 | `Op::SpawnCreature` |
| `crates/sand-harness/src/scenario.rs` | 改 | `creatures.ron` / `spells.ron` 加载与指纹；场景 `inputs` 时间线 |
| `crates/sand-harness/src/{runner,render,main}.rs` | 改 | `step` 调用点补 `inputs` |
| `data/creatures.ron` / `data/spells.ron` | 新建 | 生物模板 / 法术表 |
| `data/scenarios/duel.ron` | 新建 | 验收场景 |
| `crates/sand-core/tests/creature_behavior.rs` | 新建 | 生物行为测试 |
| `crates/sand-core/tests/projectile_behavior.rs` | 新建 | 弹体与法术行为测试 |

---

## Task 1: 管线与签名骨架（零行为变化）

把所有会波及既有调用点的签名churn 一次做完，并把哈希结构从 `combine3` 升到 `combine4`——此时实体层恒为空表，**世界行为逐位不变**，golden 重录一次即可，后续 Task 不再动 golden 之外的既有文件。

**Files:**
- Create: `crates/sand-core/src/input.rs`、`crates/sand-core/src/sin_table.rs`
- Modify: `crates/sand-core/src/fixed.rs`、`hash.rs`、`lib.rs`
- Modify: `crates/sand-harness/src/scenario.rs`、`runner.rs`、`render.rs`
- Test: `crates/sand-core/src/input.rs`（`#[cfg(test)]`）、`crates/sand-core/src/fixed.rs`（`#[cfg(test)]`）

**Interfaces:**
- Produces:
  - `sand_core::input::{InputFrame, BTN_LEFT, BTN_RIGHT, BTN_JUMP, BTN_FIRE, BTN_DOWN, MAX_SLOTS}`
  - `InputFrame::new(buttons: u8, aim: u16, slot: u8) -> InputFrame`、`InputFrame::pack(self) -> u32`、`InputFrame::unpack(u32) -> InputFrame`、`InputFrame::held(self, mask: u8) -> bool`
  - `sand_core::fixed::{Bam, dir_of}`：`pub type Bam = u16;`、`pub fn dir_of(a: Bam) -> (Fx, Fx)`
  - `sand_core::hash::combine4(grid: u64, particles: u64, bodies: u64, entities: u64) -> u64`
  - `Sim::new(cfg: &InitConfig, table: MaterialTable, reactions: ReactionTable, creatures: CreatureTable, spells: SpellTable) -> Result<Sim, String>`
  - `Sim::step(&mut self, ops: &[Op], inputs: &[InputFrame])`
  - `CreatureTable::empty() -> CreatureTable`、`SpellTable::empty() -> SpellTable`（本 Task 只需空壳，字段留到 Task 3/5）
  - `sand_harness::scenario::Scenario::inputs_for_tick(&self, tick: u64) -> Vec<InputFrame>`

- [ ] **Step 1: 生成 sin 表并提交为源码**

用一次性脚本生成（**脚本不入库**，产物入库；核心运行时永不算 sin）：

```bash
python3 -c '
import math
vals = [round(math.sin(2*math.pi*i/1024) * 65536) for i in range(1024)]
print("//! 1024 项 Q16.16 正弦表（索引 i 对应角度 i/1024 圈）。")
print("//! **生成物，勿手改**：由一次性脚本产出并入库——核心禁用系统数学库超越函数")
print("//! （总纲 §6），故运行时只查表。表内容由 fixed.rs 的 sin_table_golden_checksum 钉死。")
print("pub(crate) const SIN_TABLE: [i32; 1024] = [")
for i in range(0, 1024, 8):
    print("    " + ", ".join(str(v) for v in vals[i:i+8]) + ",")
print("];")
' > crates/sand-core/src/sin_table.rs
```

- [ ] **Step 2: 写 `fixed.rs` 的失败测试**

追加到 `crates/sand-core/src/fixed.rs` 的 `mod tests`：

```rust
#[test]
fn sin_table_golden_checksum() {
    // 金值：表一旦被改动即失败（生成物不得手改）
    let mut h = xxhash_rust::xxh3::Xxh3::new();
    for v in crate::sin_table::SIN_TABLE {
        h.update(&v.to_le_bytes());
    }
    assert_eq!(h.digest(), 0x0000_0000_0000_0000, "sin 表被改动——重跑生成脚本或恢复");
}

#[test]
fn dir_of_cardinals_are_exact() {
    assert_eq!(dir_of(0), (Fx::from_int(1), Fx::ZERO), "0° = +x");
    assert_eq!(dir_of(16384), (Fx::ZERO, Fx::from_int(1)), "90° = +y（屏幕坐标向下）");
    assert_eq!(dir_of(32768), (Fx::from_int(-1), Fx::ZERO), "180° = -x");
    assert_eq!(dir_of(49152), (Fx::ZERO, Fx::from_int(-1)), "270° = -y");
}

#[test]
fn dir_of_is_unit_length_within_tolerance() {
    // 查表 + 定点截断的误差上界：逐项检查 |v|² 落在 1.0 ± 1/256 内
    for a in (0u32..65536).step_by(37) {
        let (cx, cy) = dir_of(a as u16);
        let n = (cx.mul(cx) + cy.mul(cy)).0 as i64;
        let one = 1i64 << 16;
        assert!((n - one).abs() < one / 256, "角 {a} 的模平方 {n} 偏离 1.0 过多");
    }
}
```

金值 `0x0000_0000_0000_0000` 是占位——Step 4 跑出真值后回填。

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p sand-core fixed:: -- --nocapture`
Expected: FAIL，`sin_table` 模块与 `dir_of` 未定义（编译错误）。

- [ ] **Step 4: 实现 `Bam` 与 `dir_of`**

`crates/sand-core/src/fixed.rs` 追加：

```rust
/// BAM 角（binary angle measurement）：无符号 16 位，65536 = 360°，
/// 逆时针为正、0 = +x。选它而非度数/弧度是因为**加减法天然环绕**，
/// 无需取模，且与架构 §3 `bridge-input` 条目定的编码一致。
pub type Bam = u16;

#[path = "sin_table.rs"]
mod sin_table_mod;
pub(crate) use sin_table_mod::SIN_TABLE;

/// BAM 角 → 单位方向向量 `(cos, sin)`，查 1024 项表（角分辨率 0.35°）。
/// 核心禁用系统超越函数（总纲 §6），故这里是唯一的三角来源。
pub fn dir_of(a: Bam) -> (Fx, Fx) {
    let i = (a >> 6) as usize;                       // 65536 / 1024 = 64
    let sin = Fx(SIN_TABLE[i]);
    let cos = Fx(SIN_TABLE[(i + 256) & 1023]);       // cos θ = sin(θ + 90°)
    (cos, sin)
}
```

`lib.rs` 加 `mod sin_table;`（若用 `#[path]` 则无需，二选一，保持一处）。

- [ ] **Step 5: 跑测试，回填金值**

Run: `cargo test -p sand-core fixed::tests::sin_table_golden_checksum -- --nocapture`
把断言失败信息里的 `left` 值填回 Step 2 的金值，重跑：
Run: `cargo test -p sand-core fixed::`
Expected: 3 条全 PASS。

- [ ] **Step 6: 写 `input.rs` 的失败测试**

新建 `crates/sand-core/src/input.rs`，先只写测试与空壳：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_roundtrip_is_identity() {
        for &(b, a, s) in &[(0u8, 0u16, 0u8), (0b1_0101, 12345, 3), (0xFF, 65535, 3)] {
            let f = InputFrame::new(b, a, s);
            assert_eq!(InputFrame::unpack(f.pack()), f, "buttons={b} aim={a} slot={s}");
        }
    }

    #[test]
    fn slot_is_clamped_into_range_at_construction() {
        // 越界槽位不得进状态：加载期/桥侧可能传脏值，构造点收口
        assert_eq!(InputFrame::new(0, 0, 200).slot, (MAX_SLOTS - 1) as u8);
    }

    #[test]
    fn held_reads_the_right_bit() {
        let f = InputFrame::new(BTN_LEFT | BTN_FIRE, 0, 0);
        assert!(f.held(BTN_LEFT) && f.held(BTN_FIRE));
        assert!(!f.held(BTN_RIGHT) && !f.held(BTN_JUMP));
    }
}
```

- [ ] **Step 7: 跑测试确认失败**

Run: `cargo test -p sand-core input::`
Expected: FAIL（`InputFrame` 未定义）。

- [ ] **Step 8: 实现 `InputFrame`**

```rust
//! 玩家意图的**唯一**入核通道（架构 §1 铁律 1、§3 `bridge-input`）。
//! 生物控制器只吃本结构——这让 P2"Godot → 核心唯一写入路径 = InputFrame"
//! 在 M4 就获得类型级担保，不必等 bridge 落地。

use crate::fixed::Bam;

pub const BTN_LEFT: u8 = 1 << 0;
pub const BTN_RIGHT: u8 = 1 << 1;
pub const BTN_JUMP: u8 = 1 << 2;
pub const BTN_FIRE: u8 = 1 << 3;
pub const BTN_DOWN: u8 = 1 << 4;

/// loadout 槽位数（spec §3.2/§6.1）。
pub const MAX_SLOTS: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct InputFrame {
    pub buttons: u8,
    pub aim: Bam,
    pub slot: u8,
}

impl InputFrame {
    /// `slot` 在构造点 clamp 进 `0..MAX_SLOTS`——脏值不得进状态。
    pub fn new(buttons: u8, aim: Bam, slot: u8) -> InputFrame {
        InputFrame { buttons, aim, slot: slot.min((MAX_SLOTS - 1) as u8) }
    }

    pub fn held(self, mask: u8) -> bool {
        self.buttons & mask != 0
    }

    /// 网络/回放编码：4 字节小端打包（架构 §3 定的"约 8 字节"上限内）。
    pub fn pack(self) -> u32 {
        (self.buttons as u32) | ((self.aim as u32) << 8) | ((self.slot as u32) << 24)
    }

    pub fn unpack(v: u32) -> InputFrame {
        InputFrame::new((v & 0xFF) as u8, ((v >> 8) & 0xFFFF) as u16, ((v >> 24) & 0xFF) as u8)
    }
}
```

- [ ] **Step 9: 跑测试**

Run: `cargo test -p sand-core input::`
Expected: 3 条 PASS。

- [ ] **Step 10: 加 `combine4` 及其测试**

`hash.rs` 追加（沿用 `combine3` 的体例与注释风格）：

```rust
/// 总哈希折叠（M4 起四层）：网格 + 粒子 + 刚体 + 实体（生物 + 弹体）。
/// **结构变更**（M3 的三层 `combine3` 退役）⇒ 既有 golden 全部重录一次，
/// 取证程序同 M3：先用 `--grid-only` 证明网格哈希流逐位不变。
pub fn combine4(grid_root: u64, particle_hash: u64, body_hash: u64, entity_hash: u64) -> u64 {
    let mut h = Xxh3::new();
    h.update(&grid_root.to_le_bytes());
    h.update(&particle_hash.to_le_bytes());
    h.update(&body_hash.to_le_bytes());
    h.update(&entity_hash.to_le_bytes());
    h.digest()
}
```

测试（追加进 `hash.rs` 的 `mod tests`）：

```rust
#[test]
fn combine4_is_sensitive_to_every_input() {
    let base = combine4(1, 2, 3, 4);
    assert_eq!(base, combine4(1, 2, 3, 4), "同输入必须同值");
    assert_ne!(base, combine4(9, 2, 3, 4), "网格根差异必须可见");
    assert_ne!(base, combine4(1, 9, 3, 4), "粒子层差异必须可见");
    assert_ne!(base, combine4(1, 2, 9, 4), "刚体层差异必须可见");
    assert_ne!(base, combine4(1, 2, 3, 9), "实体层差异必须可见");
}
```

Run: `cargo test -p sand-core hash::`
Expected: PASS。

- [ ] **Step 11: `Sim` 签名扩展 + 空表**

`creature.rs` / `spell.rs` 先各建一个空壳（字段留到后续 Task）：

```rust
// creature.rs
/// 生物模板表（Task 3 填字段）。与 `MaterialTable` 同体例：加载期构造、只读。
#[derive(Clone, Debug, Default)]
pub struct CreatureTable { /* Task 3 */ }
impl CreatureTable { pub fn empty() -> CreatureTable { CreatureTable::default() } }

/// 生物表（Task 2 填字段）。
#[derive(Clone, Debug, Default)]
pub struct Creatures { /* Task 2 */ }
impl Creatures {
    pub fn new() -> Creatures { Creatures::default() }
    /// 实体层哈希的生物部分。空表时恒返回 0——Task 1 的"零行为变化"依赖此。
    pub fn hash_into(&self) -> u64 { 0 }
}
```

`spell.rs` 同形：`SpellTable::empty()`、`Projectiles`（放 `projectile.rs`，Task 4 填）。
本 Task 里 `projectile.rs` 也建空壳并给 `hash_into() -> u64 { 0 }`。

`lib.rs` 改动：

```rust
pub mod creature;
pub mod input;
pub mod projectile;
pub mod spell;

pub use input::{InputFrame, MAX_SLOTS};

pub struct Sim {
    // ... 既有字段不变 ...
    creature_table: creature::CreatureTable,
    spell_table: spell::SpellTable,
    creatures: creature::Creatures,
    projectiles: projectile::Projectiles,
}

impl Sim {
    pub fn new(
        cfg: &InitConfig,
        table: MaterialTable,
        reactions: ReactionTable,
        creature_table: creature::CreatureTable,
        spell_table: spell::SpellTable,
    ) -> Result<Sim, String> { /* ... */ }

    pub fn step(&mut self, ops: &[Op], inputs: &[InputFrame]) {
        // 第 2 步的四个子步骤在 Task 2/4/5 逐一接线；本 Task 只留位置与注释
        // ...既有 1 / 3 / 4 / 5 / 7 / 7' 全部不动...
    }

    /// 总哈希 = combine4（网格 / 粒子 / 刚体 / 实体）。
    pub fn state_hash(&self) -> u64 {
        hash::combine4(
            self.grid_hash(),
            self.particles.hash_into(),
            self.bodies.hash_into(&self.physics),
            self.entity_hash(),
        )
    }

    /// 实体层哈希 = 生物 + 弹体（两者都为空时恒 0）。
    fn entity_hash(&self) -> u64 {
        let mut h = xxhash_rust::xxh3::Xxh3::new();
        h.update(&self.creatures.hash_into().to_le_bytes());
        h.update(&self.projectiles.hash_into().to_le_bytes());
        h.digest()
    }
}
```

- [ ] **Step 12: harness 调用点补 `inputs`**

`scenario.rs`：`ScenarioFile` 加 `#[serde(default)] pub inputs: Vec<InputEntry>`；

```rust
/// 输入时间线（spec §7.3）：稀疏声明，**缺省沿用上一条**——避免逐帧铺满。
#[derive(Deserialize, Clone)]
pub struct InputEntry {
    pub tick: u64,
    /// 按 controller 序号排列；长度不足者补 InputFrame::default()
    pub frames: Vec<InputSpec>,
}

#[derive(Deserialize, Clone)]
pub struct InputSpec {
    #[serde(default)] pub left: bool,
    #[serde(default)] pub right: bool,
    #[serde(default)] pub jump: bool,
    #[serde(default)] pub fire: bool,
    #[serde(default)] pub down: bool,
    /// 瞄准角，度（0 = +x，逆时针）。加载期一次性量化为 BAM。
    #[serde(default)] pub aim_deg: f64,
    #[serde(default)] pub slot: u8,
}
```

`Scenario` 加 `pub inputs: Vec<(u64, Vec<InputFrame>)>`（按 tick 升序，加载期校验严格递增）与：

```rust
impl Scenario {
    /// 本 tick 生效的输入（稀疏时间线：取 tick 不大于当前值的最后一条）。
    /// 无任何条目时返回空切片——生物按"全键松开"处理。
    pub fn inputs_for_tick(&self, tick: u64) -> &[InputFrame] {
        match self.inputs.partition_point(|(t, _)| *t <= tick) {
            0 => &[],
            i => &self.inputs[i - 1].1,
        }
    }
}
```

量化：`aim_deg` → BAM 用 `quantize_bam(deg: f64) -> Result<Bam, String>`，公式
`round(deg / 360.0 * 65536.0) as i64 & 0xFFFF`，越界（|deg| > 1e6）报错。
与既有 `quantize_fx` 同体例，同样只在加载期发生一次。

`runner.rs:101,138` 与 `render.rs:80` 的 `sim.step(&ops)` 改为 `sim.step(&ops, sc.inputs_for_tick(t))`；
`runner.rs:48` 的 `Sim::new` 补两张空表。同时改三个 runner 入口的签名，**一次改到位**，
Task 5 接真表时只换实参：

```rust
// runner.rs：三个入口统一多收两张表（Task 5 之前 harness 传 empty()）
pub struct Tables<'a> {
    pub materials: &'a MaterialTable,
    pub reactions: &'a ReactionTable,
    pub creatures: &'a CreatureTable,
    pub spells: &'a SpellTable,
}
pub fn build_sim(sc: &Scenario, t: &Tables, threads: usize, scan: ScanMode) -> Result<Sim, String>;
pub fn run(sc: &Scenario, t: &Tables, fp: Fingerprints, ..) -> Result<Report, String>;
pub fn synctest(sc: &Scenario, t: &Tables, threads: usize, ticks: u64) -> Result<(), String>;

// Fingerprints 同步扩容（P5：数据表指纹全部入握手）
pub struct Fingerprints { pub materials: u64, pub reactions: u64,
                          pub creatures: u64, pub spells: u64 }
```

`main.rs` 的 `run()` 里构造 `Tables`；Task 5 之前 `creatures`/`spells` 用 `empty()` +
指纹 0，Task 5 起改为真加载。**指纹 0 不得进 golden 输出行**——本 Task 先不打印这两个
指纹，Task 5 一并加入输出（否则 golden 要重录两次）。

- [ ] **Step 13: 场景 `inputs` 往返测试**

`scenario.rs` 的 `mod tests` 追加：

```rust
#[test]
fn inputs_timeline_is_sparse_and_holds_last_value() {
    let sc = Scenario {
        name: "t".into(), world: (1, 1), seed: 0, ticks: 10,
        setup: vec![], script: vec![], fingerprint: 0,
        inputs: vec![
            (0, vec![InputFrame::new(BTN_RIGHT, 0, 0)]),
            (5, vec![InputFrame::new(BTN_JUMP, 0, 0)]),
        ],
    };
    assert_eq!(sc.inputs_for_tick(0)[0].buttons, BTN_RIGHT);
    assert_eq!(sc.inputs_for_tick(4)[0].buttons, BTN_RIGHT, "缺省沿用上一条");
    assert_eq!(sc.inputs_for_tick(5)[0].buttons, BTN_JUMP);
    assert_eq!(sc.inputs_for_tick(999)[0].buttons, BTN_JUMP);
}

#[test]
fn quantize_bam_maps_cardinals_exactly() {
    assert_eq!(quantize_bam(0.0).unwrap(), 0);
    assert_eq!(quantize_bam(90.0).unwrap(), 16384);
    assert_eq!(quantize_bam(-90.0).unwrap(), 49152, "负角必须环绕，不报错");
}
```

Run: `cargo test -p sand-harness scenario::`
Expected: PASS。

- [ ] **Step 14: 取证——网格哈希流逐位不变**

golden 现存于 `crates/sand-harness/tests/golden/*.golden`，由 `hashrun --write-golden` 产出
（**没有 `record` 子命令**）。取证：

```bash
S="sand_pile waterfall_ci mixed explosion_ci fire_oil_chain crate_yard"
mkdir -p .superpowers/m4-task1-gridonly
# 改动前基线（先 git stash 本 Task 的改动）
for s in $S; do cargo run -q -p sand-harness --release -- hashrun --grid-only \
  data/scenarios/$s.ron > .superpowers/m4-task1-gridonly/$s.before; done
git stash pop
for s in $S; do cargo run -q -p sand-harness --release -- hashrun --grid-only \
  data/scenarios/$s.ron > .superpowers/m4-task1-gridonly/$s.after; done
for s in $S; do diff .superpowers/m4-task1-gridonly/$s.{before,after} || echo "DIFF $s"; done
```
Expected: 六个 diff **全空**。非空说明本 Task 破坏了网格语义，停下排查——
本 Task 的既定性质是"零行为变化"。

- [ ] **Step 15: 重录 golden 并跑全量测试**

```bash
for s in sand_pile waterfall_ci mixed explosion_ci fire_oil_chain crate_yard; do
  cargo run -q -p sand-harness --release -- hashrun data/scenarios/$s.ron \
    --write-golden crates/sand-harness/tests/golden/$s.golden
done
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
Expected: 全绿。**重录的唯一合法理由是 `combine4` 的结构变更**——若某场景的
`--grid-only` 流在 Step 14 里有 diff，不许靠重录掩盖。

- [ ] **Step 16: 提交**

```bash
git add -A
git commit -m "feat(core): M4 Task 1 管线与签名骨架——InputFrame、BAM 查表、combine4

Sim::new/step 签名一次改到位（创建期收两张空表、步进期收 inputs），
哈希由 combine3 升 combine4（实体层此刻恒 0）；harness 场景新增稀疏
inputs 时间线与 quantize_bam。sin 表为生成物、金值 checksum 钉死。
--grid-only 取证 6 场景网格哈希流逐位不变，golden 重录。

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_017GZJc7RrRKLh4XgoJMUb4m"
```

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
