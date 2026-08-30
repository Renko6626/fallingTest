# docs/ 导航

> 文档路径：`docs/README.md`
> 最近更新：2026-08-30 (UTC+8)

## 现行真源（先读这两篇）

| 文档 | 管什么 |
|---|---|
| [`overview/kernel-charter.md`](overview/kernel-charter.md) | **项目宪法**：第一性原则 P1–P5、三层内核、联机模型、确定性法典、里程碑 M0–M6、决策日志（含翻案记录） |
| [`overview/program-architecture.md`](overview/program-architecture.md) | **程序架构**：四环结构、crate 布局、子系统清单、规范 tick 管线、跨层通信白名单 |

## 当前优先队列

1. **下一会话首项：双机 hashrun**（M0 起挂账的唯一跨机验收项，用户手动；M0/M1 场景一起跑——
   `hashrun` 用法见 `sessions/2026-08-30-m0-implementation.md`，两机各跑后 diff 逐字比对）。
2. **Layer G 运动语义重做（实施中，spec: Proposed）**——`superpowers/specs/2026-08-31-layer-g-velocity-design.md`。
   范围 = 液体色散 ≤8 + 重力速度积分 + 撞击溅射脱格，分三 Task 独立落地。
   - **Task 1 液体色散 ≤8：代码与验收已完成（2026-08-31），仅剩 GIF 目检结论待用户**——`materials.ron` 加
     `dispersion` 字段（water 5），`rules::side` 改"最远可达空格"+ `DISPERSION_MAX` clamp。单测 10 条、golden
     重录（`sand_pile` 逐 tick 哈希逐位不变，仅 `materials_fp` 变）、SyncTest waterfall+mixed 各 2 万 tick 六配置
     零分叉、bench 见 `perf/2026-08-31-layer-g-task1-dispersion.md`。目检 GIF：`out/waterfall_disp{1,5}.gif`、
     `out/mixed_disp{1,5}.gif`（改动前/后对照，看水面锯齿与摊平速度）。总纲 §4、§11 已同步。
   - **Task 2 重力速度积分**（下一步）：Cell 位段 17–21 存竖直速度 + 子步循环；关键取证是 `G_ACCEL=0` 逐位回归。
   - **Task 3 撞击溅射脱格**：并行 pass 新增 `spawns` 写入源，须落总纲 §11。
3. **M2 场层与反应表**（Layer G 三 Task 之后）：spec 里裁决 O2 场降本 + O3 粉末惯性时点 + durability/hardness
   字段化 + 粒子穿水/弹跳评估 + M1 遗留两条测试补强（见 `sessions/2026-08-30-m1-particle-layer.md`"留给后续"；
   其中 Task 6 minor ①②③⑤ 已由 commit `098fe23` 修掉，剩 ④⑥ 两条测试债）。
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
| `sessions/` | 会话总账（按时间索引） | 现行 |
| `proposals/` | 改动提案；2026-06 两篇为原型时代产物，部分被总纲取代（见各自 Status 行） | 部分史料 |
| `algorithms/` `materials/` | CA 算法与材质体系文档（Python 原型时代，思想仍有效，代码锚点指向 `archive/`） | 史料为主 |
| `perf/` | 性能基准；Rust 基线已建立（M0 `2026-08-30-m0-rust-baseline.md` + M1 `2026-08-30-m1-particle-baseline.md`），Python 基线保留作参照 | 现行 |
| `reference/` | 外部调研（Noita 深挖、确定性联机调研、EP01 对照等），**仍然有效** | 现行 |
| `superpowers/` | brainstorm specs 与 plans（原型时代产物见各自 Status 行） | 混合 |

## 史料阅读须知

2026-08-29 项目大转向：东方同人横版动作（Python 原型 → Godot C#）→ **1v1 落沙法术对战（Rust 内核 + Godot 表现层）**。此日期之前的文档中，凡与总纲冲突的结论以总纲为准；被显式推翻的裁决列于总纲 §11 翻案记录。Python 原型代码及其 80+ 测试归档于 `archive/prototype-python/`。
