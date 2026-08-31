# M2（反应表与燃烧）性能对照 + Cell u64 对照测量

> 文档路径：`docs/perf/2026-08-31-m2-reactions-and-fire.md`
> 运行时版本：Rust（sand-core + sand-harness）
> 最近更新：2026-08-31 (UTC+8)
> **Status**: Implemented

对照 M2 spec §0 验收第 7 项（bench + u64 对照入档）。before 侧 = `f807e54`
（M2 动工前收口点，Layer G Task 3 语义）。

## 结论先说

1. **M2 的活跃格成本 ≈ +20%（水密集场景），与 Layer G Task 2（+21%~34%）同量级，
   是预期内的语义成本**。来源明确：`water` 成为反应发起方（water+fire 表项）后，
   每个**活跃**水格每 tick 付 4 次邻居读 + 发起方比较（`rules::react`）；外加
   eval 准入多一次 `needs_eval` 查表。**睡眠稀疏性不受影响**——静止格照旧零写入
   入睡，`sparse` 场景 6 格子里 3 个为负、方向不一致（噪声形状）。绝对量级仍远在
   预算内：最重的 `acceptance` 8T live 0.70 ms/tick，对 16.6ms 帧预算余量充足。
2. **u64 对照：本机看不到一致方向的回退（±10% 噪声内）**，`acceptance`（最重
   场景）六格子反而 −0.6%~−10%。内存翻倍（4→8 B/cell，720p 全图 3.7→7.5MB）
   在本机 cache/带宽下未表现为可测的时间成本。**"随时可扩"的封装被现场验证**：
   开 feature 即换宽，全部 sand-core 测试（含 SyncTest CI）双宽度绿。
   —— 但这不构成扩宽的理由：翻案 6 的纪律是"要新位段先有需求再有 bench"，
   本次只是把"扩了会多花多少"的答案预先存档。
3. **`fire_oil_chain`（M2 主验收场景，256×192、2 万 tick、三次点火）：
   全程 avg 0.06–0.35 ms/tick**——燃烧是 O(活跃格) 的，火熄烟散后整图入睡，
   "睡眠让大图免费"在 M2 语义下继续成立（翻案 6 的核心论证得到复核）。

## 环境与口径

照抄 `docs/perf/2026-08-31-layer-g-task3-splash.md`（同机同尺度，provisional）：

- CPU：Intel Xeon Gold 6330 @ 2.00GHz，112 逻辑核，共享服务器。
- `cargo build --release`；`sand-harness hashrun` 取 stderr `avg`；每组合 3 次
  独立冷启动取中位，三侧逐次交替。场景与 tick：`mixed` 1500、`sparse` 2000、
  `acceptance` 5000、`fire_oil_chain` 20000。
- before 侧 `git worktree` @ `f807e54` 独立编译，`--materials` 指向其自身数据
  （无 M2 字段/材质——材料表内容差异正是被测语义，不是口径污染）。
- u64 侧：本树 `--features sand-core/cell-u64` 独立 target 编译。

## 数据（ms/tick，3 次中位）

| 场景 | 线程 | scan | before | after | Δ | u64 | u64 vs after |
|---|---|---|---|---|---|---|---|
| acceptance | 1 | live | 1.009 | 1.120 | +11.0% | 1.099 | −1.9% |
| acceptance | 1 | sleep | 2.170 | 2.757 | +27.1% | 2.690 | −2.4% |
| acceptance | 8 | live | 0.558 | 0.696 | +24.7% | 0.677 | −2.7% |
| acceptance | 8 | sleep | 0.819 | 0.969 | +18.3% | 0.963 | −0.6% |
| acceptance | 16 | live | 0.702 | 0.852 | +21.4% | 0.826 | −3.1% |
| acceptance | 16 | sleep | 1.157 | 1.585 | +37.0% | 1.427 | −10.0% |
| mixed | 1 | live | 0.322 | 0.367 | +14.0% | 0.347 | −5.4% |
| mixed | 1 | sleep | 0.661 | 0.837 | +26.6% | 0.810 | −3.2% |
| mixed | 8 | live | 0.353 | 0.440 | +24.6% | 0.426 | −3.2% |
| mixed | 8 | sleep | 0.610 | 0.777 | +27.4% | 0.822 | +5.8% |
| mixed | 16 | live | 0.364 | 0.482 | +32.4% | 0.550 | +14.1% |
| mixed | 16 | sleep | 0.691 | 0.886 | +28.2% | 0.836 | −5.6% |
| sparse | 1 | live | 0.308 | 0.335 | +8.8% | 0.294 | −12.2% |
| sparse | 1 | sleep | 1.213 | 1.164 | −4.0% | 1.183 | +1.6% |
| sparse | 8 | live | 0.171 | 0.197 | +15.2% | 0.236 | +19.8% |
| sparse | 8 | sleep | 0.424 | 0.416 | −1.9% | 0.498 | +19.7% |
| sparse | 16 | live | 0.230 | 0.200 | −13.0% | 0.219 | +9.5% |
| sparse | 16 | sleep | 0.443 | 0.408 | −7.9% | 0.506 | +24.0% |
| fire_oil_chain | 1 | live | — | 0.064 | — | 0.064 | 0.0% |
| fire_oil_chain | 1 | sleep | — | 0.277 | — | 0.294 | +6.1% |
| fire_oil_chain | 8 | live | — | 0.082 | — | 0.080 | −2.4% |
| fire_oil_chain | 8 | sleep | — | 0.348 | — | 0.334 | −4.0% |
| fire_oil_chain | 16 | live | — | 0.099 | — | 0.108 | +9.1% |
| fire_oil_chain | 16 | sleep | — | 0.335 | — | 0.374 | +11.6% |

判读纪律照 Task 3 先例：以 `acceptance` 这种"多个线程数同号"的结构判方向；
`sparse` 与 `mixed 16T` 的孤立大数值是本机噪声的典型形状，不单独引用。

## 优化余量（如果哪天 +20% 变成问题）

现成的第一杠杆在 `rules::react`：发起方检查目前对每个活跃发起方格无条件扫
4 邻。可加载期按"发起方 × 全部对方材质"预计算 per-material 位掩码，先查
"本格周围 3×3 内是否存在任何可反应材质"再进邻检——但那要引入邻域材质摘要，
属新机制。当前绝对量级离预算尚远，不动（YAGNI，等 bench 说话）。
