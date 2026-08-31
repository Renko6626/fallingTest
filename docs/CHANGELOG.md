# Changelog — fallingTest 文档与开发记录

本文件按"日期 → 条目"倒序记录 docs/ 下的产出与重要发现。
日期使用 UTC+8。所有条目应给出受影响文件路径。

## [Unreleased]

## 2026-08-31

### Changed
- **两项挂在用户身上的裁决收口（用户口头裁定，2026-08-31）**：
  ① **Layer G Task 1 的 GIF 目检通过** —— 验收 §0 第 7 项清账，Task 1 全项完成
  （`docs/superpowers/specs/2026-08-31-layer-g-velocity-design.md` 实施进度表已更新）。
  ② **四相棋盘的镜像不对称残留偏置（−0.8%）裁定为特性，不算 bug，不修** ——
  它是四相棋盘（总纲 §4 核心调度）的固有性质，修它要动调度本身，代价与收益不成比例；
  量级也低于可感知阈值。规避措施照旧采用（竞技地图镜像轴避开 64 的倍数，放 chunk 正中
  实测降到噪声级、CI 跨 0）。**该条不再列为待办，M4 不重开。**
  受影响文件：`docs/proposals/2026-08-31-powder-scan-direction-bias.md` §7 第 1 条、
  `docs/overview/kernel-charter.md` §11 实施期决策第 3 条、`docs/README.md` 优先队列、
  上述 spec 实施进度表。

### Fixed
- **双机 hashrun 完成（M0 起挂账的唯一跨机验收项，用户在 Windows 侧执行）——
  仿真确定性通过，指纹层查出真缺陷并修复**（总纲 §11 **实施期决策第 4 条**）。

  **正面结论（重要）**：Linux（rustc 1.89.0）与 Windows（rustc **1.97.1**）在 9 个场景、
  最长 2 万 tick 上，**全部 tick 哈希与 final 逐位相同**。这比总纲 §1 承诺的"同版本
  二进制互联"条件更严——跨了编译器大版本仍逐位一致。（**不构成新承诺**，§1 的口径不变，
  这只是一个正面数据点。）验收项 `docs/sessions/2026-08-30-m0-implementation.md` 表格第 3 行
  的 ⬜ 可以勾了。存档：`out/hashrun-zhustation.txt`（Linux）与 Windows 侧同名文件。

  **查出的缺陷**：`scenario_fp` / `materials_fp` 两端不同（9 场景 × 2 = 18 行，其余全同）。
  根因是握手指纹直接哈希磁盘原始字节（`scenario.rs` 的 `load_materials`/`load_scenario`），
  而 Windows 的 `core.autocrlf=true` 把 RON 检出成 CRLF。**同一个 commit、仿真逐位一致，
  却算出不同指纹** —— 这是 P5 的假阳性：语义相同的数据被判为不同版本。用户已用
  "输入 LF 归一化后重跑"精确复现 Linux 的 `c1558de1ce2c433e` 与 4 个 `scenario_fp`，
  证据链闭合（非推测）。第二个后果：Windows 上 `--test golden` **4/4 全挂**，因为
  golden 文件被检出成 CRLF 而 harness 输出恒为 LF，纯文本比对对不上（`golden.rs:28`）——
  CI 只在 Linux 跑，故一直没暴露。

  **修复三件套**（缺一不可）：① `scenario::normalize_for_fingerprint` 哈希前剥 CR（治本）；
  ② `.gitattributes` 钉死 `*.ron`/`*.golden` 的 LF 检出（治检出层——只有 ① 挡不住
  golden 的**纯文本**比对，那不是哈希问题）；③ `golden.rs` 比对也做行尾归一化
  （只有 ② 挡不住 zip 分发与编辑器改行尾）。**未选"改哈希解析后的结构"**这条更彻底的
  路：裸字节哈希有"自动完备"这个宝贵性质（字段新增自动进指纹），结构化折叠漏一个字段
  就是静默覆盖漏洞（`fold_fx_fields` 正是那种局部结构哈希）；归一化保住完备性同时消掉
  唯一已知假阳性。

  **指纹值不变，golden 无需重录**：仓库内 `.ron` 全部 CR=0，归一化在规范内容上是恒等
  变换 —— 实测改动后重跑与改动前归档 **229 条哈希 + 18 条指纹逐字不变**。CRLF 机器是
  向既有 LF 值**收敛**，不是双方都换新值。（这一点与初步评估不同，初评认为要重录 4 个
  golden。）

  新增 3 条回归测试：`materials_fingerprint_is_line_ending_agnostic`、
  `scenario_fingerprint_is_line_ending_agnostic`、`normalize_is_identity_on_lf_content`。
  受影响文件：`.gitattributes`（新增）、`crates/sand-harness/src/scenario.rs`、
  `crates/sand-harness/tests/golden.rs`、总纲 §11。

  ⚠️ **已存在的工作树需要重新归一化**才能享受 `.gitattributes`：
  `git add --renormalize . && git checkout -- .`，或重新 clone。

- **粉末堆积的系统性方向偏置：行扫描定向改为每 tick 全局哈希相位**
  （`docs/proposals/2026-08-31-powder-scan-direction-bias.md`，Status: Implemented；
  总纲 §11 **实施期决策第 3 条**，并**订正总纲 §4 正文** `kernel-charter.md:58`）。

  **起因**：用户目检 Task 1 的 `out/mixed_disp5.gif` 时发现左右不对称并提出质疑。
  排查确认**与 Task 1 无关**——`powder_step`（`rules.rs:98`）从不调 `side()`，且
  `mixed_disp1`/`mixed_disp5` 的沙分布**逐列完全相同**。GIF 里"水位左右不平"经查是
  **正确行为**：沙堆在 x=159..164 堆到 y=154 高过右池水面，把盆地切成两个不连通的池子。

  **根因**：`(y + tick) & 1` 对**运动中**的粒子自我抵消失效——自由下落者 `y+1`/
  `tick+1` 使奇偶恒定，整个下落被锁死在同一扫描方向，交替只对静止粒子生效。
  仪器实测（对称场景）：LTR 行斜右 +45.19%、RTL 行斜左 −41.58%（每行内部偏置等量
  反向，交替**设计意图正确**），但两类行移动总量 38823 vs 28981（差 34%），加权不平
  ⇒ 净 +8.11%。更一般地，**任何周期为 2 的定向方案都会与周期为 2 的动力学共振**
  （`tick & 1` 实测 +9.61%，比原方案更差——这条对照否掉了"下落锁定"是唯一机制的
  初始假设）。`diag_side` 掷骰本身公平（378 万样本 50.010%），已排除 RNG 偏斜。

  **修复**：`(y ^ scan_flip(fseed)) & 1`，新增 `STREAM_SCANDIR = 4`（`rng.rs`）。
  三个设计选择：① key 只取 `tick`、不掺 `x`/`y`/chunk——方向若随 chunk 变，物理行为
  就依赖 chunk 划分，在竖缝上引入新各向异性；② 取 **bit16** 而非 bit0，与 `diag_side`
  错开——`rng_u32` 的 `pos` 是线性折叠非单射，存在一条格子集合使两次掷骰返回**完全
  相同的 u32**（当前远在世界宽度外，但那是凑巧安全非构造安全），执法测试
  `rng::tests::scandir_bit_independent_of_diag_bit`；③ 编号 4 而非 3——3/5 已由
  Layer G Task 2/3 的 `STREAM_FALLSTEP`/`STREAM_SPLASH` 预留，一次排好免得撞号后
  再作废一次 golden。

  **数据**（对称挡板场景，32 种子，沙量左右差）：

  | 方案 | 均值 | 95% CI | 同向 |
  |---|---|---|---|
  | `(y + tick) & 1`（原） | **−6.266%** | [−6.46, −6.07] | 32/32 |
  | B 逐行哈希 | −1.169% | [−1.27, −1.07] | 32/32 |
  | **B′ 每 tick 全局相位（采纳）** | **−0.769%** | [−0.84, −0.70] | 32/32 |

  三者都不跨 0 ⇒ **还有第二个偏置源**。几何对照实验（16 种子，512×192）把它分离出来：

  | 镜像轴 | 原方案 | B′ |
  |---|---|---|
  | chunk **边界** | −6.383%，16/16 | −0.795%，16/16 |
  | chunk **正中** | +1.705%，15/16 | **+0.042%，CI [−0.044,+0.127] 跨 0，9/16** |

  ⇒ **B′ 在相位对称几何下彻底消除扫描方向偏置**；残留的 −0.795% 来自**四相棋盘的
  镜像不对称**（镜像轴压在 chunk 边界时缝两侧属不同相位，处理次序不镜像），是独立
  的第二个 bug，**本次不修**，规避措施 = 竞技地图镜像轴避开 64 的倍数（提案 §7）。
  注意原方案在两种几何下**符号相反**，两个源会相互干涉——这正是必须做几何对照才能
  分开它们的原因。

  **一条方法论教训**：单种子曾测得 `y & 1` 给出 +0.07%，看着完美，**32 种子下站不住**
  （`y & 1` 另有致命伤：方向对每行永久固定，被约束在少数行内的结构会拿到永久无抵消的
  方向偏好；且 Task 2 让 n ∈ 1..4 后在 n 为偶数时重新锁死）。单点数据不能证明"偏置消失"。

  **升格一条红线**（此前是隐含前提）：**行扫描方向必须是 `(tick, y)` 的纯函数，禁读
  活矩形/扫描起始矩形/`own_ci`/脏状态/线程上下文**。O1 活矩形的三模式逐位等价论证要求
  全扫访问序 V 三模式一致，而 V 的行内定向正由它给出。该论证本身**对方向取值不敏感**
  （承重的是"起点用快照、终点每步重读"的非对称结构），故改定向不破等价性——已写入
  `rules.rs::update_chunk` 与 `rng::STREAM_SCANDIR` 文档注释，执法靠 SyncTest 六配置。

  **验收**：① `cargo test --workspace` 全绿（新增 2 条 RNG 执法测试）、`clippy
  --all-targets` 零警告；② **SyncTest 六配置在重录 golden 之前先绿**（`synctest_ci`
  126s）；③ golden **预期先落再验**："四个全变，含纯沙的 `sand_pile`"——实测完全兑现
  （若 `sand_pile` 没变即说明改动没生效，这是反向哨兵）；④ 2 万 tick 六配置
  （waterfall + mixed）；⑤ **`--grid-only` 逐 tick 基线重建并归档**
  `docs/perf/baselines/*.grid-only.txt`——**Layer G Task 2 的 `G_ACCEL=0` 逐位回归
  以此为新基线**，漏了这步 Task 2 最硬的取证就废了。

  **bench 无回退**（同机同轮次 before/after，`git worktree` 另建 `HEAD` 工作树独立编译）：
  `acceptance`（640×384，最接近总纲 §7 目标量级）8/16 线程一致变快 10–20%（8T live
  **−19.9%**）；`mixed`/`sparse` 在 ±8% 内。**一条测量教训**：`sparse 1T live` 初测
  **+42.2%**，单独重测 7 次后为 **+3.9%**，两侧区间大幅重叠——该格在本共享机上自身抖动
  约 45%，初值纯噪声。理论成本只是每 tick 每 chunk 一次 squirrel5，可忽略。

  受影响文件：`crates/sand-core/src/{rng,rules}.rs`、`crates/sand-harness/tests/golden/*`
  （4 个全部重录）、`docs/perf/baselines/`（新增）、总纲 §4+§11、
  `docs/superpowers/specs/2026-08-30-o1-live-rect-design.md`（访问序 V 定义 + 红线）、
  `docs/superpowers/specs/2026-08-29-m0-skeleton-design.md:105`、`docs/README.md`。

### Added
- **Layer G Task 1 落地：液体色散 ≤8**（spec `docs/superpowers/specs/2026-08-31-layer-g-velocity-design.md` §3；
  三 Task 的第一个，色散打头）。`crates/sand-core/src/rules.rs:126` 的 `side()` 从"探 1 格"改为
  "沿方向探至多 `dispersion` 格、遇非 AIR 即停、移到**最远可达空格**"；`material.rs` 加 `dispersion`
  字段 + 访问器 + `DISPERSION_MAX=8` 常量；`sand-harness::scenario` 加 serde 缺省 1 与
  `validate_dispersion`（取值域 `[1, 8]`）；`data/materials.ron` 给 water 定 `dispersion: 5`
  （Noita 调研 `docs/reference/noita-deep-dive.md:242` 参考值），其余材质吃缺省 1。
  方向记忆不变量（2026-06-14 液面冻结修复语义）一字未动。

  **实际效果（本 Task 的目标，不是性能优化）**：水面摊平从 **254 tick 降到 96 tick，2.6× 提速**
  （`rules_behavior.rs::higher_dispersion_levels_water_faster`，192×128 盆地判据 = 最高水面行降到
  y ≥ 118）。该测试写成"两个 dispersion 配置的相对比较"而非魔法 tick 数，改动前必然失败
  （`side()` 不读该字段 ⇒ 两次运行逐位相同，实测两边都是 254）。视觉结论留用户目检：
  `out/waterfall_disp{1,5}.gif`、`out/mixed_disp{1,5}.gif`（同参数改动前/后对照）。

  **验收五项**（spec §7.1）：① `cargo test --workspace` 153 项全绿、`cargo clippy --workspace
  --all-targets` 零警告；② golden 重录——**预期先落再验，实测完全兑现**：`sand_pile`（唯一不含
  water 的 golden）逐 tick 哈希与 final **逐位不变**，文件里只有 `materials_fp` 一行变；
  `mixed`/`waterfall_ci`/`explosion_ci` 状态哈希全变（语义变更，预期内）。取证手段是重录**前**
  先跑 `hashrun --grid-only` 做 before/after diff（M1 那次同款程序）；③ SyncTest 零分叉——
  `waterfall` 与 `mixed` **各 2 万 tick 六配置**（1/8 线程 × Full/ChunkSleep/LiveRect），
  分别 598s / 175s；④ bench 无回退，见下条；⑤ GIF 目检**结论待用户**。

  **两条实施期发现**（均已落文档）：
  - **`dispersion` 不适用 `blast_cost` 的"core 侧不校验"先例**——已立为总纲 §11 实施期决策第 2 条。
    取值直接决定 `WriteWindow` 读写半径的字段，越界后果是 release 下同相邻 chunk 数据竞争 →
    SyncTest 分叉，即**破坏 P4 写域论证本身**，不是"手感不对"。故两道防线：I/O 层给可读报错，
    core 在使用点无条件 clamp（`side()` 的 `.min(DISPERSION_MAX)`）。回归测试
    `water_dispersion_is_clamped_to_max_inside_core` 刻意绕过 harness 直接构表来打这道防线。
  - **`window.rs:14` 的"M0 实际移动半径 = 1"注释被本改动写失效**，顺手把 r 契约从人肉纪律
    升级为编译期断言（`MAX_WRITE_RADIUS = DISPERSION_MAX + 1 <= HALO`，现值 9 ≤ 16，余量 7）。
    这是把 spec §5 计划在 Task 2 做的事先做了色散那一半——Task 2 引入格内移速后按 spec §5
    扩写为完整不等式。**属超出 spec §3.3「Task 1 不碰 window.rs」的范围**，理由是留着失效注释
    + 无保护的半径常量跨越整个 Task 2 周期不值当；零行为变更（golden 与 SyncTest 已证）。

  受影响文件：`crates/sand-core/src/{rules,material,window,lib}.rs`、
  `crates/sand-core/tests/{rules_behavior.rs,common/mod.rs}`、`crates/sand-harness/src/scenario.rs`、
  `crates/sand-harness/tests/golden/*.golden`（4 个全部重录）、`data/materials.ron`、
  总纲 §4 + §11、`docs/README.md`。（`dda/emit/explode/particle.rs` 只是测试构造器补字段。）

- **Layer G Task 1 性能对照文档**：`docs/perf/2026-08-31-layer-g-task1-dispersion.md`。
  **无回退，大场景显著变快**：`acceptance`（640×384，最接近总纲 §7 目标量级）LiveRect 下
  1/8/16 线程一致提速 27%/55%/48%，ChunkSleep 下 −25%..+3.5%；`sparse` 基本持平；
  `mixed`（256×192）8/16 线程 +2.6%..+16.8% 的**真实小幅劣化**（不粉饰为噪声：1500 tick 里
  0–900 全是浇注段，`side()` 的 5 格探测成本收不回来，绝对值 ≤ 0.05ms/tick）。机制是
  "水更快摊平入睡 ⇒ 稳态活跃 cell 少" 与 "浇注段每 cell 探测成本涨" 的相对大小之争。
  **口径升级**：本次用 `git worktree` 在 `HEAD` 另建工作树独立编译 before 侧二进制、
  交替执行，是同机同轮次对照，比跨天比对基线文档表格可信；16 线程列信噪比最差
  （本机常态倒挂，M0 基线文档已记录），不单独下结论。**注意**：这不是加速优化的成果而是
  语义变更的副产品，Task 2 速度积分预期净增成本，别把这里的余量当可继承。

### Changed
- **M1 验收闭环：GIF 目检经用户确认通过**（验收 §0 第 4 项，此前结论留用户；确认版本为爆炸手感收口后 `66cea0a`..`33ab3da`）。M1 至此仅剩双机 hashrun（跨机项，移交下一会话首项，M0/M1 场景一起跑）。`docs/sessions/2026-08-30-m1-particle-layer.md` 验收表、`docs/README.md` 优先队列同步（下一步二选一待用户裁决：Layer G 速度积分提案 vs M2 场层与反应表）。

### Proposed
- **Layer G spec 评审修订两处（Fable 5 交叉评审，源码锚点逐一核验后用户裁决落笔）**：
  `docs/superpowers/specs/2026-08-31-layer-g-velocity-design.md`（Status 仍为 Proposed）。
  ① **溅射两骰（触发 + 抖动）的 RNG key 从撞停坐标改为起始坐标**（§6.1③/§6.3，决策记录
  #10）：撞停坐标同 tick 不唯一——cell A 撞停脱格后原格变 AIR，上方 cell B 同 tick 落入
  同格再撞停，key 用撞停坐标则两骰同值 → 同材质整列连锁全脱或全停，正是总纲 §11 翻案 4
  点名的偏置；起始坐标每 tick 每 cell 唯一（frac_roll §4.2③ 同一论证）。② **`side()` 循环
  上界 core 侧 clamp 到 `DISPERSION_MAX`**（§3.1/§3.2，决策记录 #11）：`dispersion` 越界
  会写出 WriteWindow（debug 撞 `window.rs:105` 断言，release 变同相数据竞争 → SyncTest
  分叉），属破坏 P4 写域论证的字段而非 `blast_cost` 式手感旋钮，不能只靠 I/O 层校验。
  另将"`MovedSide` 撞停也触发溅射"从隐含行为提为显式暂定裁决（决策记录 #12，横流水花
  是否过量待 Task 3 目检终裁），§3.4/§6.7 测试清单与 §7.1 目检清单同步补条目。
- **Layer G 运动语义重做 spec（M1 遗留债立项，brainstorming 完成，用户裁决 2026-08-31）**：
  `docs/superpowers/specs/2026-08-31-layer-g-velocity-design.md`（Status: Proposed）。范围
  = 液体色散 ≤8 + 重力速度积分 + 撞击溅射脱格，**分三 Task 独立落地**（色散打头，各自重录
  golden + 跑 SyncTest + 目检；依据是 golden 哈希 diff 只有在单一语义变更下才可诊断，总纲
  §11 tick-583 教训）。关键裁决：① 采纳"逐步 step 循环 + Cell 位段存竖直速度"（jason.today /
  Noita `docs/reference/noita-deep-dive.md:168` 路线），否决 DDA 竖直冲刺（会改变已被 golden
  锁住的沙堆安息角）与材质常量落速（拿不到加速感）——采纳理由是**行为连续性**：`n = max(1, …)`
  使 `v=0` 时退化为今日语义，`G_ACCEL=0` 即可做逐位回归取证；② 脱格触发 = 外部冲量 +
  高速撞击溅射，重力 clamp 在 4 格/tick，自由落体本身不脱格，粒子数正比于撞击面宽度而非
  下落体积；③ Cell 位段总规划一次定死（bits 17–21 竖直速度 Q3.2、22 预留 O3 free-falling、
  23–31 留白）。两条实施期发现记入 spec：**速度无条件写回会毁掉 chunk 休眠**（静止堆体速度
  从 0 涨起 → 每 tick `set()` → `mark_dirty_around` → 整图永不入睡，M0 稀疏性能作废），
  故立"只在值变化时写回"为正确性红线并配休眠不变量测试；**色散与子步循环相乘会撑爆 HALO**
  （4×8=32 > 16），但现有 `liquid_step` 结构使色散走到即撞停 break，最坏水平位移收敛到
  `(n−1)+8 = 11`，加探测 ±1 = 12 ≤ 16，余量 4——该不等式固化为 `window.rs` 编译期
  `const _: () = assert!(...)`，取代"新增规则须自证半径"的人肉纪律。落地时须同步总纲 §11
  实施期决策（位段规划 + HALO 编译期契约 + 网格 pass 新增 `spawns` 写入源）与 §4 Layer G
  措辞。`docs/README.md` 优先队列同步。

## 2026-08-30

### Changed
- **爆炸手感迭代四项（用户目检裁决）+ `explosion_splash.ron` 场景收窄**：
  ① `EXPLODE_SPEED` 16→8（"粒子更重"）；② 新增密度冲量缩放
  `REF_BLAST_DENSITY=40`，出射速度按 `参考密度/材质密度` 缩放（`v∝1/密度`，
  沙锚定系数 1.0、水系数 2.5 受 `clamp_speed` 封顶）；③ 射线方向涨落——每
  条射线独立掷能量 ±25%、射程 −25%..0 两骰（`ray_fluct`，分母
  `EXPLODE_FLUCT_DIV=4`），`explode_attempt` 编码随之从"低 1 位骰子标号"
  扩为"低 2 位"（4 颗骰），与 `emit_attempt` 独立演化、互不牵连；④
  `data/materials.ron` 汽化阈值调整：`sand` 0.7→**0.95**（更耐炸，近心
  汽化圈显著收窄），`water` 维持 0.4。配合本轮手感目检，`explosion_splash.ron`
  的 `ticks` 从 20000 收窄到 **8000**、script 从 19 炮（`Every(from:500,
  until:19500,step:1000)`）收窄到 **8 炮**（`until:7500`），单次渲染/回放
  耗时大幅下降。新增单测 `explode_crater_is_not_perfectly_circular`/
  `blast_mass_factor_golden_values`/`fire_ray_lighter_material_launches_faster_than_heavier`
  等锁定四项行为；4 个 golden 因 `materials.ron` 内容哈希变化重录
  ——`sand_pile`/`mixed`/`waterfall_ci` 三个无爆炸场景仅 `materials_fp` 行变、
  tick 哈希序列逐位不变（重录前 `cargo test` 失败输出 diff 实测确认），
  `explosion_ci` 因爆炸语义变更状态哈希全变（预期内）。`cargo test
  --workspace`、`cargo clippy --workspace --all-targets` 全绿。涉及：
  `crates/sand-core/src/world.rs`、`data/materials.ron`、
  `data/scenarios/explosion_splash.ron`、
  `crates/sand-harness/tests/golden/*.golden`、
  `docs/superpowers/specs/2026-08-30-m1-particle-layer-design.md`
  §6.2（新增）/§10/§13 决策记录第 8 条。

- **`world.rs` 拆分为 `explode.rs`/`emit.rs`（纯搬移重构，M1 收口）**：
  `world.rs` 原扛"网格存储 + Op::Emit + Op::Explode"三职责、约 1300 行；
  新建 `crates/sand-core/src/explode.rs`（`circle_offsets`/`fire_ray`/
  `ray_fluct`/`explode_attempt`、`EXPLODE_*` 全部常量、`REF_BLAST_DENSITY`
  及其全部测试）与 `crates/sand-core/src/emit.rs`（`emit_salt`/
  `emit_attempt`/`emit_jitter`、`EMIT_ROLL_*`、`MAX_EMIT_JITTER_RAW` 及其
  全部测试）；`World::apply_op` 留在 `world.rs`，`Op::Explode`/`Op::Emit`
  分支体抽为 `explode::apply_explode(...)`/`emit::apply_emit(...)` 调用。
  `emit_jitter` 声明 `pub(crate)` 供 `explode.rs` 复用（同一套抖动数学）；
  `World::vaporized_total` 字段收紧为 `pub(crate)`（`explode::fire_ray`
  跨模块直接自增）。**一行逻辑未改**——纯度证据：拆分前（`66cea0a`）后
  `cargo test --workspace` 全绿（sand-core 单测仍 102 项，逐条搬到对应
  模块，无一丢失/新增逻辑分支）且拆分前重录的 4 个 golden 一个不红（=
  逐位零扰动）；`cargo clippy --workspace --all-targets` 零警告。涉及：
  `crates/sand-core/src/{world,explode,emit,lib,material}.rs`、
  `docs/superpowers/specs/2026-08-30-m1-particle-layer-design.md` §6.2
  文件引用同步更新。

### Fixed
- **GIF 帧延迟与采样率解耦**（用户目检发现帧率异常）：`render.rs` 原公式 delay = every×100/60（墙钟等速），`--every 100` 时 1.67 秒/帧成幻灯片——M1 验收两张 GIF 即中招。默认仍等速但 clamp [2,10]cs（稀疏采样自动转延时摄影），新增 `--fps` 显式覆盖回放帧率；3 项 `frame_delay` 金值单测；`out/waterfall.gif`、`out/explosion_splash.gif` 已重渲（10cs/帧 ≈ 20 秒）。`crates/sand-harness/src/{render,main}.rs`，commit `d982c70`。

### Added
- **爆炸近心汽化 `vaporize_threshold`（用户裁决 2026-08-30）**：严格质量守恒
  （每格必生成一颗粒子）在爆心附近观感不对——"近心没了、外圈飞溅"改由每
  材质新增 `vaporize_threshold` 字段实现：射线剩余能量比例（与既有速度衰减
  公式同一口径的 `remaining`）**严格超过**该阈值即汽化（置 air、不入生成
  队列、质量确定性蒸发），否则照旧摧毁+溅射。数据：RON 写 `0.0..=1.0`
  十进制，缺省 `1.0`（永不汽化），加载期经 `quantize_vaporize_threshold`
  一次性 `×255 round` 量化为 `u8`（负值/超界报错，仿 `quantize_fx` 先例）；
  `data/materials.ron` 初值 `water 0.4`（量化 102）、`sand 0.7`（量化
  179）、`air`/`wall` 吃缺省。诊断计数 `World::vaporized_total`（私有字段 +
  访问器，仿 `Particles::rejected_total`/`buried_total` 先例）不参与
  `hash::state_hash`，不影响 SyncTest。挖坑守恒断言口径同步改为"摧毁格数
  == 生成粒子数 + 汽化计数"；新增沙水混合目标测试锁定 water 汽化比例高于
  sand（阈值更低）。golden 影响：`materials.ron` 内容哈希变 → 4 个 golden
  的 `materials_fp` 行全部重录；`explosion_ci` 额外因爆炸语义变更导致状态
  哈希全变（重录），`sand_pile`/`mixed`/`waterfall_ci` 无爆炸，重录前 diff
  确认状态哈希逐位不变（改动完全隔离在爆炸路径内）。测试新增 10 项（core
  5 + harness 5），`cargo test --workspace` 137 项全绿，
  `cargo clippy --workspace --all-targets` 无警告。涉及：
  `crates/sand-core/src/{material,world}.rs`、
  `crates/sand-harness/src/scenario.rs`、`data/materials.ron`、
  `crates/sand-harness/tests/golden/*.golden`、
  `docs/superpowers/specs/2026-08-30-m1-particle-layer-design.md` §6.1/§13
  决策记录第 7 条。

- **`explosion_splash.ron` 沙丘几何裁决（用户目检三轮，2026-08-30）**：M1
  验收爆炸溅射场景的目标几何从"厚沙板"迭代到"沙丘"——①厚沙板深埋爆点
  （`y=220`/`r=40`）导致粒子被自己炸出的坑壁困死，无侧向飞溅；②抬到板面
  （`y=155`）后只有正上方 ±40° 锥角能从"烟囱口"逃逸，侧向/斜上仍撞
  440 宽厚板的墙；③改阶梯沙丘、爆点置于丘顶，侧向射线约 30 格即出丘壁，
  粒子带约 90% 速度进入开阔空气、抛物线落两侧水池——达成"火山口喷发"
  画面。裁决史记入场景文件注释，此前未落 CHANGELOG，本次补记。
  `data/scenarios/explosion_splash.ron`。

- **M1 粒子层（Layer P）实施完成，spec → Implemented**（`docs/superpowers/specs/2026-08-30-m1-particle-layer-design.md`，
  Task 1–7 全部完成，验收标准 §0 五项全过）。跨 Task 1–6 的完整产出：`crates/sand-core/src/fixed.rs`
  （手写 Q16.16 定点：add/sub/neg/mul/mul_int/from_ratio/to_cell/isqrt，全部配金值单测）、
  `particle.rs`（SoA `Particles`：x/y/vx/vy/material + `next_id`/`rejected_total`/`buried_total`，
  spawn 顺序即 id 序、65536 容量确定性拒绝、保序 `compact`、并行 `integrate` + 串行按 id `commit`）、
  `dda.rs`（`CellWalk` 迭代器：i64 交叉相乘整数网格穿越，供粒子飞行路径与爆炸射线复用同一算法）、
  `world.rs` 新增 `Op::Emit`（发射器，harness 场景 `Every` script 驱动）与 `Op::Explode`（Noita 射线模型：
  Bresenham 圆周逐条 DDA 射线、`blast_cost` 逐格消耗能量、遮挡免费涌现）、`rng.rs` 新增
  `STREAM_EMIT`/`STREAM_EXPLODE`、`hash.rs` 新增粒子层哈希折叠 + `Sim::grid_hash()`/`state_hash()` 分离。
  harness 侧：`scenario.rs` 新增 `OpSpec::Emit`/`Explode` + `quantize_fx`（I/O 层唯一浮点量化点）+
  场景指纹 `combine(源字节哈希, 已解析 Fx 字段折叠)`；`--grid-only` 开关支撑 spec §9 两步 golden
  重录程序。数据：`data/scenarios/{waterfall,waterfall_ci,explosion_ci,explosion_splash,particle_stress}.ron`，
  `materials.ron` 新增 `blast_cost` 字段（缺省 1，wall 用 `BLAST_COST_INFINITE` 哨兵）。测试从 M0 的
  22 项增长到 **122 项**（`cargo test --workspace` 全绿，逐 suite 核验：core 单测 91 + `particle_behavior`
  3 + `rules_behavior` 5 + `synctest_ci` 1 + harness 单测 16 + golden 4 + `synctest` 2 =
  91+3+5+1+16+4+2 = 122）；`cargo clippy --workspace --all-targets` 全程无警告。（修正 2026-08-30
  评审：此前误写"124 项"，系沿用 Task 6 报告的加总错误——`synctest_ci` 实为 1 项而非 3 项。）
  各任务分述详见 `.superpowers/sdd/2026-08-30-m1-particle-layer-plan/task-{1..6}-report.md`；本条汇总
  Task 7 验收前的实施全貌，Task 1/3/5/6 的独立 Added 条目见下方（保留原有颗粒度）。

  **实施期修复轮要点**（详见各任务 report 的"修复轮"节）：Task 4 评审 C1（Critical）——落格五邻格
  全占后原设计"重置为悬浮、速度清零"在静态堆场景构成**活锁**（40 颗同位同速沙粒复现，32 颗永久卡死，
  两 tick 一循环）；改判为向上兜底搜索（候选格正上方逐格找空位，搜到世界顶仍无空位则确定性出界销毁，
  `buried_total` 计数不入哈希），**完全废除"继续飞行/悬浮"分支**——`kernel-charter.md` §4/§11 已同步
  修正（见下方 Changed 条目）。Task 5 评审 I1（Important）——同帧同格多个 `Op::Emit`（或 setup 阶段与
  紧接的 tick 0 首个 `step()`）共享 `fseed` 导致 RNG key 撞车，新增 `emit_salt(op_idx, i)`/
  `emit_attempt(stamp, roll)` 折进 salt/attempt 高位区分（呼应总纲翻案记录第 4 条"同帧同格多次掷骰
  必须彼此不同"纪律）；I2——场景指纹从"仅源字节哈希"改为 `combine(源字节哈希, 已解析 Fx 字段折叠)`，
  堵住"源字节相同但跨平台解析出不同 Fx"的假设性分叉。

### Fixed
- **M1 终审修复波**（详见 `.superpowers/sdd/2026-08-30-m1-particle-layer-plan/final-fix-report.md`，
  仿真语义零改动，全部落在 harness 校验/注释/文档）：① `crates/sand-harness/src/scenario.rs`
  `resolve_op` 的 `Op::Explode` 分支补加载期范围校验——`x`/`y` 需 `|v| < 32768`（`Fx::from_int`
  安全域）、`r ∈ [1, 32767]`、`power ∈ [1, i32::MAX as u32]`，越界 `Err`（此前纯透传，越界会
  静默腐化为 wrapping/翻号），新增 2 条拒绝测试；② `crates/sand-core/tests/common/mod.rs` 订正
  失实注释（`explode_behavior.rs` 不存在，测试实际内联在 `world.rs`）；③ `crates/sand-core/src/world.rs`
  `Op::Explode` 应用处补注释：格子已清 air 后若 spawn 队列 drain 被 `MAX_PARTICLES` 拒绝，该质量
  永久丢失（计入 `rejected_total`），非 bug 但需知悉；④ `docs/superpowers/specs/2026-08-30-m1-particle-layer-design.md`
  §4 补充实际执行位置：粒子相在 `Sim::step` 里位于 `scheduler::step`（网格四相 + tick 自增 +
  脏矩形交换）**之后**执行，落格唤醒经 `next_dirty` 于下一 tick 生效——既定语义，SyncTest 已覆盖，
  供 M2 插入场层时参考。`cargo test --workspace` 122→**124** 项全绿，`cargo clippy --workspace
  --all-targets` 零警告，既有 golden/SyncTest 零回归。

- **M1 Task 7 完成：验收与收尾**。SyncTest 验收（release，`--threads 8`，六配置 = {1,8}线程 ×
  {Full,ChunkSleep,LiveRect}）：`waterfall.ron` 2 万 tick 零分叉（`scenario_fp 39575dfa5dfed750`，
  实跑 577.8s）；`explosion_splash.ron` 2 万 tick 零分叉（`scenario_fp f229c61b5deb0328`，实跑
  856.1s）——spec §0 验收标准第 2 条完成。bench 对照（`docs/perf/2026-08-30-m1-particle-baseline.md`，
  同 Task 1 口径复测）：mixed/sparse 网格路径在 ±10% 内持平或更快；acceptance 1 线程组合超出 ±10%
  阈值（+14.8%/+15.6%），判定为共享服务器噪声（基线文档已记录同组 3 次波动可达 20–30%），非回归，
  已如实记录待后续复核。`particle_stress.ron` 压测：稳态粒子数 = 容量硬上限 `65536`（33 tick 内触顶）；
  用容量爬升期"零网格落地"窗口做总耗时差分（未改动 `sand-core`），直接测得 2 万粒子量级下粒子相
  开销 **0.586ms/tick**，另用 65536 粒子恒定窗口折算得 **0.504ms/tick**（20k 等效），均低于总纲 §7
  预算"2 万粒子 ≈ 0.8ms"，预算校准通过。render GIF 目检（`crates/sand-harness/src/render.rs` 新增
  `draw_particles`——只读叠加粒子层单像素点，不影响任何哈希/模拟语义）：`out/waterfall.gif`、
  `out/explosion_splash.gif`（均不入库，`.gitignore` 已排除 `/out`）；目检要点见
  `docs/sessions/2026-08-30-m1-particle-layer.md`"验收状态"节，结论留给用户复核。文档四件套：
  `docs/overview/kernel-charter.md` §11 新增"实施期决策（2026-08-30，M1）"小节第 1 条（M1 粒子相
  插入 tick 管线第 5 步 + Layer P 落格语义修正）、`docs/CHANGELOG.md`（本条）、
  `docs/sessions/2026-08-30-m1-particle-layer.md`（新增）、
  spec Status → Implemented、`docs/README.md` 优先队列更新为"M1 完成，下一项 Layer G 速度提案 / M2"。

### Changed
- **`docs/overview/kernel-charter.md` §4 Layer P 落格语义措辞修正 + §11 新增"实施期决策"小节**
  （2026-08-30）：原文"输家按定序邻格搜索或继续飞行"中的"继续飞行"（悬浮）分支已改写为
  "定序邻格（上/左/右/左上/右上）搜索空位；五邻格全占则沿候选格正上方继续向上搜索最近空格，搜到
  世界顶仍无空位则确定性出界销毁——不存在'继续飞行/悬浮'分支"，与 M1 spec §5（决策记录第 6 条）
  实现口径对齐。§11 在既有"翻案记录"列表之后新立小节标题"实施期决策（2026-08-30，M1）"（翻案记录
  列表标题与既有 1–5 条原样不动），第 1 条记：M1 粒子相插入 tick 管线第 5 步
  （`program-architecture.md` 自立宪起已把该步骤排在管线第 5 位，M1 实施是让其从占位转为真实生效，
  仍属协议版本变更）+ 上述落格语义修正的依据（Task 4 评审 C1 活锁实证）。**归档位置修正
  （2026-08-30 评审 I2）**：该条最初误放进"翻案记录"编号列表（记作第 6 条）——它记的是实施期对
  本文措辞的补记，不是"推翻既有裁决"，分类失准，已移出并单独立节，翻案记录列表标题与条目内容
  未改动。

### Added
- **M1 Task 6 完成：`Op::Explode` Noita 射线模型 + 爆炸/压测场景**（spec §6/§10）。
  `crates/sand-core/src/material.rs`：`MaterialDef`/`MaterialTable` 新增
  `blast_cost: u32`（+ `BLAST_COST_INFINITE` 哨兵），`data/materials.ron` 全部材料
  显式赋值（air 0 / water 1 / sand 2 / wall 4294967295）。`world.rs`：`Op::Explode
  { x, y, r, power }`（整数签名）；`circle_offsets`（经典 Bresenham 圆，`d=3-2r`
  决策变量，八分圆镜像展开去重用长度 ≤8 的线性 `contains` 扫描——"相邻+首尾折返"
  式去重对轴上退化点是错的，实现前已发现并修正）；`fire_ray` 逐格消耗
  `blast_cost`，能量 ≥ cost 摧毁（置 air + 溅射，速度=方向单位向量×
  `MAX_SPEED×剩余能量/power`+`emit_jitter` 抖动，`clamp_speed` 收尾），能量耗尽或
  撞 `BLAST_COST_INFINITE` 材料断线；爆心格按"该射线第一格"计费（r≥1 必炸爆心）。
  `dda.rs` 抽取公共 `CellWalk` 迭代器（`trace()` 内部算法原样搬入，纯重构、23 项
  既有测试不变），供爆炸射线复用同一套穿越算法。`rng.rs` 新增
  `STREAM_EXPLODE=2`（`(x,y)` 用被摧毁格坐标本身，`salt=op_idx` 区分同 tick 多个
  Explode——Task 5 评审 I1 同款纪律）。`particle.rs::clamp_speed` 收紧为
  `pub(crate)` 供 world.rs 复用。harness `scenario.rs`：`MatSpec.blast_cost`
  （serde 缺省 1）、`OpSpec::Explode`（纯整数透传，无需量化）。新场景
  `data/scenarios/explosion_ci.ron`（256×192，1200 tick，并入
  `tests/golden/explosion_ci.golden` + `tests/synctest.rs` 六配置 CI）、
  `explosion_splash.ron`（640×384，20000 tick 验收版，同 waterfall.ron 先例不进
  CI）、`particle_stress.ron`（640×384，3000 tick，持续 2000/tick Emit 顶满
  65536 容量，bench 专用不进 CI/golden）。测试：core 新增 21 项（`CellWalk` 4 +
  `circle_offsets` 5 + `fire_ray` 白盒 3 + `Op::Explode` 行为 8 + 金值 1）+ harness
  新增 2 项 + golden 1 项 + synctest 1 项，`cargo test --workspace`（122 项，修正
  2026-08-30 评审：此前误写"124 项"，加总错误——`synctest_ci` 实为 1 项而非 3 项）与
  `cargo clippy --workspace --all-targets` 全绿。**golden 重录**：既有
  `sand_pile`/`mixed`/`waterfall_ci` 三个 golden 的 tick 周期哈希与终态哈希
  **逐位不变**（`git diff` 确认三个文件都只有 `materials_fp` 一行变化），证明
  `blast_cost` 字段新增是非语义变更；用 `sand-harness replay --write-golden`
  按既有口径（4 线程 LiveRect）重录。详见
  `.superpowers/sdd/2026-08-30-m1-particle-layer-plan/task-6-report.md`。

### Added
- **M1 Task 5 完成：`Op::Emit` 发射器 + 瀑布场景**（spec §7/§8）。`crates/sand-core/src/world.rs`：`Op::Emit { material, x, y, vx, vy, count, jitter }`（坐标/速度 `Fx`）；`World::apply_op` 签名新增 `fseed`/`spawns: &mut Vec<SpawnRequest>` 出参（`SpawnRequest` 从 `lib.rs` 移到此处，`pub(crate)`），Emit 分支逐粒子用 `rng_u32(fseed, STREAM_EMIT, 发射点格x, 发射点格y, salt=i, attempt)` 掷两骰（`attempt=0` vx / `attempt=1` vy，挪用 attempt 位区分"同 salt 下第几骰"而非其原始重试语义，注释显式记录）→ `emit_jitter` 整数映射到 `[-jitter,+jitter]`（无除法，`(r as u64 * width) >> 32` 重缩放）。`apply_op`/`scheduler::step` 因新增 `pub(crate)` 型参数一并收紧到 `pub(crate)`（原 `pub` 会产生私有类型泄漏警告，且从无 crate 外调用方）。`rng.rs` 新增 `STREAM_EMIT=1`。`Sim::apply_setup`/`Sim::step` 改为把 `Op::Emit` 产出的生成请求并入既有 `spawn_queue`，与 `queue_spawn` 走同一入队序；fseed 计算在 `scheduler::step` 内挪到 ops 循环之前（纯函数提前算，不改变可观测的三步顺序，非协议变更）。harness 侧：`crates/sand-harness/src/scenario.rs` 新增 `OpSpec::Emit`（RON 十进制小数）+ `quantize_fx`（round 量化，I/O 层浮点，唯一允许出现的位置）+ `resolve_op` 校验 jitter 非负；场景指纹 `xxh3_64(原始文件字节)` 天然覆盖 Emit 参数变化（新增测试钉死该保证；**该指纹口径随后在修复轮 1 改为 combine(源字节哈希, 已解析 Fx 字段折叠)，见下条**）。新场景 `data/scenarios/waterfall.ron`（640×384，20000 tick，验收用）与 `data/scenarios/waterfall_ci.ron`（256×192，1200 tick，并入 `crates/sand-harness/tests/synctest.rs` 六配置 CI SyncTest + `tests/golden/waterfall_ci.golden` 入库）。测试：核心新增 6 项单测（`emit_jitter` 金值/边界 + salt-attempt 独立性 + 确定性重跑，lib 单测总数 66 项）+ harness 新增 7 项（quantize_fx 金值/`resolve_op`/指纹敏感性，lib 单测总数 7 项）+ 1 项 harness CI SyncTest + 1 项 golden；`cargo test --workspace` 与 `cargo clippy --workspace --all-targets` 全绿；既有 3 个 golden（`sand_pile`/`mixed`）与 CI SyncTest（六配置 6000 tick）哈希不受影响。详见 `.superpowers/sdd/2026-08-30-m1-particle-layer-plan/task-5-report.md`。

### Fixed
- **M1 Task 5 评审修复轮 1：2 Important + 5 Minor**（详见 `.superpowers/sdd/2026-08-30-m1-particle-layer-plan/task-5-report.md` "修复轮 1"节）。**I1**（同帧同格多 Emit 撞 key）：`crates/sand-core/src/world.rs` 新增 `emit_salt(op_idx, i)`（`op_idx` = 本 tick `ops` 切片下标，折进 salt 高 16 位，区分同 tick 内多个 `Op::Emit`）与 `emit_attempt(stamp, roll)`（`stamp` 折进高位，额外区分 `Sim::apply_setup` 与紧接的 tick 0 首个 `step()`——两者共享同一 `fseed`）；`World::apply_op`/`scheduler::step`/`Sim::apply_setup` 均改为对 `ops` 切片 `enumerate()` 传入 `op_idx`。**注意**：该修复改变了 `Op::Emit` 实际消费的 RNG 序列（`attempt` 位模式变化），凡使用 Emit 的场景物理结果随之改变——`waterfall_ci.golden` 全部哈希值（非仅指纹）已重录；`sand_pile`/`mixed`（无 Emit）逐 tick 哈希验证位级不变，只有指纹行因 I2 而变。**I2**（指纹口径）：`crates/sand-harness/src/scenario.rs` 的 `Scenario::fingerprint` 从"仅源字节哈希"改为 `combine(源字节哈希, fold_fx_fields(全部已解析 Op::Emit 的 Fx 字段))`，满足 spec §7"量化后数值入指纹"，堵住"源字节相同但解析出的 Fx 不同"这一类假设性跨平台分叉；测试从字符串哈希同义反复改为写临时 RON 走真实 `load_scenario` 断言指纹随 vx 改变而改变。**Minor**：`emit_jitter` 算术改显式 `wrapping_*`；新增 `MAX_EMIT_JITTER_RAW = (1<<30)-1`（`world.rs`，`pub` 导出）防定点重缩放溢出 i32 静默 wrapping，`emit_jitter` 内 `debug_assert` + `quantize_fx`/`resolve_op` 加载期同一常量校验双重防线；`emit_jitter` doc comment 补乘移法残余非均匀性说明；`quantize_fx` 改 `Result`，非有限数或量化后越过 `i32` 边界报错（不再默默 wrapping）；报告措辞"等比例缩小"改"几何同构"。测试：lib 单测总数从 core 66/harness 7 增至 core 71/harness 14（新增 `emit_op_idx_differentiates_*`/`emit_attempt_differentiates_*`/`emit_jitter_*_bound_*`/`fold_fx_fields_*`/`quantize_fx_rejects_*`/`resolve_op_*jitter*` 等回归测试）；`waterfall_ci.golden` 重录（哈希值变化，语义未变——见上方"注意"）；`cargo test --workspace` + `cargo clippy --workspace --all-targets` 全绿。

### Added
- **M1 Task 3 完成：粒子池 SoA + 状态哈希并入 + golden 重录**（spec §3/§9）。`crates/sand-core/src/particle.rs` 新增 `Particles`（x/y/vx/vy: Vec<Fx>、material: Vec<u8>、`next_id`/`rejected_total`），`spawn` 顺序即遍历序、容量满（65536）确定性拒绝、`compact` 保序压缩、`hash_into` 按下标序折叠字段位+`next_id`+粒子数。`lib.rs`：`Sim` 挂 `particles` + `spawn_queue`，`step()` 新增粒子相骨架（drain 生成队列 + 占位式全 keep compact，运动留 Task 4）；`Sim::grid_hash()`（网格哈希树根单独导出）与 `Sim::state_hash()`（= `hash::combine(grid_hash, particles.hash_into())`，`hash.rs` 新函数）。golden 重录按 spec §9 两步程序：harness 新增 `--grid-only` 开关（`sand-harness hashrun <scenario> --grid-only`），改动前后网格层逐 tick 哈希序列 diff 为空（sand_pile.ron、mixed.ron 各 2 组，取证存 `.superpowers/sdd/2026-08-30-m1-particle-layer-plan/task-3-grid-hash-before.txt`），证明 Layer G 零扰动后用 `--write-golden` 重录 `crates/sand-harness/tests/golden/{sand_pile,mixed}.golden` 终态。测试：33 项核心单测（含 5 项新增粒子测试）+ 5 项行为测试 + CI SyncTest（六配置 6000 tick）+ 2 项 golden 全绿；clippy 全绿。详见 `.superpowers/sdd/2026-08-30-m1-particle-layer-plan/task-3-report.md`。

### Proposed
- `docs/superpowers/specs/2026-08-30-m1-particle-layer-design.md`：M1 粒子层实现级设计（brainstorming 全节口头批准，待用户过目 spec）。要点：手写 Q16.16 定点（用户裁决维持总纲 §6，浮点四雷区论证入 spec §2）、SoA 顺序即 id 序、并行积分 + 串行按 id 提交、DDA 阻挡一视同仁、**爆炸采 Noita 射线模型**（wiki 查证：ray energy 逐格消耗 + durability 门槛，遮挡免费涌现——替代圆盘扫描）、发射器 = script Every + Op::Emit 零新概念、哈希格式变更 golden 两步重录程序、动工前落 `docs/perf/` 正式基线。

### Changed
- **M1 粒子层范围裁决（用户批准，brainstorming 进行中）**：M1 走**最小闭环**——粒子来源 = 场景发射器（瀑布）+ 爆炸 Op（半径内网格 cell 轰成带外向速度的粒子，即真实脱格路径），Layer G 语义零改动、golden 不作废；DDA / 串行按 id 落格 / 64k 容量限流照总纲。**Layer G 速度积分**（格内移速 ≤4、超限自然脱格——总纲 §4 Layer G/P 衔接的原文语义）**后置为 M1 之后的独立提案**：它直接顶在 r≤16 并行安全论证上，须单独立项、过 §11、跑 SyncTest；此为分期实施而非翻案。依据：Noita 实为"网格速度积分 + 脱格粒子"双系统（官方 32px/帧上限锚点 `docs/reference/noita-deep-dive.md:200`、GDC 原话同文 208-210），网格速度是迟早要还的债，记账不弃账。O3 粉末惯性同理不入 M1（一次只动一个语义层，M0 tick-583 教训）。M1 动工前先落 `docs/perf/` 正式 bench 基线。`docs/README.md` 优先队列同步。

### Added
- `docs/perf/2026-08-30-m0-rust-baseline.md`：M1 动工前正式性能基线（Task 1，纯测量不改代码，commit `5653be6`）。取代 `docs/perf/2026-08-30-m0-rust-informal.md`（已加取代指针）。口径：`data/scenarios/{mixed,acceptance,sparse}.ron`，{1,8,16} 线程 × {ChunkSleep,LiveRect} 六组合各测 3 次取中位。执行期纠偏：原计划用 acceptance 20000→100000 tick 差分隔离"睡眠常态"单组耗时 ~60s、总预算超支被叫停，改为**短程稳态口径**——acceptance 截断至 5000 tick（非全量）、睡眠常态改用 mixed 场景 900→1500 tick 尾段差分（600 tick 窗口）；与 informal 文档长程口径不可直接对比。要点：LiveRect 在全部测得组合中不劣于 ChunkSleep（sparse 1T 收益 4.3×），16 线程在现有小场景规模下有时慢于 8 线程（判定为场景太小、NUMA 调度开销盖过并行收益，非回归）；差分法测睡眠常态信噪比有限，9000/11000 窗口口径曾测出物理不可能的负值（已弃用，留痕于 `.superpowers/sdd/2026-08-30-m1-particle-layer-plan/task-1-report.md`）。原始三次数据与全部异常观察见同一份 report。
- **O1 chunk 内活矩形实施完成**（spec `docs/superpowers/specs/2026-08-30-o1-live-rect-design.md` → Implemented）：`ScanMode { Full, ChunkSleep, LiveRect }` 替换 `sleep_skip`（`crates/sand-core/src/lib.rs`）；单代码路径动态边界扫描（`rules.rs::update_chunk`——起始矩形参数化，Full/ChunkSleep 传 FULL 时扩张自然无效）；WriteWindow 任务本地活矩形追踪（`window.rs`）；调度起始矩形 = dirty ∪ next_dirty 快照（`scheduler.rs`）。等价性论证核心 = 以全扫访问序定义"前方"，过度包含永远安全（spec §1）。**双证**：既有 golden 用 LiveRect 重放哈希一字不变 + SyncTest 升级六配置（{1,N 线程}×{Full,ChunkSleep,LiveRect}）全绿。收益（`docs/perf/` O1 节）：稀疏 2.7×、worst 1.2×、睡眠持平。新增 `data/scenarios/sparse.ron` bench 场景与 harness `--scan full|sleep|live` 开关。
- **M0 骨架与执法实施完成**（spec `2026-08-29-m0-skeleton-design.md` → Implemented）：
  - `crates/sand-core`：Cell u32 位段（`cell.rs`）、chunk/脏矩形原子合并（`chunk.rs`）、chunk 寻址世界 + brush/fill 共用写入路径（`world.rs`）、SquirrelNoise5 RNG **与 Python 版金值交叉锚定**（`rng.rs`）、WriteWindow unsafe 窗口 + debug 写域断言（`window.rs`）、数据驱动沙/水规则含方向承诺不变量（`rules.rs`）、四相 rayon 调度器（`scheduler.rs`）、xxh3 哈希树叶层（`hash.rs`）、Sim 门面（`lib.rs`）。
  - `crates/sand-harness`：RON 场景/材料加载 + xxh3 指纹（P5）、synctest/replay/hashrun/render 四子命令、golden 回归 ×2 入库、GIF 占位渲染器（消费 `Sim::world()` 只读视图 = Channel A 雏形）。
  - `data/materials.ron` + 三场景（sand_pile / mixed / acceptance 640×384）。
  - 测试 22 项全绿：单测（位段/RNG 金值/相几何互斥穷举/哈希/写域 should_panic）+ 行为 5 项（沙落/堆守恒/沉水/跨缝摊平/方向承诺）+ CI SyncTest（256×192×6000 tick×四配置）+ golden ×2。
  - 验收：release 版 640×384 × 10 万 tick × 四配置 SyncTest 零分叉；GIF 目检通过（安息角/沙沉水/液面摊平）。双机 hashrun 待用户执行。
  - 性能参考（release，640×384 活跃期）：见 synctest 日志；正式 bench 与 `docs/perf/` Rust 基线留 M1 后。

### Added
- `docs/perf/2026-08-30-m0-rust-informal.md`：M0 非正式性能测量（Xeon 6330 服务器 CPU，provisional）。要点：640×384 全活跃最坏 ~3ms@8T、1280×768 ~8ms@8T（>总纲 4ms 目标但在帧预算内；§7 单线程估算实测偏乐观 3×）、睡眠常态 0.066ms、1080p 全图崩塌 9.6ms@16T 仍可 60Hz；地图上限的真实约束 = 同时活跃面积（本机 ~百万 cell ≈ 8–10ms）而非驻留面积；与 Noita 对照表（调度同构、规则复杂度暂不可比、Layer F 落地后大图结论需重估）。

### Proposed
- `docs/proposals/2026-08-30-noita-derived-optimizations.md`：Noita 对照四项优化入档——O1 chunk 内活矩形（M1 门口；含与全扫逐位等价的论证，区别于被否决的冻结矩形）、O2 Layer F 低分辨率场格+半频（M2 设计期裁决）、O3 粉末惯性 is_free_falling/inertial_resistance（M1 可选）、O4 运行时周期哈希口径（M5）；另记录两项明确不采纳（reality bubble 违反 P1、非确定随机跳过违反 D2）。docs/README 优先队列同步。

### Changed
- **总纲 §11 翻案 5：里程碑验收 10 万 tick → 2 万 tick 密集场景**（用户裁决）。依据：架构内无 10 万独有效应（RNG 无状态、睡眠为无状态等待、世代戳 6k 已绕 23 圈、结构性分叉在触发配置后数百 tick 内暴露——tick 583 实证）；10 万 soak 降级为 M5/M6 发布门过夜测试。`data/scenarios/acceptance.ron` 改为 20000 tick（唤醒波挪至 12k/16k，事件覆盖不变），四配置 synctest 从 ~50 分钟降至 ~10 分钟。spec §0 验收节同步。M0 已按原 10 万标准过关，本改动自 M1 起生效。

### Fixed
- **spec §1.4 实施期修订：cell 级冻结脏矩形被 SyncTest 当场击落**——单缓冲扫描允许 tick 内链式移动（整段静止水沿扫描方向一 tick 整体平移，链长无上界），tick 起点冻结的矩形切断链，与全扫语义分叉（实测 tick 583，256×192 场景）。修复：休眠粒度提升为 **chunk 级 + 相位边界唤醒**（重查 dirty ∪ next_dirty，屏障后原子合并结果调度无关），活跃 chunk 全量扫描；等价论证入 spec §1.4。`crates/sand-core/src/scheduler.rs`。这正是 SyncTest 作为常驻执法的第一次实战开张。

## 2026-08-29

### Changed
- **项目大转向（用户裁决）**：东方同人横版动作（Python 原型 → Godot C#）→ **1v1 落沙法术对战，Rust 内核 + Godot 4/gdext 表现层，全栈确定性 lockstep**。产品与内核约束由用户新写的两份文档定义（见 Added）。
- `CLAUDE.md` 全文重写：薄化为协作规范 + 确定性红线速查 §5，技术真源指向总纲/架构文档（消除旧版正文与补丁横幅漂移的模式）；命名约定换 Rust/RON/GDScript。
- **四项翻案落档**（总纲 §11 翻案记录）：①刚体入核心全 lockstep（推翻 R3-A 状态同步）；②温度场作为 Layer F 回归主线（推翻 2026-06-06 降级裁决）；③并行语义换四相棋盘 + r≤16（替代 M0.5 正方形写域）；④RNG key 收敛为 hash(tick,x,y,salt/stream)，pass_id/attempt 维度并入 stream 且实现时必须显式保留。对应旧文档加 Superseded 标注：`docs/overview/architecture.md`、`docs/proposals/2026-06-14-determinism-hardening-r1-r3.md`、`docs/proposals/2026-06-06-deterministic-parallel-and-netcode.md`、`docs/superpowers/specs/2026-05-26-fire-system-design.md`。

### Added
- `docs/overview/kernel-charter.md`：内核顶层设计总纲 v0.1（用户撰写，2026-08-29 采纳为项目宪法）——P1–P5 第一性原则、三层内核（四相 push 网格 + 稀疏粒子 + pull 场）、1v1 延迟制 lockstep、确定性纪律法典、里程碑 M0–M6、决策日志。
- `docs/overview/program-architecture.md`：程序架构文档 v0.1（用户撰写）——四环结构、crate 布局与单向依赖、Ring 0 子系统读写清单、规范 tick 管线（时序即契约）、跨层通信白名单。
- `docs/README.md`：docs 导航入口 + 当前优先队列（CLAUDE.md §3.1 规划已久，首次补建）。
- **Rust workspace 骨架**：`Cargo.toml`（workspace，edition 2024）+ `crates/sand-core`（Ring 0 纯库，暂只载铁律文档注释）+ `crates/sand-harness`（CLI stub）+ `clippy.toml` disallowed_types deny std HashMap/HashSet（charter §6 执法，**红绿验证**：临时违规代码确认 clippy 报错后移除）。`cargo clippy`/`cargo test` 全绿。commit `453eec0`。

### Proposed
- `docs/superpowers/specs/2026-08-29-m0-skeleton-design.md`：M0 实现级设计（用户批准六节设计 + 两裁决：M0 即上 rayon、水走简版横流；GIF 占位渲染器并入 M0）。要点：Cell u32（8 位世代戳替代总纲的 1-bit 奇偶位——自审抓到陈旧位撞车 bug，M0.5 决策①同款结论）、脏矩形原子 min/max 合并为相内唯一共享写、WriteWindow debug 写域断言、SyncTest 四配置（1/N 线程 × 跳过开关）、材料表走 RON + InitConfig 注入保持 Ring 0 零 I/O。

### Removed
- **Python 原型归档**：`prototype/` → `archive/prototype-python/`（只读史料 + README 定性：算法语义参考，不做一对一移植）。83 tests 与 M0/M0.5 成果封存，commit `f5f2371`。

## 2026-06-14

### Fixed
- **液体/气体"方向承诺"bug（day-one 缺陷，非 M0/M0.5 回归）**：`_move_liquid`/`_move_gas` 侧移走 `-vel` 方向时不翻转方向记忆，下帧先试 `+vel`（= 刚腾出的空格）→ 表面像素在两格间永久打乒乓，净输运为零，**液面冻结成沙堆形**（盆中水柱 6000 帧 profile 一字不变；单像素轨迹追踪铁证）。三版本（pre-M0 / M0 / M0.5）行为一致证明非回归。修复后水柱 600 帧摊平至 spread 1（修复前 13）。`prototype/core/rules.py`（侧移段方向承诺）；新增 3 测试（方向承诺 ×2 + 液面摊平守恒）`prototype/tests/test_rules.py`。72 passed；benchmark 27.2/13.9 FPS 无回退（基线 27.1/14.0）。注意：本修复改变模拟语义，既往 hash 序列作废（与 M0.5 同口径）。commit `fcc9312`。

### Fixed
- **R1 加载顺序确定性（A1 实施完成）**：`material.py` type_id 改按 `sorted(material names)` 分配（原按 toml 声明序），解耦 C# `Dictionary` 枚举序——消除跨平台 type_id 漂移 → state_hash 不一致的隐患。新增 `test_materials.py::test_type_id_assigned_by_sorted_name`（非字母序 fixture 红绿）+ `tests/test_load_order.py`（D3 capstone：真实 toml type_id 按 name 排序 + 双载 hash 一致）。83 passed。type_id 重排 → 既往 state_hash 序列作废（语义等价，录放/同 seed 等价测试不受影响）。A2（reaction 排序）经核对为非 live bug 已砍。commits `f3a9600`、`ef48c8d`。

### Added
- `docs/reference/ep01-sandsim-comparison.md`：外部参考实现对照（GameEngineering/EP01_SandSim，C+OpenGL 教学 demo，main.c 3215 行通读）。结论：算法/确定性/数据驱动/并行我们全面更强（EP01 硬编码材质 + `rand()` 非确定 + 无 chunk）；EP01 唯一不可替代价值是**实机渲染参考**——bloom 后处理、整纹理上传+NEAREST、velocity 多格手感、fire 视觉分层。可落地借鉴项已并入路线（velocity 队列 #2、bloom/fire 视觉留 Phase 2）。
- `docs/reference/2026-06-14-deterministic-physics-netcode-survey.md`：刚体/物理确定性联机方案专项调研（deep research，5 角度 / 23 源 / 25 条对抗式验证，23 confirmed / 2 killed）。结论：**Teardown 2026-03 混合架构（破坏走确定性命令流 + 刚体走状态同步）是与我们同构的直接商用先例**，"刚体走状态同步"是合理默认而非无奈；Box2D 3.1 默认已跨平台确定（无需定点，但需关 FMA + 确定接触顺序 + 无 rollback）、Quantum 全栈定点已出货 32 人物理。为 R3 三路线对比提供依据。
- `docs/reference/2026-06-14-tech-route-critical-review.md`：技术路线批判性复核（deep research，5 角度 / 21 源 / 25 条对抗式验证，15 confirmed / 10 killed）。结论：多数决策有一手先例支撑（定点+counter RNG、正方形写域、运动学、单缓冲布局），**联机 lockstep 是最高风险**。
- `docs/overview/architecture.md`：架构总览导航（三阶段 + M0–M3 里程碑 + Phase 1 玩法队列 + 最终架构两支柱 + 代码地图 + 文档指针 + 不变量速查）。新建 `docs/overview/` 目录。

### Proposed
- `docs/proposals/2026-06-14-determinism-hardening-r1-r3.md`：R1 迭代顺序 + R3 刚体桥接确定性加固方案。**R1**：审计确认 sim 热路径无顺序依赖，只加载期两处（`material.py:43` type_id 按 toml 序、`reaction.py:35` tag set 枚举序）→ 改 `sorted()` + 打乱顺序防回归测试（Phase 1 即做，~1h）。**R3**：刚体取 **R3-A（Teardown 式混合）**——刚体属实体层、不进地形 tick，走状态同步+客户端预测；R3-B（Box2D 3.1 lockstep）/R3-C（全栈定点）留作 M2 评估的升级路径。同步 architecture §5 加刚体归属句。

### Changed
- **采纳 deep research 复核（2026-06-14），3 项 actionable 落账**：①**R1** proposal §3 **D3 契约补强**——sim 遍历禁裸 `Dictionary`/`HashSet`、强制稳定排序（Box2D 作者亲证无序迭代即非确定；M1 C# 迁移头号风险），同步 architecture §8 不变量；②**R2**（用户校正）proposal §4.4 澄清 Noita Entangled Worlds **不构成 lockstep 反证**——它走状态同步是被闭源不确定引擎所迫（没得选），仅证退路 C 工程可行；**路线 B 仍首选，退路 C 维持兜底，M2 实测确认，不因它调整 B/C 优先级**；③**R3** 刚体桥接浮点确定性陷阱可能使"地形/弹幕/刚体"成三层同步，记入开放问题。另确认：velocity 用 8.8 定点累加器（非概率取整）；C# 数据布局不预设 SoA 收益（"SoA 必快"论断 0-3 否决，需实测）。涉及 `docs/proposals/2026-06-06-deterministic-parallel-and-netcode.md`、`docs/overview/architecture.md`。
- **液体/气体 dispersion rate**（spec `docs/superpowers/specs/2026-06-07-liquid-dispersion-design.md`、plan `.../plans/2026-06-07-liquid-dispersion-plan.md`，commits `fecdc68`→`951f0fa`）：
  材质字段 `dispersion`（water 5/oil 2/lava 1/steam 3，缺省 1），横移一帧沿方向记忆探测最多 N 格、落最远连续 AIR，
  首格保留 ±1 密度置换；探测纯确定（无 RNG）、写域边界夹断。`_move_liquid`/`_move_gas` 共用 `_probe_side` helper（`prototype/core/rules.py`）。
  摊平收敛 800→100 帧（≈8×）；benchmark 128² 26.6 FPS / 192² 13.2 FPS（基线 27.2/13.9，变化 -2.2%/-5.0%，预算内）。
  新增 5 测试 + 1 速度契约测试 + 更新 2 个 ±1-era 测试为方向承诺不变量断言（80 passed）。hash 序列作废（语义变更，与 M0.5/方向承诺修复同口径）。
- `prototype/demo_density.py`：密度沉浮演示场景（沙穿水下沉、油浮上水面）。
- `docs/sessions/2026-06-14-leveling-fix-and-dispersion.md`：本会话总账（液面冻结修复 + dispersion 实施）。

## 2026-06-07

### Added
- **M0.5 单线程 4-pass/chunk 调度器完成**（提案 §5 M0.5 行；fresh evidence：69 passed / 缝隙守恒 / 产物盖戳红绿验证 / replay 仍确定 / M0 hash 序列按预期作废）：
  - `prototype/core/chunks.py`：正方形写域 `[chunk−32, chunk+96)²` + 4-pass parity 纯几何，含"同 pass 写域两两不相交"穷举单测。
  - `prototype/core/grid.py`：update() 重写为所有权制 pass→chunk 扫描；STRIDE 4→5 加 `UPDATED_AT` 世代戳（swap 双方盖戳、`set_cell(stamp=)` 显式参数——决策②）；**删 FLAG_DIRTY/FLAG_STATIC 与每帧清 flag pass**（决策①）；`_check_reactions` 读域夹断 + 产物盖戳；RNG pass_id 接线。
  - `prototype/tests/test_chunks.py` + `test_chunked_semantics.py`（192×128 多 chunk）：材质计数守恒（缝隙无源汇）、沙柱跨水平缝、水过垂直缝、多 chunk 污染测试、产物同帧不动（**红绿验证**：去掉盖戳必红）、写域拒绝直测。
  - 性能意外向好：128² **27.1 FPS，较 M0 后 +18%**（删 O(N) 清 flag pass 收益 > 调度开销），基本回到 M0 前水平；192² 14.0 FPS 数据点入档（决策③）。
- `docs/superpowers/specs/2026-06-07-m05-chunked-scheduler-design.md` + `plans/2026-06-07-m05-chunked-scheduler-plan.md`；提案 §2.3 row 5 同步决策②偏离（观察契约不变）。
- **M0 确定性地基完成**（提案 §5 M0 行，验收四件套全过——fresh evidence：56 passed / 污染测试过 / replay CLI 两遍逐字一致 / benchmark 入档）：
  - `prototype/core/rng.py`：SquirrelNoise5 counter RNG，完整 7 元 key（素数折叠 + 每帧预计算 frame_seed），金值锚定 + "sim 模块禁 import random" 防回归断言。
  - `prototype/core/ops.py` + `prototype/replay.py`：apply_brush 共用写入路径；JSONL demo 录制/headless 回放（header 嵌 materials.toml sha256，不匹配拒绝）；`main.py --seed/--record`。
  - `prototype/benchmark.py` + `docs/perf/baseline.md`：正式基准定版——M0 前 27.6 FPS → M0 后 23.0 FPS（同机同场景，**-17%，在 20% 预算内**；42 FPS provisional 降级为留档）。
  - `prototype/tests/test_rng.py` / `test_determinism.py` / `test_ops.py`：金值、key 独立性、同 seed 等价、**污染测试**（帧间扰动全局 random，hash 不变）、录放等价、错误 TOML 拒绝。
- `docs/superpowers/specs/2026-06-07-m0-determinism-design.md`、`docs/superpowers/plans/2026-06-07-m0-determinism-plan.md`：M0 实现级设计（用户批准）与 9-task TDD 计划（superpowers 流程：brainstorming → writing-plans → executing-plans → verification）。

### Changed
- `prototype/core/{grid,rules,material,reaction}.py`：6 处全局 `random.*` 全部替换为 keyed RNG（D2）；density 全线整数化、reaction probability → u32 threshold（D1）；CellGrid 增加 seed/_fseed/state_hash(crc32)（D5）。`data/materials.toml` 与全部测试 fixture 密度 ×10。
- 既有测试重钉：删除全部 `random.seed()`，seed 扫描循环 collapse 为确定性单断言。

## 2026-06-06

### Added
- `docs/reference/noita-deep-dive.md`：Noita 深度调研报告（4 路并行网络调研 + prototype 现状对照）。覆盖：目标效果全景（材质规模/染色 stains/打击感构成）、核心算法确证（单缓冲循环与我们一致）、超越朴素 CA 的运动学扩展（速度/重力积分、CA↔粒子双轨、dispersion rate、粉末 inertia）、刚体桥接与多线程核验、Phase 1 行动队列（§6）。
- `docs/reference/noita-multiplayer-and-determinism.md`：联机专题调研——Noita 多线程公开细节"挖尽"声明、模拟确定性证据链（世界生成确定、模拟大概率不确定）、四个联机模组架构对比（NT / NoitaMP / Entangled Worlds / Arena，含 NEW 同步协议源码级细节）、同类先例（Factorio lockstep / Teardown 确定性命令流 / Terraria diff / rollback 不可行性）。
- `docs/CHANGELOG.md`、`docs/sessions/`：按 CLAUDE.md §4 / §3.1 首次建账。
- `docs/perf/baseline.md`：M0 前性能基线（opus 评审实测，provisional）——`128x128, 30% active, ~42 FPS`、空网格底价 7.2ms、counter RNG 纯 Python ≈1.8μs/次（慢 `random.random()` 一个数量级，Phase 1 接受）；含里程碑对比与回退预算约定（落实 CLAUDE.md §5.3，评审 M6）。

### Proposed
- `docs/proposals/2026-06-06-deterministic-parallel-and-netcode.md`：①确定性棋盘格论证——写域互斥 + 读域夹断 + counter-based RNG 三条件 ⇒ 任意线程数位级一致；②确定性工程契约 D1–D10；③联机推荐"地形 lockstep + 实体状态同步"双层架构（NEW 式 chunk RLE 快照做修复/late-join 兜底）；④分阶段 M0–M3。后应用户质询补强 §2.3"顺序账本"——逐项论证 7 个顺序来源如何钉死到数据（核心：同 pass chunk 间因 footprint 不相交而可交换，"顺序不存在"；并显式声明棋盘格语义 ≠ 串行全网格语义，迁移时一次性接受）。待裁决：M0 入 Phase 1 队列、联机目标形态确认。

### Changed
- `docs/algorithms/parallel-update-strategies.md`：按已核验事实精化——十字写域精确表述（含 Petri 原话）、Margolus 标注"非 Noita 方案 + 天然确定性"、补 64/512 双层 chunk 结构与确定性 caveat。
- **四项用户裁决落账**：①fire spec 走 Noita 式（温度场降级实验分支，spec 头部加裁决横幅）；②M0 确定性地基批准、排 Phase 1 队首；③联机目标形态定为 coop + 小规模 PvP（M2 需加对称竞技场景）；④旧反应表火焰调参留档于 commit `b99b2ec`。提案 Status: Proposed → Trial。涉及：`docs/superpowers/specs/2026-05-26-fire-system-design.md`、`docs/proposals/2026-06-06-deterministic-parallel-and-netcode.md` §7。
- **采纳外部评审（用户提供的 GPT 审阅，6/6 成立）**：①RNG key 升级为完整 7 元组 `(seed, tick, pass_id, x, y, salt, attempt)`——修复"确定但强相关"隐患（同帧同格多次掷骰返回同值、子像素概率取整被偏置）；②staged plan 插入 **M0.5**（Python 单线程 4-pass 语义原型，避免 Phase 2 同时换语言+调度+并行）；③D1 补整数化细则（density 整数等级、概率 u32 阈值 + 2 的幂量化加载）；④实体连续占位升级为 §4.3 一等规则（量化实体快照 = 地形 tick 输入，量化边界 = 确定性边界）。涉及 `docs/proposals/2026-06-06-deterministic-parallel-and-netcode.md`。
- **采纳 opus 独立评审（第二轮，2 blocker / 6 major / 10 minor / 5 nit 全部成立并落账）**：除 Fixed 节的修复外——M3 M0 验收谓词重写（污染测试 + RNG 金值 + 回放等价，原"同 seed 同 hash"在 CPython 上空洞）；M6/m1/m8 性能账（基线入档、RNG 成本断言修正、回归测试规模上限）；m3 RNG key 取数时点钉死；m4 M0.5 测试网格 ≥192×192；m5 补 `lava+[flammable]→lava+fire` 反应（浸没木头）；m7 demo 录制头嵌 toml 哈希；m9 文档同步欠账（parallel 文档 Phase 1 行、CLAUDE.md §5.1/§5.2 过时表述、deep-dive §6 过时标注）；m10/n4 测试几何与调参注记；n2 Margolus 条件修正；n3 PvP 公平性入 M2/风险。工作量复核：M0 ~3 天、M0.5 ~2.5–3 天。**评审总判断：M0 可以开工**。
- `CLAUDE.md`：§5.1 velocity 行更新（8.8 定点目标）、§5.2 追加"2026-06-06 更新"块（燃烧不走反应表的豁免说明、旧示例反应过时标注、密度整数化、指向确定性契约与性能基线）。
- `docs/superpowers/specs/2026-05-26-fire-system-design.md`：**全文重写为 v2**——主线 Noita 式（fire_hp / 静态温度比较 / requires_oxygen / counter RNG 完整 key），新增**延迟点燃队列**设计（防帧内沿扫描方向的连锁偏置）；蔓延行为显式由数值编码（wood 仅经火苗+氧气表面蔓延、oil 相邻直燃含水下、水蒸发复用燃烧机制）；v1 温度场整章降级为附录 A（实验分支，3 项开启前置条件）。消除"裁决横幅 vs 正文温度场"的自相矛盾。

### Fixed
- **opus 独立评审揪出 2 个设计 blocker，已修复**：①B1 冷油自燃——fire spec 热源定义缺 `fire_hp==0` 门控，未燃烧的油（temperature_of_fire=120 > 自身阈值 100）会无火自爆并蒸干邻水；②B2 十字写域角落死锁——所有权扫描语义下 (63,63)→(64,64) 对角移动永久不可达、每 pass 25% 面积无人可写，**写域改为正方形 `[chunk−32, chunk+96)²`**（穷举验证：同 pass 两两不相交且恰好密铺，交换律保持）。另修复 M1（ignite_queue 陈旧条目湮灭 ash，apply 时复检）、M2（缝隙延迟"≤3 pass 帧内"为假，正确口径=最坏下一帧对应 pass）、M4（实体占位快照必须走 reliable 命令流，通道按"是否进入地形 tick"划分）、M5（burn pass 的 pass_id=4 约定 + M1 分块化预告）。涉及：fire spec、提案、parallel-update-strategies.md。
- `docs/reference/noita-deep-dive.md`：应用户质询，对 5 组承重结论做一手来源逐字抽查（80.lv / macuyiko / jason.today / FSS issues #3 #4 / materials.xml dump 直查），4 组全部逐字命中；删除 1 条伪引语（"temperature is not part of this simulation" 不存在于其声称出处），"Noita 无温度场"结论改由数据文件结构证据支撑（报告 §2.3 + §7 抽查记录）。

### Investigating
- **重大发现：Noita 没有温度场/热传导**（开发者直述，80.lv）。火 = 材质静态常量比较（`temperature_of_fire` vs `autoignition_temperature`）+ 随机方向概率点燃 + `fire_hp` 消耗；连 lava 点火/固化都走反应表。`docs/superpowers/specs/2026-05-26-fire-system-design.md` 的"每像素温度场 + 传导 pass"属自创设计，与 chunk 休眠优化正面冲突——待裁决，建议先实现 Noita 式、传导降级为实验分支（报告 §5.3）。
- Noita 的 `cell_type` 只有 solid/liquid/fire/gas 四种，粉末 = liquid + `liquid_sand="1"`；玩家/敌人不是 Box2D 刚体而是逐像素碰撞的 kinematic entity——均与我们原先的直觉假设不同（报告 §2.2、§4.3）。
