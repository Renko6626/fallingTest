//! # sand-core — Ring 0 确定性内核（纯库，无 I/O）
//!
//! 真源文档：`docs/overview/kernel-charter.md`（宪法）、
//! `docs/overview/program-architecture.md`（模块与数据流，本 crate 对应其 §3 Ring 0 表）。
//!
//! 铁律（violation = 架构违规，评审一票否决）：
//! - 本 crate 不知道外界存在：无 gdext、无网络、无文件系统、无墙钟。
//!   它唯一的世界就是（状态，输入）。
//! - 世界演化是纯函数 `step(state, inputs)`；`step()` 内部阶段顺序是协议的
//!   一部分（architecture §4），改顺序必须过 charter §11 决策日志。
//! - 网格逻辑纯整数；一切逻辑随机 = hash(tick, x, y, salt/stream) 纯函数族。
//!
//! 模块规划（M0 起逐个落地，见 architecture §3）：
//! state / scheduler / grid / particles / fields / stamp / physics_adapter /
//! entities / rng / events。
