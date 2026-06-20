> 文档路径：`docs/reference/ep01-sandsim-comparison.md`
> 运行时版本：对照对象为 C + gunslinger/OpenGL
> 最近更新：2026-06-14 (UTC+8)

# 外部参考实现对照：GameEngineering EP01_SandSim

> 对照对象：[GameEngineering/EP01_SandSim](https://github.com/GameEngineering/EP01_SandSim)（YouTube 系列 EP01，"重现 Noita 落沙"）。
> 方法：静态通读核心 `source/main.c`（3215 行）+ README，对照 fallingTest 技术路线。未编译/运行。行号锚点均指 EP01 的 `main.c`。
> 结论先行：**EP01 是单文件、硬编码、用 `rand()` 的教学 demo；算法/确定性/数据驱动/并行我们全面更强；它唯一不可替代的价值是"实机 OpenGL 渲染 + bloom + velocity 手感"的现成参考。**

---

## 1. EP01 技术路线速览（事实，带锚点）

| 维度 | 事实 |
|---|---|
| **数据结构** | AoS 单缓冲。`particle_t`（`main.c:26-32`）：`u8 id` / `f32 life_time` / `gs_vec2 velocity`（2×f32）/ `color_t color` / `b32 has_been_updated_this_frame`，约 22 字节/格。世界平铺 `g_world_particle_data`，位置由 `compute_idx(x,y)=y*w+x` 隐式编码。color 存进 cell（渲染耦合）。 |
| **更新循环** | `update_particle_sim()`（`:1038-1093`）自底向上，x 每帧交替左右（`g_frame_counter%2`）——与我们一致。防同帧二动用**布尔字段** `has_been_updated_this_frame` + **帧末全量清零循环**（`:1087-1092`）。**无 chunk、无 dirty-rect、无多线程**，全量扫 + switch 分派。 |
| **材质系统** | 13 种材质，**全硬编码**：`#define mat_id_*`（`:55-68`），属性散落在各 `update_XXX()` 局部变量，**无属性表、无配置文件**。沉浮**无 density 字段**，靠硬编码 if-else 替换（Sand 沉 Water `:1502`、Water 沉 Oil `:3002`）。反应硬编码概率：Lava+Water→Steam+Stone（`:2359`）、火点燃 Wood/Oil（`:2107`）、Acid 溶解（`:2780`）。 |
| **运动规则** | Powder/Liquid/Gas 优先级链。有 velocity 积分（`velocity.y += gravity*dt`，截断 ±10，`:1456`），每帧跳 `(s32)velocity` 格 = **整数截断多格下落，无子像素精度**（无累加器、无概率取整）。液体 dispersion：water `spread_rate=5`（`:2952`）、oil=4、lava=1。 |
| **随机性** | `random_val()`（`:219`）= 标准库 `rand()`，全局隐式种子。**完全非确定、不可重放**。 |
| **渲染** | CPU 维护全像素颜色数组，每帧整张 RGBA8 纹理上传 GPU（`:1251`）+ 全屏 quad + NEAREST（`:659`）。**有 bloom 后处理**：bright filter → 分离高斯 blur → composite，`b` 键开关（`:942`）。 |
| **特效/寿命** | 每帧累加 `life_time`。Steam/Smoke 寿命 10s、Ember 0.5s、Fire 随机死亡 + 颜色红→橙→黄随机切换（`:1980`），Fire 派生 Smoke(1/500)/Ember(1/250)。**无独立粒子系统**——Ember 仍是格内材质。 |

## 2. 逐项对照

| 维度 | EP01 | fallingTest | 谁更适合我们目标 |
|---|---|---|---|
| 缓冲/布局 | AoS 22B/格，含 float velocity + color | 单缓冲平铺 int，STRIDE=5 紧凑整数 | **我们**（整数化利确定性 + C# Span；EP01 存 color 进 cell 是渲染耦合） |
| 遍历顺序 | 自底向上 + 左右交替 | 同 | 平手 |
| 防同帧二动 | 布尔标志 + 帧末全量清零 | `updated_at` 世代戳（免清零 pass） | **我们** |
| 分块/并行 | 无 | 4-pass 棋盘 chunk（64²+写域）锁多线程语义 | **我们**（EP01 单线程全量扫） |
| 材质定义 | 硬编码 enum+switch+散落局部变量 | 数据驱动 TOML + tag 通配反应表 | **我们**（加材质改表不改码） |
| 沉浮 | 无 density，硬编码替换 | density 整数等级数值比较 | **我们** |
| 反应 | 硬编码概率 if | u32 阈值反应表 | **我们**（符合 CLAUDE.md 禁 if-else 链） |
| dispersion | water5/oil4/lava1 | water5/oil2/lava1（已实现） | 平手（同思路） |
| 重力积分 | float velocity 截断多格，非确定 | 计划 8.8 定点累加器（确定） | **我们**（EP01 用 float dt 不可重放） |
| 随机 | `rand()` 全局种子 | counter SquirrelNoise5 坐标纯函数 | **我们碾压** |
| 联机/校验 | 无 | lockstep + state_hash(CRC32) | **我们** |
| 渲染 | 整纹理上传 + **bloom** + NEAREST | Python 逐像素→ImageTexture，未碰后处理 | **EP01**（实机 OpenGL，bloom 是我们没碰的） |
| 粒子 | 无独立粒子 | 计划 CA↔粒子双轨 | **我们** |

## 3. 值得借鉴的（EP01 有、对"横版弹幕 + 打击感"有用）

> 全部集中在**渲染/视觉/手感**——这是我们 Phase 1（Python 原型）刻意没碰、而 EP01 作为实机 demo 现成的部分。

1. **Bloom 后处理**（`:1288-1307`：bright filter → 分离高斯 blur → composite）。EP01 唯一明显强于我们规划的部分。**借鉴点：让 fire/lava/爆炸/弹幕的颜色亮度超阈值自动吃 bloom，不写单独发光逻辑。** Phase 2 迁 Godot 后用内置 `WorldEnvironment` Glow，思路一致（亮度提取 + 模糊 + 叠加）。
2. **整纹理一次上传 + NEAREST**（`:1251`/`:659`）。证明"CPU 维护全像素颜色数组、每帧整张上传"在 ~266K 像素实机可行。我们 Godot 阶段 `Image→ImageTexture` 走同路线，NEAREST 保像素锐利。
3. **velocity 多格跳的手感方向**（`:1456-1500`）。"越落越快、能溅开"对打击感（击飞碎屑、爆炸抛射）重要。**EP01 验证了手感方向对，但其 float 实现不可联机——我们做成 8.8 定点累加器版，保手感 + 保确定。**
4. **Fire 视觉分层**（`:1980`/`:2071`/`:2086`）：火本体 + 概率派生上飘烟 + 飞溅火星，三者叠加。我们 fire 系统可参考这套派生概率结构——**但派生概率走材质字段，不硬编码**。
5. **Ember = 短命飞行火星**（`:1830`）：低成本特效，弹幕命中地形的火花溅射可复用思路。

## 4. 我们明显更强 / 更适合

1. **确定性内核**（最大代差）：EP01 `rand()`+全局种子，完全不可重放/联机；我们 counter SquirrelNoise5 + 全整数化 + state_hash。lockstep 目标 EP01 连概念都没有。
2. **数据驱动材质 + 反应表**：EP01 加材质要写新函数 + 改 switch + 手写每对沉浮/反应；我们改 TOML 不改码。
3. **并行语义 / chunk 调度**：EP01 全量单线程扫 266K 格；我们 4-pass 棋盘 + 写域缓冲已为多线程锁语义。
4. **世代戳 vs 布尔标志**：EP01 每帧多跑一趟全量清零 pass，我们 `updated_at` 省掉。
5. **重力积分确定性路线**：EP01 float velocity 依赖 dt、跨平台不可复现；我们 8.8 定点。

> **诚实校准**：上述"更强"是**架构事实**，但 EP01 是**已跑通的实机**，我们这些优势目前多在 Python 原型 + 规划阶段，**手感与性能尚未实机验证**——这是纸面优势待兑现的部分。

## 5. 一句话总评

EP01_SandSim 是单文件、硬编码、用 `rand()` 的**教学型 demo**（C + gunslinger/OpenGL，13 材质 + bloom），算法深度远低于我们的确定性内核规划；它对我们唯一不可替代的价值是作为**"实机 OpenGL 渲染 + bloom 视觉 + velocity 手感"的现成参考**——证明了我们 Phase 1 还没碰的渲染/打击感方向可行，而算法、确定性、数据驱动、并行我们全面更强。

## 附：可落地的借鉴项（已并入路线）

- velocity 8.8 定点（队列 #2）：EP01 印证多格下落手感方向，我们保确定性实现——**已记入 `docs/overview/architecture.md` §4 待办**。
- bloom / 发光：Phase 2 迁 Godot 后用 WorldEnvironment Glow，靠材质颜色亮度阈值驱动——本文档为唯一记录点，Phase 2 视觉规划时取用。
- fire 视觉分层 + 派生概率（走材质字段）：fire 系统实施（队列 #4）时参考。
