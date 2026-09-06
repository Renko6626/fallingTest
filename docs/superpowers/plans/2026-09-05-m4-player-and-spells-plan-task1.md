> 文档路径：`docs/superpowers/plans/2026-09-05-m4-player-and-spells-plan-task1.md`
> 运行时版本：Rust（内核）+ Godot 4 + gdext（表现层）
> 最近更新：2026-09-05 (UTC+8)
> **Status**: Implemented
> 总纲：`2026-09-05-m4-player-and-spells-plan.md`（Goal / Architecture / **Global Constraints** / File Structure / Task 索引）

# M4 · Task 1：管线与签名骨架

> **For agentic workers:** 本文只含一个 Task。**开工前必读总纲的 Global Constraints 全节**
> ——它是本 Task 验收的隐含组成部分。
> **Spec:** `docs/superpowers/specs/2026-09-05-m4-player-and-spells-design.md`

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

