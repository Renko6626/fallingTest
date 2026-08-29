//! sand-harness — 无头工具面（architecture §3 工具面表）。
//! 子命令：synctest（多配置逐 tick 哈希比对）、replay（golden 回归）、
//! hashrun（双机人肉 diff 用哈希流）、render（GIF 占位渲染器）。
//!
//! I/O（文件、RON、墙钟计时）都住这里——sand-core 保持零 I/O。

pub mod render;
pub mod runner;
pub mod scenario;
