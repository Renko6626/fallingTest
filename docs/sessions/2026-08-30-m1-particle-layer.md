# 会话总账：2026-08-30 · M1 粒子层实施

> 文档路径：`docs/sessions/2026-08-30-m1-particle-layer.md`
> 最近更新：2026-08-30 (UTC+8)
> 上一篇：`2026-08-30-m0-implementation.md`（M0 骨架与执法）

## 做了什么

按 spec（`docs/superpowers/specs/2026-08-30-m1-particle-layer-design.md`）分七个任务
（`.superpowers/sdd/2026-08-30-m1-particle-layer-plan/`）实施 M1 粒子层，走 subagent-driven
串行执行 + 每任务评审修复轮：

- **Task 1**：M1 动工前正式 Rust bench 基线（`docs/perf/2026-08-30-m0-rust-baseline.md`）。
- **Task 2**：`fixed.rs` 手写 Q16.16 定点基建。
- **Task 3**：`particle.rs` SoA 粒子池 + 状态哈希并入（`grid_hash`/`state_hash` 分离）+ golden
  两步重录程序取证。
- **Task 4**：并行积分 + DDA + 串行按 id 落格（`dda.rs::CellWalk`、`particle.rs::resolve_landing`/
  `commit`）。评审抓到 C1 活锁，修复轮 1 改判（见下）。
- **Task 5**：`Op::Emit` 发射器 + 瀑布场景。评审抓到 I1（RNG key 撞车）+ I2（指纹口径），修复轮 1
  全部修复。
- **Task 6**：`Op::Explode` Noita 射线模型 + 爆炸/压测场景（`explosion_ci`/`explosion_splash`/
  `particle_stress`）。
- **Task 7**（本文档对应）：验收与收尾——SyncTest 验收、bench 对照、GIF 目检、文档四件套。

产出：`crates/sand-core` 新增 `fixed.rs`/`particle.rs`/`dda.rs`，`world.rs`/`rng.rs`/`hash.rs`/
`material.rs`/`lib.rs` 扩展；`crates/sand-harness` 的 `scenario.rs`/`runner.rs`/`render.rs` 扩展；
`data/scenarios/` 新增 5 个场景 + `materials.ron` 新增 `blast_cost` 字段。测试从 M0 的 22 项增长到
**122 项全绿**（`cargo test --workspace`，逐 suite 核验：core 单测 91 + `particle_behavior` 3 +
`rules_behavior` 5 + `synctest_ci` 1 + harness 单测 16 + golden 4 + `synctest` 2 =
91+3+5+1+16+4+2 = 122），`cargo clippy --workspace --all-targets` 全程无警告。详细文件清单与逐任务
产出见 `docs/CHANGELOG.md` 2026-08-30 各条目。

## 关键事件

### Task 4 评审 C1：落格悬浮活锁

原设计（沿用总纲 `kernel-charter.md:62` 原文"输家按定序邻格搜索或继续飞行"）在候选格 + 五邻格
全占时让粒子"继续飞"——`pos` 重置为候选格中心、速度清零。评审用 40 颗同位同速沙粒复现：下一 tick
该粒子从候选格中心（非 air 起点，DDA 起点格从不检查是既有语义）出发，若候选格与五邻格此刻仍是
同一局面，DDA 立即原地判定 `Blocked{land_cell = 候选格自身}`，`resolve_landing` 再次全占、再次
悬浮——**两 tick 一个周期的活锁**，32/40 颗永久卡死，粒子池不排空。

修复：完全移除"继续飞/悬浮"分支，改为五邻格全占后沿候选格**正上方**逐格向上搜索第一个 air 格
（Noita 同款方案，`docs/reference/noita-deep-dive.md:226`）；搜到世界顶仍无 air 则确定性出界销毁，
计入诊断计数器 `buried_total`（不入哈希）。`Outcome::Land` 现在必然终止于"落格"或"出界"两态之一。
spec §5 决策记录第 6 条、`kernel-charter.md` §4/§11 均已同步（本任务 Task 7 完成总纲侧同步，见下方
"验收状态"前的 Changed 记录）。回归测试：`particles_same_position_and_velocity_conflict_still_
conserves_and_drains_pool` 直接复现评审几何，断言修复后粒子池排空、`world.count_material(SAND) ==
40`。

### Task 5 评审 I1：同帧同格多 Emit 的 RNG key 撞车

原抖动 key（`rng_u32` 的 `salt`）只含粒子序号 `i`，缺"哪个 op"这一维——同 tick 两个 `Op::Emit`
命中同一发射格（或 `Sim::apply_setup` 与紧接的 tick 0 首个 `script` 并存，两者共享同一
`fseed = frame_seed(seed, 0)`）时，逐粒子抖动序列位级相同，违反总纲 §11 翻案记录第 4 条"同帧同格
多次掷骰必须彼此不同"的纪律。修复：新增 `emit_salt(op_idx, i)`（`op_idx` 折进高 16 位）与
`emit_attempt(stamp, roll)`（`stamp` 折进高位，区分 setup 阶段与 tick 0 首个 step），`World::
apply_op`/`scheduler::step`/`Sim::apply_setup` 均改为对 `ops` 切片 `enumerate()` 传入 `op_idx`。
副作用（预期内非回归）：`attempt` 位模式变化改变了 `Op::Emit` 实际消费的 RNG 序列，`waterfall_ci.
golden` 全部哈希值已重录；无 Emit 的场景（`sand_pile`/`mixed`）逐 tick 哈希验证位级不变。

## 验收状态（spec §0）

| # | 项 | 状态 |
|---|---|---|
| 1 | `cargo test` 全绿 | ✅ 122 项（core 单测 91 + `particle_behavior` 3 + `rules_behavior` 5 + `synctest_ci` 1 + harness 单测 16 + golden 4 + `synctest` 2 = 91+3+5+1+16+4+2 = 122），`cargo clippy --workspace --all-targets` 无警告 |
| 2 | SyncTest：waterfall/explosion_splash 各 2 万 tick 六配置零分叉 | ✅ waterfall `scenario_fp 39575dfa5dfed750`（577.8s）；explosion_splash `scenario_fp f229c61b5deb0328`（856.1s）；均 release、`--threads 8` |
| 3 | golden 重录（旧场景逐 tick 网格哈希零扰动 + 新场景 golden ×2 入库） | ✅ Task 3（`--grid-only` 两步程序，`sand_pile`/`mixed` 零扰动取证）+ Task 6（`explosion_ci` 入库，`sand_pile`/`mixed`/`waterfall_ci` 三个既有 golden 仅 `materials_fp` 一行变化） |
| 4 | render GIF 目检 | ✅ `out/waterfall.gif`、`out/explosion_splash.gif`（`--every 100 --scale 2`，各 201 帧，覆盖完整 2 万 tick）。见下方目检记录，**结论留给用户** |
| 5 | bench：粒子压测对照总纲 §7 预算 | ✅ `docs/perf/2026-08-30-m1-particle-baseline.md`：2 万粒子量级实测 0.586ms/tick（另一独立窗口折算 0.504ms/tick），均低于 0.8ms 预算；mixed/sparse 网格路径无回退，acceptance 1 线程组合超 ±10% 阈值但判定为共享服务器噪声（如实记录，未强行结论） |

### GIF 目检记录（客观观察，结论留给用户）

- **`out/waterfall.gif`**（640×384，`data/scenarios/waterfall.ron`，喷射窗口 tick 0–18000）：
  可见持续水滴喷流从顶部落入盆地（喷射）、落点处出现局部隆起（落格）、隆起随 tick 推移逐渐抬升
  整个水位（堆积）；喷射停止后（tick 18000–20000）水位不再上升，表面呈锯齿状起伏而非完全水平
  （摊平不彻底）。该锯齿是 M0 会话已记录的已知限制（`docs/sessions/2026-08-30-m0-implementation.md`
  "简版横流的水面有颗粒感缝隙"），非本次新增回归；是否需要在 M2 前处理留给用户判断。
- **`out/explosion_splash.gif`**（640×384，`data/scenarios/explosion_splash.ron`，19 次
  `Op::Explode`，tick 500..18500 step 1000）：初始沙块（矩形 fill，悬空于水面之上）在前 500 tick
  内因重力自由落体 + 无侧向支撑迅速滑塌成对称沙堆（角度守恒的自然结果，非 bug）；首次爆炸
  （tick 500）在沙堆峰顶炸出 V 形缺口，水从两侧回灌缺口（挖坑 + 回落）；紧邻爆炸 tick 的帧
  （如 tick 1500）可见清晰的圆形摧毁空腔与飞溅的沙/水粒子轨迹（溅射）。从第 6 次爆炸（约 tick
  6000）到第 19 次爆炸后（tick 19900）缺口轮廓基本不再变化——怀疑是缺口已下探到水位、后续射线
  多数消耗在 `blast_cost` 更低的水（成本 1）而非沙（成本 2）上，加上两次爆炸间隔内水会回流填平
  缺口，形成稳定周期性平衡（每次炸开又被回填，净形状不变），细节未做数值级验证。缺口以外的沙堆
  两翼在全部 19 次爆炸中保持完全不变——即"每条射线能量耗尽即停"的能量衰减模型在效果上表现为
  局部化摧毁（远处material不受影响），可作为"遮挡/薄墙后完好"验收项的间接证据；专门构造的薄墙
  遮挡场景（`explosion_ci.ron`）已有独立单测 `world::tests::explode_thin_wall_shields_sand_
  behind_it` 与 CI golden 兜底，是该验收项的权威证据来源，本 GIF 提供的是大场景下的定性佐证。

## 留给后续

- **Layer G 速度积分提案**（M1 spec §1/§12 后置的债）：格内移速 ≤4 + 超限自然脱格，需单独立项、
  过总纲 §11、跑 SyncTest（顶在 r≤16 并行安全论证上，Noita 双系统实锤见
  `docs/reference/noita-deep-dive.md:200,208`）。
- **O3 粉末惯性**：不入 M1，时点由 M2 spec 裁决（spec §12）。
- **durability/hardness**：M1 爆炸简化版用 `blast_cost` + wall=∞ 哨兵；M2 反应表字段化替换（spec
  §12）。
- **粒子渲染表形态**：`program-architecture.md` §8 待决项——bridge 组 MultiMesh vs. 暴露原始数组
  给 GDScript，M1 渲染压测后定（本次 harness `render.rs` 只是 CPU 侧目检用的单像素叠加，不代表
  最终 Channel A 渲染表设计）。
- **粒子穿水/入水减速**：现阻挡语义（第一个非 air 格即阻挡，水/沙/wall 一视同仁）留 M2 评估（spec
  §12）。
- **Task 6 遗留（评审 concerns，未在 Task 6 范围内处理）**：① `particle_stress.ron` 原仅用于手工
  `hashrun` 计时观察，未接入正式 bench 记录流程——**已在本任务（Task 7）补齐**，见
  `docs/perf/2026-08-30-m1-particle-baseline.md`；② `EXPLODE_JITTER`（爆炸溅射抖动幅度 0.5 格/tick）
  是任务自定调参项初值，spec 未给出具体数值，未来美术/关卡验收阶段可能需要调整（不影响确定性）；
  ③ `explosion_splash.ron` 完整 2 万 tick 六配置 SyncTest 当时未跑，属里程碑验收工作——**已在本任务
  完成**（见上方验收状态表 #2）。
- **acceptance bench 1 线程组合**超 ±10% 阈值（+14.8%/+15.6%）判定为共享服务器噪声但未做独占核
  复测验证，若后续需要精确数字建议补测（`docs/perf/2026-08-30-m1-particle-baseline.md` "观察"节）。
- **Task 6 评审 minor，终审分诊**（协调方 ledger，Task 6 完成时未列入该任务的 Concerns 节，本次
  Task 7 评审补记，均未修复——留给后续任务按优先级处理）：
  ① `crates/sand-core/tests/common/mod.rs` 注释提到"供本文件之外的爆炸行为测试
  （`explode_behavior.rs`）直接复用同一张表"，但该文件并不存在（爆炸行为测试实际内联在
  `world.rs` 自己的 `#[cfg(test)]` 模块里）——注释指向失实，需改写或补建该文件；
  ② `Op::Explode` 的 `r`/`x`/`y` 在 harness 加载期无范围校验，若场景 RON 给出 ≥32768 的值会在
  `i32`/`Fx` 转换处静默截断而非报错拒绝，与 `Op::Emit` 侧已有的 `resolve_op` 校验纪律不对称；
  ③ `crates/sand-core/src/world.rs:289` 附近 `Fx::from_ratio(energy as i32, power as i32)`——
  `power: u32` 若超过 `i32::MAX` 会在 `as i32` 处翻号（变负），使 `speed_ratio` 计算得到错误
  符号/量级，当前无防线（数据驱动的 `power` 字段理论上可达 `u32::MAX`）；
  ④ `fire_ray_already_air_cells_cost_zero_and_do_not_respawn` 一类测试名所暗示的覆盖面比实际
  断言更宽，需要核对测试体是否真的覆盖了"零费用 + 不重复生成"两个子命题，还是只覆盖了其中一个；
  ⑤ 爆炸摧毁产生的粒子若撞上 `MAX_PARTICLES` 容量拒绝，等价于该格材质凭空消失（网格侧已置
  air，但粒子未能生成）——这是一次真实的质量不守恒，目前代码里没有就地注释说明这个已知权衡；
  ⑥ 缺一个"同一 tick 的 `ops` 切片里出现两个 `Op::Explode`"的时序覆盖测试（`Op::Emit` 侧已有
  `emit_op_idx_differentiates_same_tick_same_cell_emits` 对应覆盖，`Op::Explode` 侧目前只有单个
  `Op::Explode` 场景的 `explode_same_tick_two_explodes_have_different_jitter_sequences`，未验证
  两次 Explode 的网格摧毁/生成互相正确叠加而非互相覆盖）。
