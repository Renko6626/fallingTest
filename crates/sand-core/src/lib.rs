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
mod dda;
pub mod fixed;
pub mod hash;
pub mod material;
pub mod particle;
pub mod rng;
mod rules;
pub mod scheduler;
mod window;
pub mod world;

pub use cell::Cell;
pub use fixed::Fx;
pub use material::{Category, MaterialDef, MaterialTable, MAT_AIR, MAT_WALL};
pub use particle::{Particles, MAX_PARTICLES};
pub use world::{Op, World};

/// 扫描模式（O1 spec §2.1）。三种模式**语义逐位等价**（SyncTest 六配置执法）；
/// LiveRect 为运行默认，Full/ChunkSleep 是执法对照配置。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScanMode {
    /// 全部 chunk 全量扫描（参照语义）。
    Full,
    /// chunk 级休眠 + 相位边界唤醒，活跃 chunk 全量扫描（M0 语义）。
    ChunkSleep,
    /// chunk 级休眠 + 起始矩形 = dirty ∪ next_dirty 快照 + 扫描中活扩张。
    LiveRect,
}

/// 初始状态配方（MatchConfig 的 M0 子集）。
/// `threads` 与 `scan` 都是本地自由参数——只影响快慢，不影响结果（architecture §5）。
#[derive(Clone, Debug)]
pub struct InitConfig {
    pub width_chunks: usize,
    pub height_chunks: usize,
    pub seed: u64,
    pub threads: usize,
    pub scan: ScanMode,
}

/// 生成队列条目（M1 spec §4 第 3 步 a）：`Op::Explode` / `Op::Emit`（Task 4/5）
/// 与测试代码经 [`Sim::queue_spawn`] 压入，本 tick 粒子相开头按入队序 drain。
#[derive(Clone, Copy, Debug)]
struct SpawnRequest {
    material: u8,
    x: fixed::Fx,
    y: fixed::Fx,
    vx: fixed::Fx,
    vy: fixed::Fx,
}

/// 模拟实例门面：World + MaterialTable + 线程池 + 粒子池。
/// harness 与未来的 sand-session 都经此驱动；`world()` 是 Channel A 只读视图的雏形。
pub struct Sim {
    world: World,
    table: MaterialTable,
    pool: rayon::ThreadPool,
    scan: ScanMode,
    particles: Particles,
    spawn_queue: Vec<SpawnRequest>,
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
            scan: cfg.scan,
            particles: Particles::new(),
            spawn_queue: Vec::new(),
        })
    }

    /// 压入一个粒子生成请求，本 tick `step()` 开头按入队序 drain（M1 spec §4）。
    /// 白名单通信介质：Task 4 的 `Op::Explode`/`Op::Emit` 与测试代码经此复用，
    /// 粒子层本身不关心生产者是谁。
    pub fn queue_spawn(&mut self, material: u8, x: fixed::Fx, y: fixed::Fx, vx: fixed::Fx, vy: fixed::Fx) {
        self.spawn_queue.push(SpawnRequest { material, x, y, vx, vy });
    }

    pub fn particles(&self) -> &Particles {
        &self.particles
    }

    /// 场景 setup（仅 tick 0 之前调用）；与脚本 brush 共用同一确定性写入路径。
    pub fn apply_setup(&mut self, ops: &[Op]) {
        assert_eq!(self.world.tick, 0, "setup 只允许在首个 step 之前");
        for op in ops {
            self.world.apply_op(&self.table, op, SETUP_STAMP);
        }
    }

    pub fn step(&mut self, ops: &[Op]) {
        let tick = self.world.tick;
        scheduler::step(&mut self.world, &self.table, &self.pool, self.scan, ops);

        // 粒子相（M1 spec §4 第 3 步）：a. 生成（drain 入队序 + 容量拒绝，
        // Particles::spawn 内置）；b/c/d. 并行积分 → 串行提交 → 保序压缩，
        // 整体委托 particle::advance（Task 4）。stamp 与网格四相同一口径
        // （本 tick 的 tick 值，取自四相调度前，与 scheduler::step 内部一致）。
        for req in self.spawn_queue.drain(..) {
            self.particles.spawn(req.material, req.x, req.y, req.vx, req.vy);
        }
        let stamp = (tick % 256) as u8;
        particle::advance(&mut self.particles, &mut self.world, &self.table, &self.pool, stamp);
    }

    pub fn tick(&self) -> u64 {
        self.world.tick
    }

    /// 网格哈希树根（spec §9），单独导出用于 golden 重录取证：证明 Layer G
    /// 在粒子层并入前后逐 tick 位级一致（M1 Task 3 commit 记录了 diff 结果）。
    pub fn grid_hash(&self) -> u64 {
        hash::state_hash(&self.world)
    }

    /// 总哈希 = `combine(网格哈希树根, 粒子层哈希)`（spec §9）。
    pub fn state_hash(&self) -> u64 {
        hash::combine(self.grid_hash(), self.particles.hash_into())
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn table(&self) -> &MaterialTable {
        &self.table
    }
}
