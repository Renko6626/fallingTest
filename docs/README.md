# docs/ 导航

> 文档路径：`docs/README.md`
> 最近更新：2026-08-29 (UTC+8)

## 现行真源（先读这两篇）

| 文档 | 管什么 |
|---|---|
| [`overview/kernel-charter.md`](overview/kernel-charter.md) | **项目宪法**：第一性原则 P1–P5、三层内核、联机模型、确定性法典、里程碑 M0–M6、决策日志（含翻案记录） |
| [`overview/program-architecture.md`](overview/program-architecture.md) | **程序架构**：四环结构、crate 布局、子系统清单、规范 tick 管线、跨层通信白名单 |

## 当前优先队列

1. **M0 收尾**：双机 hashrun 验收（用户手动，见 `sessions/2026-08-30-m0-implementation.md`）。其余验收项已过，代码在 `crates/`。
2. **M1 门口：O1 chunk 内活矩形**（`proposals/2026-08-30-noita-derived-optimizations.md`）——回收 Noita 式 sub-chunk 跳过，SyncTest 加第 5 配置执法。
3. **M1 粒子层**：脱格/落格闭环、DDA 碰撞、容量限流（先 brainstorming 出 spec；顺手评估 O3 粉末惯性）。
4. M2 场层与反应表（spec 里裁决 O2 场降本）→ M3 刚体 → M5 时启用 O4 运行时周期哈希（详见总纲 §11 与上述提案）。

## 目录分工

| 目录 | 内容 | 状态 |
|---|---|---|
| `overview/` | 总纲、架构；`architecture.md` 为 Python 原型时代旧版（superseded） | 现行 |
| `CHANGELOG.md` | 工作账本（按产出索引） | 现行 |
| `sessions/` | 会话总账（按时间索引） | 现行 |
| `proposals/` | 改动提案；2026-06 两篇为原型时代产物，部分被总纲取代（见各自 Status 行） | 部分史料 |
| `algorithms/` `materials/` | CA 算法与材质体系文档（Python 原型时代，思想仍有效，代码锚点指向 `archive/`） | 史料为主 |
| `perf/` | 性能基准；Python 基线保留作参照，Rust 基线待 M0 后建立 | 待重建 |
| `reference/` | 外部调研（Noita 深挖、确定性联机调研、EP01 对照等），**仍然有效** | 现行 |
| `superpowers/` | brainstorm specs 与 plans（原型时代产物见各自 Status 行） | 混合 |

## 史料阅读须知

2026-08-29 项目大转向：东方同人横版动作（Python 原型 → Godot C#）→ **1v1 落沙法术对战（Rust 内核 + Godot 表现层）**。此日期之前的文档中，凡与总纲冲突的结论以总纲为准；被显式推翻的裁决列于总纲 §11 翻案记录。Python 原型代码及其 80+ 测试归档于 `archive/prototype-python/`。
