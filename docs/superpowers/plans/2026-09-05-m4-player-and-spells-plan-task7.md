> 文档路径：`docs/superpowers/plans/2026-09-05-m4-player-and-spells-plan-task7.md`
> 运行时版本：Rust（内核）+ Godot 4 + gdext（表现层）
> 最近更新：2026-09-05 (UTC+8)
> **Status**: Proposed
> 总纲：`2026-09-05-m4-player-and-spells-plan.md`（Goal / Architecture / **Global Constraints** / File Structure / Task 索引）

# M4 · Task 7：收口

> **For agentic workers:** 本文只含一个 Task。**开工前必读总纲的 Global Constraints 全节**
> ——它是本 Task 验收的隐含组成部分。
> **Spec:** `docs/superpowers/specs/2026-09-05-m4-player-and-spells-design.md`

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
