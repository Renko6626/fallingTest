# M3 刚体：像素↔刚体管线全链路 + 盖章耦合 + 破坏重提取

> 文档路径：`docs/superpowers/specs/2026-09-02-m3-rigid-body-design.md`
> 运行时版本：Rust（内核）+ Rapier2D（引擎，关在 `sand-core::physics` 内）
> 最近更新：2026-09-02 (UTC+8)
> **Status**: Implemented（2026-09-02 五 Task 落地；GIF 目检待用户——`crate_yard.gif`）
> 上游：`docs/overview/kernel-charter.md` §5（刚体住核心内、同步态、全 lockstep）、§11 翻案 1、里程碑 M3；`docs/overview/program-architecture.md` §3 `stamp` / `physics-adapter`、§4 tick 管线第 3/7 步
> 调研：`docs/reference/noita-deep-dive.md` §1.5/§4.2（FSS 管线、逐像素浮力涌现）、`docs/reference/noita-material-schema.md` §3.3；wiki 一手核查见 §1.2

---

## 实施进度

| Task | 内容 | 状态 |
|---|---|---|
| 1 | `physics` 适配层（Rapier2D 封装、确定性红线、快照）+ 几何工具（矩形覆盖 / 4 连通分量） | ✅ 2026-09-02 |
| 2 | `body` 本体：位图、盖章/反盖章（含 counter 往返）、`Op::SpawnBody`、哈希入 `state_hash` | ✅ 2026-09-02 |
| 3 | 地形碰撞（B′：刚体附近 chunk 缓存的硬格矩形）+ 浮沉（水面线采样阿基米德） | ✅ 2026-09-02 |
| 4 | 破坏对账 + 限额重提取 + 碎片脱格；燃烧散架端到端 | ✅ 2026-09-02 |
| 5 | 收口：`crate_yard` golden/SyncTest、快照往返、bench、总纲 §11、GIF 目检 | ✅ 2026-09-02（目检待用户） |

---

## 0. 验收标准

1. **总纲 M3**：爆炸切割木箱（body 数 1 → ≥2）、落水浮沉（木箱浮、石箱沉）、SyncTest 六配置 2 万 tick 零分叉（`crate_yard`）。
2. **燃烧散架**：木箱被点燃后逐像素烧掉、形状重算、最终散架成碎片（Noita 语义，§1.2 第 3 条）。
3. **休眠不退化**：静止/漂浮的刚体零写入，全图入睡（`resting_body_lets_chunk_sleep`）。
4. **快照往返**：`snapshot → restore → 继续 N tick` 与不恢复逐位相同（M6 决策门预演）。
5. **golden 处置**：`state_hash` 结构变更（并入刚体层）⇒ 先 `--grid-only` 证明网格哈希流逐位不变，再重录全部 golden；新录 `crate_yard`。
6. **bench**：无刚体场景无回退（第 3/7 步在刚体层为空时近零成本）；`crate_yard` 成本入档。
7. **GIF 目检**（用户）：落地堆叠、停在沙堆上、落水浮沉、切割、燃烧散架。

## 1. 裁决与查证

### 1.1 用户裁决（2026-09-02）

1. **刚体来源 C**：`Op::SpawnBody` 矩形起步，管线按任意位图设计（切割后本就是任意形）。
2. **地形 B′**：沙（粉末）托得住刚体——所有非 air / 非 Gas / 非 Liquid 且非刚体自身的格都是硬地形；碰撞几何只在刚体附近按 chunk 缓存重建。材质字段 `body_passable`（对应 Noita `liquid_sand_never_box2d`）保留每材质开关。
3. **小碎片 B**：面积 < 阈值的分量脱格成粒子（复用 G→P 通路），不换材质（`debris_to` 留 M4）。
4. **引擎：Rapier2D**（`enhanced-determinism` + `serde-serialize`）。
5. **刚体可燃**：刚体像素照常参与 CA 燃烧，烧掉即对账重算（Noita 语义）。
6. **浮沉：采样式阿基米德**（2026-09-02 二次裁决，推翻同日"Noita 逐像素反作用"之议：更优雅、更省）；`density` 单字段复用为刚体密度。溢出（"箱子掉进满池水就漫出来"）由 §3 的排开脱格保证质量守恒，与浮力公式无关。

### 1.2 Noita 一手查证（wiki + 仓库调研）

1. `liquid_sand_never_box2d="1"` 的释义是"让 box2d 物体**穿过**该材质而不被卡住"——它是关闭开关，说明**沙默认托得住刚体**；地面本就是 `liquid_static` 静态沙而非 `solid`（`solid` 铺满世界会"lag like hell"）。
2. `density` 字段原文 `(liquids, sand) More dense materials seep through lesser ones`——**只定义给液体/沙**，没有任何 solid/box2d 或浮力字段；水 4、木 11。
3. `[box2d]` 木材（`wood_prop` / `wood_player_b2` / `wood_loose`…）全部 "Burns: Yes"；Purho："每个像素知道自己属于哪个刚体……像素被毁就重算形状（可能切成多块）"——烧掉与炸掉走同一条路。
4. 浮力（社区共识，`noita-deep-dive.md:300`）：tick 末遍历 body 像素，遇沙/液体 → 该像素转入粒子系统（飞溅）+ 在该点施加反向力与阻尼；**没有显式公式**。我们只采纳其"排开像素进粒子系统"这一半（§3），浮力本身走阿基米德（§5，用户二次裁决）。
5. FSS Issue #4：刚体旋转后网格出现缝隙、沙漏进箱内——盖章必须实心光栅化（§3）。

## 2. 架构与数据流

两个子系统落在架构 §3 预留位：

- **`sand-core::body`**（= `stamp`）：刚体同步态本体。全系统**唯一同时读写 grid 与 physics 的模块**（架构 §4 白名单）。
- **`sand-core::physics`**（= `physics-adapter`）：Rapier 世界的薄封装，trait 隔离引擎，core 之外看不到 rapier 类型。

```rust
pub struct Body {
    pub id: u16,             // 单调分配，入哈希
    pub material: u8,        // 必须 Static 类别（加载期契约）
    pub w: u16, pub h: u16,  // 局部位图尺寸
    pub occ: Vec<u8>,        // 局部像素：0 = 空；否则 1 + counter（燃烧进度随刚体走）
    pub stamped: Vec<(i32, i32)>, // 上一 tick 盖章格清单（反盖章/对账驱动，不扫 AABB）
    pub dirty: bool,         // 对账发现像素被毁 ⇒ 待重提取
    handle: BodyHandle,      // 引擎句柄（不入哈希，派生量）
}
pub struct Bodies { list: Vec<Body> /* 按 id 升序 */, next_id: u16, reextract_queue: Vec<u16> }
```

**tick 管线不改编号**（架构 §4 原文位置；第 3/7 步从占位变生效，按 M1 粒子相先例记 §11 实施期决策）：

- **第 3 步（网格四相之前）**：对每个**清醒**刚体按 id 序：反盖章 → 水面线采样、施加浮力与阻力（§5）→ 刷新刚体附近 chunk 的地形碰撞（§4）→ `physics.step(1/60)` → 按新变换实心盖章（§3）。
- **第 7 步（粒子相之后）**：对账（含睡眠刚体）→ `dirty` 入队 → 限额重提取（§6）。

## 3. 盖章/反盖章与格子所有权

- **所有权 = Cell bit 23 `BODY_FLAG`**（M2 留白位）。盖章格 = `pack(material) | BODY_FLAG | counter`。标志只做两件事：地形掩码排除自身（§4）、对账识别（§6）。**不豁免任何 CA 规则**：刚体像素就是其材质的 Static 格——`needs_eval` 照常（counter > 0 进 burn）、可被点燃（`with_counter` 不动其他位，标志保留）、可产火、邻接水即灭火、烧尽 `decay_to` 写成 air（标志随之消失）→ 对账视作像素被毁。`displace` 因 `is_static` 从不置换它；爆炸射线按材质 `hp/durability` 照常摧毁。
- **加载期契约**：`SpawnBody` 的材质必须 `Category::Static`（否则"不动"要另做特判）。
- **盖章 = 实心光栅化（逆映射）**：对变换后 AABB 内每格，格心逆变换回局部坐标查 `occ`——天然无洞（§1.2 第 5 条）。两 body 争同一格：**id 小者赢**，盖章按 id 序。
- **被盖住的非 air 格**：Liquid/Powder 经 `eject_cell` 脱格成粒子（chunk `spawn_buf` 定序，质量守恒、即溅射）；Gas 直接覆盖（气体无质量语义）；Static 非刚体格（wall/wood）**不覆盖**——盖章跳过该格（刚体嵌进地形是物理层该防的事，不在这里销毁地形）。
- **反盖章** = 按 `stamped` 清单把格写回 air，**同时把每格 counter 读回 `occ`**（燃烧进度随刚体走，§1.1 第 5 条）。
- **休眠/静止刚体零写入**：Rapier 判定 sleeping、或本 tick 变换与上 tick 逐位相同的 body，跳过反盖章/采样/盖章，格子原样留着（火照样在格子里烧）；执法测试 `resting_body_lets_chunk_sleep`。

## 4. 碰撞几何

- **地形（B′）**：硬格 = 非 air、非 Gas、非 Liquid、非 `BODY_FLAG`、且材质 `body_passable == false`。只为刚体 AABB 外扩 `TERRAIN_MARGIN = 1` chunk 范围内的 chunk 生成：硬格掩码 → Marching Squares（多轮廓含洞）→ Douglas-Peucker（`DP_EPSILON = 0.5` 格）→ **polyline 静态碰撞体**（静态地形不三角化）。**按 chunk 缓存**；失效 = 该 chunk 上一 tick `dirty` 非空（现成位）或本 tick 有刚体盖章/反盖章；离开范围的 chunk 从引擎移除。
- **刚体形状**：`occ` → Marching Squares → DP → **自写耳切三角化**（遍历序固定；**不用** Rapier 的凸分解，其定序无保证）→ 三角形 compound collider；质量由引擎按面积 × 材质 `density` 算（§5）。位图变了才重算。
- 材质表新增 `body_passable: bool`（缺省 false）。

## 5. 浮沉与阻力（采样式阿基米德，用户裁决第 6 条）

- **质量**：collider 密度 = 材质 `density`（Static 材质的 `density` 自此定义为"作为刚体的密度"——CA 里 Static 永远先被 `is_static` 拦掉、从不比密度，此字段此前无消费者，零副作用）。`materials.ron`：wood 60 → **12**（< 水 16，浮）；新增 `stone`（Static，密度 40，沉）。
- **淹没体积 = 水面线采样**（不是"数流进脚印的液体格"——一 tick 只流进一层，那会把力算成正比于周长）：对每个清醒刚体，扫 AABB 左右各外扩 1 列、在脚印之外自上而下找第一个 Liquid 格，得各列水面高度；取**中位数** `h`（偶数个取较高者，整数）；找不到液体的列不计；一列都没有 ⇒ 未淹没。淹没像素集 = 刚体占位像素（世界坐标）中 `y ≥ h` 者，`n_sub` 为其计数，`ρ_liq` 取各列水面格里出现最多的液体材质的 `density`。
- **力**：`F_浮 = n_sub × ρ_liq × g` 逆重力，施于**淹没像素质心**（白送扶正力矩，箱子会自己翻正）；阻力 `F_阻 = −K_DRAG × n_sub × v_质心`。引擎重力照常施加 `−m g`，净力即 `(n_sub ρ_liq − n_total ρ_body) g`——真阿基米德，与尺寸无关。
- **确定性**：全部整数计数，进引擎前才转 f32；按 id 序施力。O(AABB) / 清醒刚体 / tick。
- **排开与溢出**：盖章覆盖到的液体格按 §3 脱格成粒子（质量守恒）——满池入箱即漫出，是 §3 的副产物，不需要额外机制。
- **防抖**：本 tick 变换与上 tick 逐位相同（`to_bits` 相等）的刚体**跳过反盖章/采样/盖章**——浮着不动或堆着不动的箱子零写入、零溅射；睡眠刚体同样跳过（§3）。
- `K_DRAG` 常量，目检调；第一版不做角阻尼（Rapier 自带 angular damping 参数，需要时开）。

## 6. 破坏对账与重提取（第 7 步）

- **对账**：每个 body（**含睡眠的**——爆炸与燃烧不管它睡没睡）遍历 `stamped`，凡格不再是 `material | BODY_FLAG` 即像素被毁 → 清 `occ` → `dirty`。
- **重提取限额**：`dirty` 入 `reextract_queue`（按 id 序），每 tick 最多 `MAX_REEXTRACT_PER_TICK = 2` 个，超限顺延（队列入状态、入哈希）。处理 = `occ` 4 连通分量分解：
  - 面积 ≥ `MIN_BODY_PIXELS = 12` → 各成新 body（新 id，**继承父 body 线速度/角速度/材质**，位置按分量质心换算），父 body 移除；
  - 面积 < 12 → 逐像素 `eject_cell` 成粒子，格置 air；
  - **滞回**：若分量只有一个且仍 ≥ 阈值 → 原 body 就地换形状、id 不变。
- 新 body 的形状按 §4 重算，下一 tick 第 3 步正常盖章。

## 7. 确定性与哈希

- **引擎红线**（写进 `physics` 模块文档）：`rapier2d` 开 `enhanced-determinism` + `serde-serialize`；单线程步进；固定 `dt = 1/60`、无子步；建/删/施力/查询**全部按 body id 序**，绝不用 Rapier handle 迭代序驱动任何写入；引擎版本锁 Cargo.lock（lockstep 本就要求同二进制）。
- **浮点边界**：引擎 → 网格只有 `transform(handle)`（f32）→ 逆映射盖章（f32 算、布尔出）；网格 → 引擎只有整数格坐标折线与整数计数 × 常量。同 bits 进同 bits 出。
- **哈希**：`state_hash = combine(网格根, 粒子层, 刚体层)`；刚体层 = 按 id 序折叠 `(id, material, w, h, occ 哈希, transform.to_bits(), linvel.to_bits(), angvel.to_bits(), sleeping)` + `next_id` + `reextract_queue`。Rapier 内部状态（接触缓存等）是派生量不折；SyncTest 另每 256 tick 比对 `snapshot()` serde 字节 checksum，专盯引擎内部分叉（M6 门预演）。
- **快照**：`physics.snapshot()/restore()` 走 serde；M3 只验往返恒等（验收 4）。

## 8. 场景 op、数据与常量

- `Op::SpawnBody { material, x, y, w, h }`（RON `SpawnBody(material: "wood", x: 100, y: 40, w: 24, h: 16)`）：加载期校验材质 Static、尺寸 ≥ 阈值、落在世界内；生成 = 全占位图 → §4 形状 → 插入引擎（静止无旋转）→ 下一 tick 首次盖章。`MAX_BODIES = 256` 超限确定性拒绝并计数（粒子池先例）。
- `materials.ron`：`body_passable`（缺省 false）；wood `density: 12`；新增 `stone`（id 8，Static，density 40，hp 6，durability 8）；`debris_to`（目检修订）：wood → `wood_debris`（id 9，Powder）、stone → `stone_debris`（id 10，Powder）。
- 常量（`pub const`，目检可调）：`MIN_BODY_PIXELS = 12`、`MAX_REEXTRACT_PER_TICK = 2`、`DP_EPSILON = 0.5`、`TERRAIN_MARGIN = 1`、`K_DRAG`、`MAX_BODIES = 256`。
- 指纹：场景字节哈希自动覆盖；`Op::SpawnBody` 无 `Fx` 字段不折叠。

## 9. 测试矩阵

- **单测**：耳切定序与面积守恒；Marching Squares 多轮廓含洞；DP 端点保持；逆映射盖章旋转 45° 无洞（FSS #4 回归）；连通分量 + 阈值分流；`physics`"两世界同序操作 → 快照字节相同"；`occ` counter 往返（反盖章读回 / 盖章写回）。
- **行为测试**（`tests/body_behavior.rs`）：木箱落墙静止后全图入睡；木箱停在沙堆上（B′）；木箱落水上浮、石箱下沉；**满池入箱水漫出**（池外出现液体格且总水量守恒）；爆炸切割 body 1 → 2；小碎片脱格粒子；**木箱点燃后 `occ` 单调减少、最终 body 消失**；快照往返恒等。
- **SyncTest**：`crate_yard`（木箱×3 + 石箱 + 沙堆 + 水塘 + 定时爆炸与点火）六配置 2 万 tick；引擎快照 checksum 每 256 tick 双实例比对。
- **golden**：`crate_yard` 新录；既有五个按验收 5 程序重录。
- **bench**：无刚体场景对照 `f807e54`…当前；`crate_yard` 入档。

## 10. Non-goals（M3）

关节/焊接；刚体间材质反应；玩家/敌人（kinematic，M4）；编辑器摆刚体（二期）；`debris_to`；rollback 本身；静置粉末以外的软地形调优；多线程物理；刚体 ↔ 粒子碰撞（粒子仍按 DDA 查网格，盖章格对它就是 Static）。

## 11. 决策记录

1. C / B′ / B / Rapier2D / 刚体可燃 / 采样式阿基米德（§1.1；Noita 逐像素反作用之议同日撤回）。
2. `BODY_FLAG` 只做所有权标记，不豁免 CA（§3）。
3. `density` 单字段复用为刚体密度，不开 `body_density`（§5）。
4. 淹没体积用水面线采样而非脚印回填计数——后者只数到一层、力正比于周长（§5）。
5. 静态地形用 polyline、刚体用自写耳切，不用引擎凸分解（§4）。
6. 哈希不折引擎内部状态，另以 serde checksum 巡检（§7）。
11. **目检修订（2026-09-02，用户三问）**：① "刚体不倾倒"——机制无恙（引擎与 Sim 两级探针
    都证实半悬空箱子翻倒），是 `crate_yard` 没安排会倒的物件；场景加台沿高箱子，行为测试
    `overhanging_crate_topples_off_ledge` 钉死。② "碎屑落回粘连"——爆炸/碎片粒子按原材质
    （Static wood）落格成悬空静态格，既像粘回箱上又成为卡住刚体的地形：落地原推迟的
    `debris_to` 字段（wood → `wood_debris`、stone → `stone_debris` 两种 Powder），
    `explode::fire_ray` 与碎片脱格都走它；测试 `explosion_debris_lands_as_powder_not_static`。
    ③ "水卡在浮箱顶"——两个根因叠加：浮力按整行计数使力随位置阶跃、浮体困在极限环里
    反复把回填水弹上箱顶；且施力用 `wake_up = true` **每 tick 强制唤醒**，水里的刚体永远
    睡不着。修：淹没量改按像素分数（用引擎变换算每像素世界 y）平滑，施力不唤醒，睡眠
    阈值收回 1 格/s；测试 `floating_crate_settles_and_stops_ejecting_water`（稳定后 300 tick
    零粒子、脚印不变）。
10. **实施期决定（Task 4，两条实测教训）**：① **盖章格用上一 tick 的世代戳**——`eval`
    以 `stamp == 当前` 判"本 tick 已处理"，若用当前戳，清醒刚体每 tick 重盖章 ⇒ 其燃烧格
    永远轮不到 CA 评估，燃烧只在刚体睡着时推进（火场里的箱子烧到 103 像素卡死）；盖章格
    是 Static、无二次移动风险，与 setup 用 255 让 tick 0 可动同一招。② 验收 2 的测试改为
    "持续火场"（每 60 tick 在箱顶补一排火）：M2 燃烧在**实心大块**上逐层推进时有概率
    断火（外壳烧尽后新一层只能靠火气体点燃，火寿命 40 + 单方向点燃骰），薄木构才自持
    烧净——那是 M2 燃烧参数（`fire_chance`/`lifetime`）的事，M3 证的是"烧掉像素 → 对账
    → 重提取 → 散架"链路。重提取拆分时子 body 继承父变换与速度、`last_xf = None` 令下一
    tick 反盖章/盖章一次接管格子；对账从清单剔除已毁格（否则反盖章会把别人的格写成 air）。
9. **实施期决定（Task 3，三条实测教训）**：① 地形缓存**比对矩形后才碰引擎**——删/重建
   静态碰撞体会重置接触并唤醒压在上面的刚体，与"盖章标脏 → 重建"形成死循环，静止箱子
   永远睡不着；② 水面线**只采 AABB 两侧各外扩 2 列**，不采 AABB 内的列——被排开、溅到
   箱顶的水会被误认成水面，形成"越浮越高"的正反馈把箱子弹出水面；③ `K_DRAG = 200`
   （16×12 木箱阻尼比 ≈ 0.8，近临界），引擎睡眠阈值按格/s 口径放宽（线 6、角 0.3；
   rapier 缺省 0.05×length_unit 是米制口径），否则浮体的力量化极限环（淹没行数跨越
   水面）永远睡不着。实测木箱平衡淹没 ≈ 75% = 12/16，阿基米德成立。
8. **实施期决定（Task 2）：第 3 步顺序改为"步进 → 变换未变则跳过 → 反盖章旧脚印（读回 counter）→ 盖章新脚印"**——
   反盖章与盖章本就在同一串行阶段背靠背，不必在步进前清旧脚印；防抖判定（`to_bits` 相等）
   由此天然成立。ops 循环从 `scheduler::step` 纯搬移到 `Sim::step`（World 不持刚体，
   `Op::SpawnBody` 在 Sim 路由），外部可观测顺序不变。整体出界 > 64 格的刚体确定性移除。
7. **实施期决定（Task 1，2026-09-02）：几何走"位图行程矩形覆盖"，不上 marching squares / DP / 耳切。**
   刚体与地形都编译成轴对齐矩形的 compound collider（`geom::rect_cover`，行主序贪心 +
   竖向合并，纯整数、天然定序）。理由：零多边形/含洞三角化的坑；矩形碰撞体比 polyline
   更不易穿隧、箱子落地更稳；只在刚体附近几个 chunk 生成，数量可控。§4 的 marching
   squares / DP 留待 bench 证明矩形数成为瓶颈时再上。`rapier2d 0.35` 自带 `PhysicsWorld`
   聚合体且整体 serde 可序列化，适配层直接包它。
