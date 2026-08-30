# Layer G Task 1（液体色散 ≤8）性能对照

> 文档路径：`docs/perf/2026-08-31-layer-g-task1-dispersion.md`
> 运行时版本：Rust（sand-core + sand-harness）
> 最近更新：2026-08-31 (UTC+8)
> **Status**: Implemented

对照 spec `docs/superpowers/specs/2026-08-31-layer-g-velocity-design.md` §7.1 的 bench 验收项。

## 结论先说

**无回退，且大场景显著变快。** 640×384 的 `acceptance` 场景在 LiveRect 下提速 27%–55%，ChunkSleep 下 −25%..+3.5%。小场景 `mixed`（256×192）在 8/16 线程下有 +2.6%..+16.8% 的劣化，`sparse` 基本持平。

机制清楚，不是噪声解释能打发的：色散让水**更快摊平进入静止**，chunk 更早入睡，稳态段活跃 cell 数下降；代价是浇注段每个活跃水 cell 的 `side()` 探测从 1 格变成至多 5 格。两股力量的相对大小取决于场景里"沉降段 vs 浇注段"的比例——`acceptance`（640×384 双水源，5000 tick 窗口里沉降占比高）净收益大，`mixed`（1500 tick，0–900 全在浇注）净收益接近零甚至转负。

**这不是加速优化的成果，是语义变更的副产品**，下一个 Task（重力速度积分）预期是净增成本，别把本文的数字当成可继承的余量。

## 环境与口径

口径完全照抄 `docs/perf/2026-08-30-m0-rust-baseline.md`（短程稳态吞吐），以便与既有基线同尺度阅读：

- CPU：Intel Xeon Gold 6330 @ 2.00GHz，112 逻辑核（2×28C56T，NUMA 双节点），**共享服务器**，数字 provisional。
- 构建：`cargo build --release -p sand-harness`。
- 路径：`sand-harness hashrun`，取 stderr 尾行 `tick 耗时 avg`。
- 每个 {场景 × 线程 × scan} 组合 **3 次独立进程冷启动取中位**。
- 场景与 tick：`mixed` 1500、`sparse` 2000、`acceptance` 5000（均沿用 M0 基线的 tick 数）。

**与 M0/M1 基线文档的关键差别：本次是同机同轮次的 before/after 对照，不是跨天数字比对。** before 侧用 `git worktree` 在 `HEAD`（`f4afecd`）另建工作树独立编译，并显式 `--materials` 指向该工作树的 `data/materials.ron`（无 `dispersion` 字段）；after 侧是本次改动的工作树 + `water dispersion: 5`。两侧交替执行、共享同一负载环境，消除了跨天机器负载差异——这是本文数字比"对照 M0 基线表"更可信的原因。

脚本与原始输出：本次会话 scratchpad（`bench.sh`，108 次测量）。

## 数据（ms/tick，3 次中位）

| 场景 | 线程 | scan | before | after | Δ |
|---|---|---|---|---|---|
| mixed | 1 | sleep | 0.630 | 0.504 | **−20.0%** |
| mixed | 1 | live | 0.283 | 0.269 | −4.9% |
| mixed | 8 | sleep | 0.466 | 0.478 | +2.6% |
| mixed | 8 | live | 0.278 | 0.289 | +4.0% |
| mixed | 16 | sleep | 0.554 | 0.641 | +15.7% |
| mixed | 16 | live | 0.304 | 0.355 | +16.8% |
| sparse | 1 | sleep | 1.046 | 1.044 | −0.2% |
| sparse | 1 | live | 0.258 | 0.232 | −10.1% |
| sparse | 8 | sleep | 0.402 | 0.399 | −0.7% |
| sparse | 8 | live | 0.170 | 0.177 | +4.1% |
| sparse | 16 | sleep | 0.406 | 0.449 | +10.6% |
| sparse | 16 | live | 0.248 | 0.152 | −38.7% |
| acceptance | 1 | sleep | 1.996 | 2.066 | +3.5% |
| acceptance | 1 | live | 1.164 | 0.845 | **−27.4%** |
| acceptance | 8 | sleep | 0.968 | 0.684 | **−29.3%** |
| acceptance | 8 | live | 1.045 | 0.465 | **−55.5%** |
| acceptance | 16 | sleep | 1.332 | 0.997 | **−25.2%** |
| acceptance | 16 | live | 1.083 | 0.559 | **−48.4%** |

## 怎么读这张表

- **16 线程列信噪比最差，不要单独拿它下结论。** M0 基线文档已记录本机 16 线程常态倒挂（活跃工作量太小、112 核 NUMA 共享机跨节点调度），同组 3 次 avg 波动可达 20–30%。本表里 `sparse 16 live −38.7%` 与 `mixed 16 live +16.8%` 方向相反、幅度都大，正是这种噪声的典型形状。
- **`acceptance` 是最有代表性的一行**：640×384、双水源+沙源，最接近总纲 §7 的 1280×720 目标量级，且 LiveRect 是产品实际使用的 scan 模式。它在 1/8/16 三个线程数上一致地大幅变快，方向一致 ⇒ 不是噪声。
- **`mixed` 的小幅劣化是真实的、可解释的**，不该粉饰成噪声：1500 tick 里 0–900 全是浇注，活跃水 cell 多、沉降窗口短，`side()` 的 5 格探测涨的成本收不回来。量级（≤ +17%，绝对值 ≤ 0.05ms/tick）远在 tick 预算内。

## 与总纲 §7 预算的关系

总纲 §7：1280×720 上限配置，tick 总预算 ≤ 4ms。本次全部场景均小于该上限，且是短程稳态口径而非全活跃最坏口径，故不直接对标 §7 数字，未推翻或修订既有校准。`acceptance` LiveRect 8 线程从 1.045ms 降到 0.465ms，在 §7 预算里的占用反而下降。

## 顺带记录：摊平速度（本 Task 的实际目标）

性能是副产品，Task 1 真正要修的是 M0 记录的"水面锯齿 / 摊平极慢"。客观测量（`crates/sand-core/tests/rules_behavior.rs::higher_dispersion_levels_water_faster`，192×128 盆地、16×34 水柱靠左释放，判据 = 最高水面行降到 y ≥ 118）：

| dispersion | 摊平所需 tick |
|---|---|
| 1（改动前语义） | 254 |
| 5（现值） | **96** |

**2.6× 提速。** 该测试写成两个配置的相对比较而非魔法 tick 数，故不随机器与场景几何漂移；改动前它必然失败（`side()` 不读 `dispersion` ⇒ 两次运行逐位相同，实测两边都是 254 tick）。

视觉结论仍留用户目检：`out/waterfall_disp{1,5}.gif`、`out/mixed_disp{1,5}.gif`（改动前/后同参数对照）。
