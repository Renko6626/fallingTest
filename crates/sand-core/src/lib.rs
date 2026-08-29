//! # sand-core — Ring 0 确定性内核（纯库，无 I/O）
//!
//! 真源文档：`docs/overview/kernel-charter.md`（宪法）、
//! `docs/overview/program-architecture.md`（架构）、
//! `docs/superpowers/specs/2026-08-29-m0-skeleton-design.md`（M0 实现级设计）。
//!
//! 铁律（violation = 架构违规，评审一票否决）：
//! - 本 crate 不知道外界存在：无 gdext、无网络、无文件系统、无墙钟。
//!   它唯一的世界就是（状态，输入）。
//! - 世界演化是纯函数 `step(state, inputs)`；`step()` 内部阶段顺序是协议的
//!   一部分（architecture §4），改顺序必须过 charter §11 决策日志。
//! - 网格逻辑纯整数；一切逻辑随机 = hash(tick, x, y, salt, stream) 纯函数族。

pub mod cell;
pub mod chunk;
pub mod hash;
pub mod material;
pub mod rng;
mod rules;
pub mod scheduler;
mod window;
pub mod world;

pub use cell::Cell;
pub use material::{Category, MaterialDef, MaterialTable, MAT_AIR, MAT_WALL};
pub use world::{Op, World};

/// 初始状态配方（MatchConfig 的 M0 子集）。
/// `threads` 是本地自由参数——只影响快慢，不影响结果（architecture §5）。
#[derive(Clone, Debug)]
pub struct InitConfig {
    pub width_chunks: usize,
    pub height_chunks: usize,
    pub seed: u64,
    pub threads: usize,
    pub sleep_skip: bool,
}

/// 模拟实例门面：World + MaterialTable + 线程池。
/// harness 与未来的 sand-session 都经此驱动；`world()` 是 Channel A 只读视图的雏形。
pub struct Sim {
    world: World,
    table: MaterialTable,
    pool: rayon::ThreadPool,
    sleep_skip: bool,
}

/// setup 期世代戳：≠ tick 0 的戳（0），保证 setup 内容从 tick 0 起可动（spec §4.4）。
const SETUP_STAMP: u8 = 255;

impl Sim {
    pub fn new(cfg: &InitConfig, table: MaterialTable) -> Result<Sim, String> {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(cfg.threads.max(1))
            .build()
            .map_err(|e| format!("线程池创建失败：{e}"))?;
        Ok(Sim {
            world: World::new(cfg.width_chunks, cfg.height_chunks, cfg.seed),
            table,
            pool,
            sleep_skip: cfg.sleep_skip,
        })
    }

    /// 场景 setup（仅 tick 0 之前调用）；与脚本 brush 共用同一确定性写入路径。
    pub fn apply_setup(&mut self, ops: &[Op]) {
        assert_eq!(self.world.tick, 0, "setup 只允许在首个 step 之前");
        for op in ops {
            self.world.apply_op(&self.table, op, SETUP_STAMP);
        }
    }

    pub fn step(&mut self, ops: &[Op]) {
        scheduler::step(&mut self.world, &self.table, &self.pool, self.sleep_skip, ops);
    }

    pub fn tick(&self) -> u64 {
        self.world.tick
    }

    pub fn state_hash(&self) -> u64 {
        hash::state_hash(&self.world)
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn table(&self) -> &MaterialTable {
        &self.table
    }
}
