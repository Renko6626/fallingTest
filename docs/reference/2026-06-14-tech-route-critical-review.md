> 文档路径：`docs/reference/2026-06-14-tech-route-critical-review.md`
> 运行时版本：Python 3.x（Phase 1）→ Godot 4.5 + C#（Phase 2+）
> 最近更新：2026-06-14 (UTC+8)

# 技术路线批判性复核（deep research，2026-06-14）

> 本篇是对既定技术路线的**第三方批判性检查**（5 角度并行调研 → 21 源 → 94 claim → 25 条对抗式验证，15 confirmed / 10 killed）。
> 与 `noita-deep-dive.md`（算法对照）、`noita-multiplayer-and-determinism.md`（联机调研）互补——本篇专做【增量 + 反驳】。
> 性质：外部证据复核，**不单方推翻已批准决策**；actionable 项以"建议"形式落到 proposal/architecture，待相应里程碑裁决。

## 0. 一句话结论

> 多数关键决策有充分的工业/学术/社区先例支撑（确定性内核方向、写域几何、运动学、数据布局）；**联机架构是最薄弱、风险最高的一环**——最成熟同类产品主动放弃了我们计划的 lockstep 路线。

---

## 1. 方案站得住的地方（confirmed，可继续）

| # | 结论 | 证据（vote） |
|---|---|---|
| S1 | **整数/8.8 定点 + counter RNG、规避浮点**的大方向正确 | 跨平台浮点默认无位级一致是工业共识；Teardown 把体素切割改写为定点整数。Gaffer/Bruce Dawson/Gustafsson（3-0） |
| S2 | **正方形写域 `[chunk−32,+96)²` 比社区流传的"十字/cardinal"更正确** | 真实不变量是"单像素一帧移动 ≤32px"含对角线；80.lv 的 "cardinal cross" 是已知简化。Purho GDC2019 + macuyiko（3-0） |
| S3 | **运动学方向正确**（优先级链 + 速度积分） | jason.today 显式积分 maxSpeed=8/accel=0.4 + 碰撞归零，与我们即将加的 velocity 积分同构（3-0） |
| S4 | **单缓冲 + 位置由网格索引隐式编码 + 自底向上交替遍历**是标准做法 | sandspiel 完全一致（dense 2D array + clock 标志解扫描自撞）；我们用自底向上+交替遍历，同样有效（3-0） |
| S5 | **弹幕层走全 lockstep（路线 A）可行**——天然确定性子系统最适合 | Factorio 纯输入流 lockstep 高度可扩展；弹幕固定轨迹、可整数化，比自演化地形更适合 lockstep（3-0） |

→ **行动**：S1–S5 均确认现有计划，无需改动。velocity 实施按 8.8 定点推进（见 §3 待决）。

## 2. 需修正 / 高风险盲点（confirmed，必须吸收）

### R1 —【高风险】迭代顺序 desync：counter RNG 远不够 ⭐
**确定性 RNG 完美也不保证内核确定**——代码隐式依赖集合迭代顺序（Dictionary/HashSet）会在相同 seed 下 desync。
- 证据（3-0）：galsov.com 直述；**Box2D 作者 Erin Catto 亲证**"contact array 顺序不同 → 解算结果不同 → 非确定"；Factorio/Ruoyu Sun 同列 "unordered iteration" 为 desync 主因。
- **对我们的直接威胁**：M1 C# 迁移按 chunk/entity 遍历时，若用 `Dictionary`/`HashSet` 即 desync——Python dict 插入有序会掩盖此 bug 到迁移才爆（正是 D3 已预警的 "迁 C# 即翻车"）。
- **行动**：proposal §3 **D3 契约补强**——显式要求"任何进入 sim 的遍历必须有稳定排序（`SortedDictionary` / 显式 sort by 稳定 key），禁裸 `Dictionary`/`HashSet` 迭代"。已同步到 architecture.md §8 不变量速查。

### R2 —【高风险】联机：lockstep 可能不是生产路径 ⭐
最成熟同类先例 **Noita Entangled Worlds**（2024-05 创建，维护至 2026-06，v1.6.3，~1.2k★，216 releases）**主动放弃 lockstep 确定性重模拟**，改用**按-chunk 权威所有权转移 + RLE 像素状态同步**。
- 证据（状态同步 3-0 / 存在性 2-1）：README 同步项含 "Pixels of the grid world"；架构为 authoritative-replication（"divides into chunks, RLE to transmit only changed pixels"，"only one client can modify a chunk at a time"），明确 "cannot achieve perfect physics sync... reasonable approximation"。
- **这是对我们"地形 lockstep 命令流"（提案路线 B 地形层）的直接反数据点**——业界最成熟的 CA 联机产品拒绝了 lockstep。
- ⚠️ 辩证：Entangled Worlds 选状态同步是**被迫的**（mod 改不了 Noita 不确定引擎，与我们自研引擎处境不同——这点 `noita-multiplayer-and-determinism.md` 已论证）；且其像素同步被多源描述 "far from perfect / many bugs"。所以它**既证 CA 地形联机可行，又反证 lockstep 不是唯一/必然路径**。不要把它当 lockstep 先例引用。
- **行动**：proposal §4 加注此反数据点；**强化既有退路 C 的地位**（我们的退路 C = chunk diff 流，正是 Entangled Worlds 路线）；M2 spike 必须把"路线 B 地形 lockstep" vs "路线 C/Entangled 式权威所有权"作为头号对照实测项。

### R3 —【中风险】刚体桥接的浮点确定性陷阱
"离散网格 = 确定性容易"被低估：像素破坏导致刚体**分裂/关节重连**的后续逻辑仍依赖浮点。
- 证据（3-0）：Gustafsson "object hierarchies may separate... a lot of this still involves floating point math"。
- **对我们**：Phase 2 刚体桥接（Marching Squares→Douglas-Peucker→三角化→Box2D），一旦像素破坏触发刚体重算/分裂，几何浮点可能跨平台不一致。
- **行动**：开放问题——刚体若做不到位级确定，可能必须**降级为状态同步**，使"地形/弹幕/刚体"成为**三层而非两层**同步。记入 proposal 开放问题，M1/M2 评估。

## 3. 待决设计岔路（openQuestion，velocity 实施前定）

**子像素运动：8.8 定点累加器 vs 概率取整？**
- jason.today 用**概率取整**（`floor + (random()<frac?1:0)`）——但**依赖 RNG**，对确定性联机内核引入额外 desync 面 + RNG 调用次数耦合。
- 报告建议：**优先 8.8 定点子像素累加器**（确定性、无 RNG 依赖），比 jason.today 教程更适合本项目。
- → 与 D1 既定方向（velocity 定点）一致，本复核**确认**该选择。velocity spec 落地时直接按定点累加器设计，不引入概率取整。

## 4. 可借鉴但没做（参考）

| # | 项 | 说明 |
|---|---|---|
| B1 | **刚体桥接开源样板** | FallingSandSurvival 源码完整实现了我们 CLAUDE.md 命名的同一管线（cpp-marching-squares → douglas-peucker → polypartition → Box2D），`world.cpp` 按序串联，Phase 2 可直接对照（2-1） |
| B2 | **sandspiel 32-bit bitpack cell** | `{species, ra:u8, rb:u8, clock:u8}`——比我们 STRIDE=5 int 数组紧凑，是 C# 迁移 bitpacking vs 平铺的对照样板（3-0） |

## 5. 被否决的论断（⚠️ 禁写进路线图依据）

对抗式验证 **3 票否决**，不可作为决策依据：

| 被否决论断 | vote | 为何否 |
|---|---|---|
| "SoA 在 C# 必然 ~2x（+SIMD ~10x）提速" | 0-3 | 单一项目（Cysharp）README 自报 benchmark，marketing 性质。落沙内核是**随机邻域读写**为主，未必吃到 SoA 顺序流式收益 |
| "Teardown 改定点是因为浮点不安全" | 0-3 | 实为设计选择；Gustafsson 后来重评全确定性 "mostly safe if you know the pitfalls" |
| "Teardown 两层架构精确镜像我们双层" | 1-2 | 过度类比 |
| "跨平台浮点具体数值差异 / 64-bit 12-shift 定点方案" | 0-3 / 1-2 | 来源/细节存疑 |

→ **行动**：**C# 迁移不预设 SoA 收益**——STRIDE=5 平铺 / SoA / bitpack / chunk-local 必须用本项目代表性 workload 实测（CLAUDE.md §5.3 已要求）。本复核把"SoA 更快"从假设降级为"待实测假说"。

## 6. 行动项汇总

| # | 行动 | 落点 | 时机 |
|---|---|---|---|
| A1 | D3 契约补强：sim 遍历禁裸 Dictionary/HashSet，强制稳定排序 | proposal §3 + architecture §8 | **现在**（已落） |
| A2 | 联机：标注 Entangled Worlds 反数据点，强化退路 C，M2 头号对照项 | proposal §4 | **现在**（已落注记）+ M2 |
| A3 | 刚体可能成第三同步层 | proposal 开放问题 | M1/M2 评估 |
| A4 | velocity 用 8.8 定点累加器，不用概率取整 | velocity spec | 下一个玩法项 |
| A5 | C# 数据布局不预设 SoA，实测裁决 | M1 benchmark | M1 |

## 附：来源质量分层

- **一手/作者级**（高可信）：Gaffer、Bruce Dawson、Box2D 作者 Catto、Factorio FFF、Teardown 作者 Gustafsson、sandspiel 作者 Bittker、Purho GDC2019。
- **时效**：Entangled Worlds 维护到 2026-06-10，当前最新同类联机先例。
- 本报告为合成产物，结论基于 15 条通过对抗式验证的 claim；运动学/数据布局结论是对"即将实现"方案的前瞻评估，非现有代码审计。
