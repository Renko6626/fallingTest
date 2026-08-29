# 会话总账：2026-08-29 · 项目大转向（Rust 内核）

> 文档路径：`docs/sessions/2026-08-29-rust-pivot.md`
> 最近更新：2026-08-29 (UTC+8)

## 背景

用户在根目录放入两份自写文档（原 `project-new.md` / `structure-new.md`，2026-08-15 起草），宣布大转向：**1v1 落沙法术对战，Rust 内核 + Godot 4/gdext 表现层，全栈确定性 lockstep**。旧 Python 原型（Phase 1，83 tests、M0/M0.5 完成）使命结束。

## 本次会话做了什么

1. **归档**：`prototype/` → `archive/prototype-python/` + README 定性（只读；算法语义参考，不一对一移植）。commit `f5f2371`。
2. **文档入档**：两份新文档 → `docs/overview/kernel-charter.md` + `program-architecture.md`（补规范头部、修互引文件名 `kernel-charter-v0.1.md` → `kernel-charter.md`）。
3. **翻案留痕**：会话中识别出 4 处与原型时代裁决的冲突，经用户确认为有意翻案，写入总纲 §11 翻案记录：刚体入 lockstep（推翻 R3-A）、温度场回归（推翻 fire spec v2 降级裁决）、四相棋盘 r≤16（替代正方形写域）、RNG key 收敛（pass_id/attempt 并入 stream，实现时必须保留）。对应 4 篇旧文档加 Superseded 标注。commit `f97940e`。
4. **CLAUDE.md 重写**：薄化，真源指向总纲；新增确定性红线速查。commit `f218eb6`。
5. **Rust 骨架**：workspace + `sand-core`（Ring 0 纯库 stub）+ `sand-harness`（CLI stub）+ clippy disallowed_types 执法（红绿验证）。`cargo clippy` / `cargo test` 绿。commit `453eec0`。
6. `docs/README.md` 首次补建（导航 + 优先队列）。

## 没做 / 留给后续

- **M0 实施未开始**：chunk 寻址存储、四相调度器、沙/水材质、SyncTest 双实例框架。动手前按 CLAUDE.md §2.3 走 `superpowers:brainstorming` 出实现级 spec（总纲是宪法不是实现设计）。
- `sand-session` / `sand-bridge` / `data/`（RON）/ `godot/` 目录：到各自里程碑再建。
- fire spec v2 的重审（M2 前）；`docs/perf/` Rust 基线（M0 后由 harness-bench 建立）。
- 本机 gdext 编译链未验证（Linux 无 sudo；Windows 交叉编译走 cargo-xwin，见全局 CLAUDE.md）——到 sand-bridge 落地时再处理。

## 下一步入口

新会话开局：读本文件 + `docs/CHANGELOG.md` 顶部 + `docs/overview/kernel-charter.md`。当前队首 = **M0 骨架与执法**（验收：双机 10 万 tick 零分叉）。
