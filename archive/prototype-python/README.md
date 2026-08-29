# Python 原型（已归档，2026-08-29）

Phase 1 原型，使命已完成：验证了单缓冲 CA、确定性契约 D1–D10（counter RNG / 整数密度 / state_hash / 录放回归）、M0.5 单线程 4-pass chunk 调度器、dispersion rate 与密度沉浮。83 tests passed，性能基线见 `docs/perf/baseline.md`。

项目已转向 Rust 内核（真源：`docs/overview/kernel-charter.md`）。本目录只读留档：

- 算法语义参考（Rust 实现**不做**一对一移植——并行语义与 RNG key 已换代，见总纲 §11 翻案记录）；
- 历史 replay / 测试的可运行版本：`pip install -r requirements.txt && python -m pytest tests/`。
