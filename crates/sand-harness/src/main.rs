//! sand-harness — 无头 CLI 工具面（program-architecture §3 工具面表）。
//! 子命令规划：synctest（双实例逐帧哈希比对）、replay（golden replay 回归）、
//! bench（最坏情况剖析，回填 charter §7）。M0 落地 synctest。

fn main() {
    eprintln!("sand-harness: 骨架阶段，子命令待 M0 实现（synctest / replay / bench）");
    std::process::exit(2);
}
