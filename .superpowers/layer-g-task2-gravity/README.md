# Layer G Task 2 —— 零加速旁路取证（spec §0 验收第 2 项）

判据：`G_ACCEL = 0` 时 `hashrun --grid-only` 的**逐 tick** 哈希序列与 Task 2
之前逐位相同。`n = max(1, v/VEL_ONE + frac_roll)` 的 `max(1, ..)` 保证 v = 0
时子步数恒 1，即退化为 Task 2 之前那条路径（spec §4.2①）。

## 怎么重跑

```bash
# before：HEAD（Task 2 之前）建 worktree，HASH_EVERY 改 1
git worktree add /tmp/base <task2 之前的 commit>
sed -i 's/HASH_EVERY: u64 = 256/HASH_EVERY: u64 = 1/' /tmp/base/crates/sand-harness/src/runner.rs
cd /tmp/base && cargo build --release -p sand-harness
for s in sand_pile mixed waterfall_ci explosion_ci; do
  ./target/release/sand-harness hashrun data/scenarios/$s.ron --grid-only > before-$s.txt
done

# after：本树 + zero-gravity feature（G_ACCEL 压成 0），用 --hash-every 1
cargo build --release -p sand-harness --features sand-core/zero-gravity
for s in sand_pile mixed waterfall_ci explosion_ci; do
  ./target/release/sand-harness hashrun data/scenarios/$s.ron --grid-only --hash-every 1 > after-$s.txt
done
diff before-$s.txt after-$s.txt   # 必须为空
```

`--hash-every` 是本 Task 新加的取证开关（默认仍是 256 = golden 格式）：
`docs/perf/baselines/*.grid-only.txt` 只有每 256 tick 的采样点，不足以支撑
"逐 tick 逐位相同"这句断言，故这里用逐 tick 序列重做。

## 实测（2026-08-31，Linux rustc 1.89）

| 场景 | 逐 tick 哈希条数 | 结果 |
|---|---|---|
| sand_pile | 600 | 逐位相同 ✓ |
| mixed | 1500 | 逐位相同 ✓ |
| waterfall_ci | 1200 | 逐位相同 ✓ |
| explosion_ci | 1200 | 逐位相同 ✓ |

合计 4500 条逐 tick 哈希 + 4 个 final + 8 条指纹，`diff` 全空。

**warning 闸门**：`sand-harness` 在 `sand_core::G_ACCEL == 0` 时向 stderr
打警告（`main.rs::run` 开头）——该 feature 改变物理，两端不一致即分叉，而
握手指纹只覆盖数据、覆盖不到代码。手改常量同样会被这条逮到。
