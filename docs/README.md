# docs/ 导航

> 文档路径：`docs/README.md`
> 最近更新：2026-08-30 (UTC+8)

## 现行真源（先读这两篇）

| 文档 | 管什么 |
|---|---|
| [`overview/kernel-charter.md`](overview/kernel-charter.md) | **项目宪法**：第一性原则 P1–P5、三层内核、联机模型、确定性法典、里程碑 M0–M6、决策日志（含翻案记录） |
| [`overview/program-architecture.md`](overview/program-architecture.md) | **程序架构**：四环结构、crate 布局、子系统清单、规范 tick 管线、跨层通信白名单 |

## 当前优先队列

> **顺序约束（2026-08-31，已全部满足）**：行扫描定向修复 → 双机 hashrun → Layer G Task 2
> 的 `G_ACCEL=0` 逐位回归（以重建后的基线为准）三步已依次跑完。基线已再次用 Task 2 后的
> 语义重建，**Task 3 的逐位回归以新基线为准**。

0. **粉末方向偏置修复：已完成（2026-08-31）**——`proposals/2026-08-31-powder-scan-direction-bias.md`。
   行扫描定向 `(y+tick)&1` → `(y ^ scan_flip(fseed))&1`；对称几何下偏置 −6.3% → +0.04%（CI 跨 0）。
   总纲 §4 正文已订正、§11 实施期决策第 3 条已落。**残留的 −0.8%（镜像轴压 chunk 边界时四相棋盘
   处理次序不镜像）已由用户裁定为四相调度的特性、不算 bug、不修**（2026-08-31，提案 §7 第 1 条）；
   规避照旧 = 竞技地图镜像轴避开 64 的倍数。本条**已结案**，M4 不重开。

1. **双机 hashrun：已完成（2026-08-31）** —— Linux rustc 1.89 × Windows rustc 1.97，9 场景最长
   2 万 tick，全部 tick 哈希与 final **逐位相同**。同轮查出握手指纹对行尾敏感（CRLF 检出使同一
   commit 算出不同 fp，仿真无碍），已修（总纲 §11 实施期决策第 4 条）。M0 起挂账的跨机验收项清账。
   **下一步 = Layer G Task 2（重力速度积分）。**
2. **Layer G 运动语义重做：三 Task 全部完成并经用户目检确认（2026-08-31，spec: Implemented）**——`superpowers/specs/2026-08-31-layer-g-velocity-design.md`。
   范围 = 液体色散 ≤8 + 重力速度积分 + 撞击溅射脱格，分三 Task 独立落地。
   - **Task 1 液体色散 ≤8：已完成并经用户目检确认（2026-08-31）**——`materials.ron` 加
     `dispersion` 字段（water 5），`rules::side` 改"最远可达空格"+ `DISPERSION_MAX` clamp。单测 10 条、golden
     重录（`sand_pile` 逐 tick 哈希逐位不变，仅 `materials_fp` 变）、SyncTest waterfall+mixed 各 2 万 tick 六配置
     零分叉、bench 见 `perf/2026-08-31-layer-g-task1-dispersion.md`。目检 GIF：`out/waterfall_disp{1,5}.gif`、
     `out/mixed_disp{1,5}.gif`（改动前/后对照，看水面锯齿与摊平速度）。总纲 §4、§11 已同步。
   - **Task 2 重力速度积分：代码与验收已完成（2026-08-31），仅剩 GIF 目检结论待用户**——`Cell`
     bits 17–21 存 Q3.2 竖直速度，`rules::eval` 外包子步循环，撞停清零。零加速旁路取证
     4 场景 4500 条**逐 tick** 哈希 diff 全空（`.superpowers/layer-g-task2-gravity/`）；
     SyncTest waterfall+mixed 各 2 万 tick 六配置零分叉；golden 四个重录（预期全兑现）；
     bench 见 `perf/2026-08-31-layer-g-task2-gravity.md`（一致变慢 5%–34%，预期内的语义
     成本，绝对量级远在预算内）。r 契约升格为编译期断言（12 ≤ 16）。目检 GIF：
     `out/sand_pile_g{0,1}.gif`、`out/mixed_g{0,1}.gif`（g0 = 改动前），重点看加速手感
     与"斜滑不清零速度"导致的沙堆坍塌是否过快。总纲 §4、§11 已同步。
   - **Task 3 撞击溅射脱格：已完成并经用户目检确认（2026-08-31）**——
     G→P（cell 撞停脱格，`Chunk::spawn_buf` + 相位屏障后按 chunk index 升序 drain）
     **与 P→G（粒子落格把撞击速度写进 cell 速度位，用户中途裁决并入）** 两个方向都补齐。
     SyncTest mixed+waterfall+explosion_splash 各 2 万 tick 六配置零分叉（跑了两轮）；
     线程数不变性 1/8/16 逐位相同；golden 四个重录；bench 见
     `perf/2026-08-31-layer-g-task3-splash.md`（`acceptance` 中位 +2%，比 Task 2 小一个量级）。
     目检 GIF：`out/mixed_splash{0,1}.gif`（G→P）、`out/waterfall_ci_splash{0,1}.gif`（P→G），
     0 = 改动前；重点看水花量与 §6.1①（`MovedSide` 也触发 ⇒ 贴地横流会不会冒过量水花）。
     **遗留**：横向撞击动量仍被丢弃（网格无水平速度场），留 M2 之后。
3. **M2 反应表与燃烧：已完成并经用户签收（2026-09-02）**——spec
   `superpowers/specs/2026-08-31-m2-reactions-and-fire-design.md`（Status: Implemented），
   四 Task 全落：数据层 + `Category::Gas`、反应表（tag 展开/发起方约定/`STREAM_REACT`）、
   `hp`+`durability` 双层破坏（哨兵退役）、counter 燃烧链（点燃/产火/衰变/闷熄/灭火）。
   `fire_oil_chain` golden + 六配置 2 万 tick 零分叉；分布回归新规矩落地；总纲 §11
   实施期决策第 7 条（含翻案 6 措辞修正）。bench + u64 对照见
   `perf/2026-08-31-m2-reactions-and-fire.md`。目检 GIF：`fire_oil_chain_preview.gif`
   （repo 根，未入库；重点看火油连锁、烟上升、木头由外向内烧）。
   **遗留债不变**：O3 粉末惯性、粒子穿水/弹跳、M1 两条测试债（用户裁决推迟）、
   横向撞击动量（无水平速度场）。
   **2026-09-01 追加**：Gas `rise_chance`（火焰逗留，Noita 查证）+ 渲染器燃烧可视化；
   目检 GIF `oil_burn_demo.gif` / `oil_wood_bonfire.gif`。
3b. **地图编辑器支线：已完成并经用户签收（2026-09-02）**——spec
   `superpowers/specs/2026-09-01-map-editor-design.md`（Status: Implemented）。场景 RON
   新增 `grid` 字段（行级 RLE + 材质名图例，加载期编译成 Fill 前缀，core 零改动）；
   `sand-harness materials --json` / `rasterize`；`tools/map-editor/`（单文件画布 +
   Python 改完即渲服务，见其 README）。**M3 刚体：2026-09-02 用户改判——回到 M4 之前**（先把世界层
   做完整）；`fire_oil_chain` 等场景不翻新（用户裁决）。**下一步 = M3 brainstorm。**
4. **M1 粒子层：已完成并经用户验收（2026-08-31）**（spec → Implemented，会话总账 `sessions/2026-08-30-m1-particle-layer.md`）：
   脱格/落格闭环、DDA、`Op::Emit`/`Op::Explode`（Noita 射线模型 + 近心汽化 + 密度冲量 + 方向涨落）、容量限流；验收 §0
   五项全过（GIF 目检经用户四轮迭代后确认）。后续爆炸手感收口 + `world.rs` 拆分 `explode.rs`/`emit.rs` 见 CHANGELOG
   2026-08-30 块（commits `66cea0a`..`33ab3da`）。
5. M3 刚体 → M5 时启用 O4 运行时周期哈希（详见总纲 §11 与相关提案）。

## 目录分工

| 目录 | 内容 | 状态 |
|---|---|---|
| `overview/` | 总纲、架构；`architecture.md` 为 Python 原型时代旧版（superseded） | 现行 |
| `CHANGELOG.md` | 工作账本（按产出索引） | 现行 |
| `tuning-knobs.md` | **手感旋钮总表**：材料表字段 + Layer G/粒子/爆炸常量的现值、拧它的后果与代价，外加"明确不是旋钮"清单与待裁决缺口。统一调参从这里进 | 现行 |
| `sessions/` | 会话总账（按时间索引） | 现行 |
| `proposals/` | 改动提案；2026-06 两篇为原型时代产物，部分被总纲取代（见各自 Status 行） | 部分史料 |
| `algorithms/` `materials/` | CA 算法与材质体系文档（Python 原型时代，思想仍有效，代码锚点指向 `archive/`） | 史料为主 |
| `perf/` | 性能基准；Rust 基线已建立（M0 `2026-08-30-m0-rust-baseline.md` + M1 `2026-08-30-m1-particle-baseline.md`），Python 基线保留作参照 | 现行 |
| `reference/` | 外部调研（Noita 深挖、确定性联机调研、EP01 对照等），**仍然有效** | 现行 |
| `superpowers/` | brainstorm specs 与 plans（原型时代产物见各自 Status 行） | 混合 |

## 史料阅读须知

2026-08-29 项目大转向：东方同人横版动作（Python 原型 → Godot C#）→ **1v1 落沙法术对战（Rust 内核 + Godot 表现层）**。此日期之前的文档中，凡与总纲冲突的结论以总纲为准；被显式推翻的裁决列于总纲 §11 翻案记录。Python 原型代码及其 80+ 测试归档于 `archive/prototype-python/`。
