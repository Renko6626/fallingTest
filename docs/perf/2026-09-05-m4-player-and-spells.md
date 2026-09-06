# M4（玩家与法术）性能对照

> 文档路径：`docs/perf/2026-09-05-m4-player-and-spells.md`
> 运行时版本：Rust（sand-core + sand-harness）
> 最近更新：2026-09-06 (UTC+8)
> **Status**: Implemented

对照 M4 spec §0.2 验收第 5 项（bench 落档、无预算外回退）。before 侧 =
`4fde2ab`（Task 6 完成点，Task 7 动工前的收口点）。

## 结论先说

1. **Task 7（本次）零 `sand-core` 代码改动**——只新增 `data/scenarios/duel.ron`、
   两条测试（`oil_spray_then_bolt_ignites_a_chain`、
   `spread_angle_is_uniform_within_the_declared_cone`）、`data/spells.ron`
   追加一条只服务测试的 `scatter_bolt`、SyncTest/golden 场景清单登记。**既有
   6 个场景的既定行为逐位不变**——golden 重录前的两侧 `grep -v '_fp'` diff
   证实：`sand_pile`/`waterfall_ci`/`mixed`/`explosion_ci`/`fire_oil_chain`/
   `crate_yard` 六个场景的哈希流一字不差，唯一变化是 `spells_fp`（`data/
   spells.ron` 内容哈希，因追加 `scatter_bolt` 而移动，预期内）。**这个前提
   本身就是"无回退"的最硬证据**：既然模拟状态逐 tick 完全相同，同机同码同
   序列的执行耗时在测量噪声之外没有理由系统性变化——下表数字是确认，不是
   排查。
2. **`duel`（M4 主验收场景，256×128、五项行为压缩进前 ~2200 tick、总长
   3000 tick）：avg ≈ 0.11 ms/tick，max ≈ 0.6–1.6 ms（法术命中/爆炸/燃烧
   点火的瞬时尖峰）**——量级与既有场景同档，远在 16.6ms 帧预算内。
3. **SyncTest 六配置（1/4 线程 × Full/ChunkSleep/LiveRect）零分叉**、
   **`hashrun --threads 1/8/16` 三份哈希流逐字节相同**——`duel` 是第一个
   跑通 M4 全链路（`inputs` 时间线 + `SpawnCreature` + 弹体 SoA + `cast_all`
   双闸门 + `STREAM_SPREAD` 掷骰）确定性执法的场景，无回退。

## 环境与口径

照抄 `docs/perf/2026-09-02-m3-rigid-body.md`（同机同尺度）：

- CPU：Intel Xeon Gold 6330 @ 2.00GHz，共享服务器。
- `cargo build --release`；数字取自 `sand-harness hashrun` stderr 收尾的
  `tick 耗时 avg / max`（本仓没有独立 `bench` 子命令，历次 M0–M3 perf 文档
  同一口径）。每场景 3 次独立冷启动取中位。
- 场景 tick 用各自 `.ron` 文件里的默认值：`sand_pile` 600、`mixed` 1500、
  `crate_yard` 20000、`duel` 3000。
- before 侧不做独立 `git worktree` 编译——本次改动不触碰 `sand-core`/
  `sand-harness` 任何源文件，"before" 与"after"是同一份二进制在同一份
  （golden 哈希证实逐位相同的）模拟状态上运行，重新拉一份 worktree 编译
  只会得到统计噪声内的相同数字，不产出额外信息（"结论先说"第 1 条已给出
  比重跑更硬的证据：状态哈希相同 ⇒ 每 tick 的计算路径相同）。

## 数据（ms/tick，3 次中位）

| 场景 | avg | max | 备注 |
|---|---|---|---|
| sand_pile | 0.075 | 0.340 | 无生物/弹体，第 2 步是两个空循环 |
| mixed | 0.441 | 1.469 | 无生物/弹体 |
| crate_yard | 0.703 | 14.171 | 无生物/弹体；max 尖峰来自刚体切割/沙托事件（M3 既有现象，`2026-09-02` 文档已记录同量级 max） |
| duel | 0.115 | 0.641 | M4 新场景，两生物 + 弹体 + 法术全链路 |

`sand_pile`/`mixed`/`crate_yard` 三个既有场景与 `2026-09-02-m3-rigid-body.md`
"追记 4"记录的量级一致（`crate_yard` avg 0.68–1.00 区间、max 常见于 1 ms 级，
本次 max 14.171ms 出现在单个 tick——刚体切割那一下的瞬时开销，非本次改动
引入：Task 7 未改任何刚体/CA 代码，`crate_yard.golden` 的哈希流逐位不变已
证实这一点）；三次独立测量之间的抖动（±5%~15%）与历次 perf 文档记录的
"本机噪声形状"同量级，不构成回退信号。

## 确定性执法记录

```
cargo test -p sand-harness --test synctest duel_six_configs_zero_divergence --release
# test duel_six_configs_zero_divergence ... ok（17.07s，256×128 × 3000 tick × 六配置）

cargo run -q -p sand-harness --release -- hashrun data/scenarios/duel.ron --threads 1  > t1
cargo run -q -p sand-harness --release -- hashrun data/scenarios/duel.ron --threads 8  > t8
cargo run -q -p sand-harness --release -- hashrun data/scenarios/duel.ron --threads 16 > t16
diff t1 t8   # 无输出
diff t1 t16  # 无输出
```

线程数 1/8/16 三份哈希流（含逐 256-tick 采样行与 `final`）逐字节相同。

## Golden 重录证据（golden gate）

重录前对 6 个既有场景做两侧 `grep -v '_fp'` diff（新哈希 vs 旧 golden）：

```
=== sand_pile / waterfall_ci / mixed / explosion_ci / fire_oil_chain / crate_yard ===
HASH LINES IDENTICAL   # 逐场景确认，六个全部
--- fp 行 diff（唯一变化）---
< spells_fp 3ec236070beafc79
> spells_fp 7a9bd8e2a092a150
```

六个场景仅 `spells_fp` 一行变化（`data/spells.ron` 追加 `scatter_bolt` 的
预期后果），`materials_fp`/`reactions_fp`/`creatures_fp`/`scenario_fp` 与全部
`tick .. hash ..`/`final` 行逐位不变——满足重录前置证据要求，随后 7 个
golden（6 个既有 + `duel` 新增）一次性重录。
