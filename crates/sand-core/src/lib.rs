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

pub mod body;
pub mod cell;
pub mod chunk;
mod dda;
mod emit;
mod explode;
pub mod fixed;
mod geom;
pub mod hash;
pub mod material;
pub mod particle;
mod physics;
pub mod reaction;
pub mod rng;
mod rules;
pub mod scheduler;
mod window;
pub mod world;

pub use body::{Bodies, Body, MAX_BODIES, MAX_REEXTRACT_PER_TICK, MIN_BODY_PIXELS};
pub use cell::{Cell, G_ACCEL, VEL_ONE, V_MAX_CELL};
pub use emit::MAX_EMIT_JITTER_RAW;
pub use fixed::Fx;
pub use material::{
    Category, MaterialDef, MaterialTable, DISPERSION_MAX, MAT_AIR, MAT_WALL,
};
pub use particle::{Particles, MAX_PARTICLES};
pub use reaction::{ReactionRule, ReactionTable};
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

/// 模拟实例门面：World + MaterialTable + 线程池 + 粒子池。
/// harness 与未来的 sand-session 都经此驱动；`world()` 是 Channel A 只读视图的雏形。
pub struct Sim {
    world: World,
    table: MaterialTable,
    reactions: ReactionTable,
    pool: rayon::ThreadPool,
    scan: ScanMode,
    particles: Particles,
    spawn_queue: Vec<world::SpawnRequest>,
    /// M3 刚体层：同步态本体 + 引擎世界（spec §2）。
    bodies: Bodies,
    physics: physics::PhysicsWorld,
}

/// setup 期世代戳：≠ tick 0 的戳（0），保证 setup 内容从 tick 0 起可动（spec §4.4）。
const SETUP_STAMP: u8 = 255;

impl Sim {
    /// `reactions`：M2 起为必传项（`ReactionTable::empty(&table)` 即无反应，
    /// 与 M2 之前行为逐位一致——golden 取证）。
    pub fn new(cfg: &InitConfig, table: MaterialTable, reactions: ReactionTable) -> Result<Sim, String> {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(cfg.threads.max(1))
            .build()
            .map_err(|e| format!("线程池创建失败：{e}"))?;
        Ok(Sim {
            world: World::new(cfg.width_chunks, cfg.height_chunks, cfg.seed),
            table,
            reactions,
            pool,
            scan: cfg.scan,
            particles: Particles::new(),
            spawn_queue: Vec::new(),
            bodies: Bodies::new(),
            physics: physics::PhysicsWorld::new(),
        })
    }

    /// 应用一个输入 op（第 1 步）：`Op::SpawnBody` 路由到刚体层，其余交给 `World`。
    fn apply_one(&mut self, op: &Op, stamp: u8, fseed: u32, op_idx: usize) {
        match *op {
            Op::SpawnBody { material, x, y, w, h } => {
                self.bodies.spawn_rect(&mut self.physics, &self.table, material, x, y, w, h);
            }
            _ => self.world.apply_op(&self.table, op, stamp, fseed, op_idx, &mut self.spawn_queue),
        }
    }

    /// 压入一个粒子生成请求，本 tick `step()` 开头按入队序 drain（M1 spec §4）。
    /// 白名单通信介质：`Op::Emit`（本任务）/ 未来 `Op::Explode` 与测试代码经此
    /// 复用，粒子层本身不关心生产者是谁。
    pub fn queue_spawn(&mut self, material: u8, x: fixed::Fx, y: fixed::Fx, vx: fixed::Fx, vy: fixed::Fx) {
        self.spawn_queue.push(world::SpawnRequest { material, x, y, vx, vy });
    }

    pub fn particles(&self) -> &Particles {
        &self.particles
    }

    /// 场景 setup（仅 tick 0 之前调用）；与脚本 brush 共用同一确定性写入路径。
    /// `Op::Emit` 在 setup 里同样合法（虽然场景通常走 `script`）：用 tick 0
    /// 的 `fseed` 掷骰，产出的生成请求并入 `spawn_queue`，随首个 `step()`
    /// 一并 drain——与 `script` 里的 Emit 走同一条队列、同一套语义。
    pub fn apply_setup(&mut self, ops: &[Op]) {
        assert_eq!(self.world.tick, 0, "setup 只允许在首个 step 之前");
        let fseed = rng::frame_seed(self.world.seed, self.world.tick);
        for (op_idx, op) in ops.iter().enumerate() {
            self.apply_one(op, SETUP_STAMP, fseed, op_idx);
        }
    }

    pub fn step(&mut self, ops: &[Op]) {
        let tick = self.world.tick;
        let stamp = (tick % 256) as u8;
        let fseed = rng::frame_seed(self.world.seed, tick);
        // 1. 输入应用（M3 起从 scheduler::step 纯搬移到此；enumerate 下标 = op 序号，
        //    折进 Op::Emit/Explode 的抖动 salt，区分同 tick 内多个同类 op）。
        for (op_idx, op) in ops.iter().enumerate() {
            self.apply_one(op, stamp, fseed, op_idx);
        }
        // 3. 刚体相（M3 spec §2）：物理步进 → 变换变化者反盖章/盖章（被盖液体/粉末
        //    脱格进 spawn_queue，与 ops 的生成请求同队列、追加序即入队序）。
        //    地形（B′ 按 chunk 缓存）与浮力（水面线阿基米德）在步进前施加；对账（Task 4）随后接线。
        self.bodies.refresh_terrain(&self.world, &self.table, &mut self.physics);
        self.bodies.apply_buoyancy(&self.world, &self.table, &mut self.physics);
        self.physics.step();
        self.bodies.stamp_all(&mut self.world, &self.table, &mut self.physics, stamp, &mut self.spawn_queue);
        // 2. 网格四相 + 封帧
        scheduler::step(&mut self.world, &self.table, &self.reactions, &self.pool, self.scan, &mut self.spawn_queue);

        // 粒子相（M1 spec §4 第 3 步）：a. 生成（drain 入队序 + 容量拒绝，
        // Particles::spawn 内置）——队列此刻已包含测试代码经 queue_spawn
        // 的历史积压 *与* 本 tick ops 阶段里 Op::Emit 刚追加的请求，追加序
        // 即入队序；b/c/d. 并行积分 → 串行提交 → 保序压缩，整体委托
        // particle::advance（Task 4）。stamp 与网格四相同一口径（本 tick 的
        // tick 值，取自四相调度前，与 scheduler::step 内部一致）。
        for req in self.spawn_queue.drain(..) {
            self.particles.spawn(req.material, req.x, req.y, req.vx, req.vy);
        }
        particle::advance(&mut self.particles, &mut self.world, &self.table, &self.pool, stamp);
    }

    pub fn bodies(&self) -> &Bodies {
        &self.bodies
    }

    /// 引擎快照 checksum（SyncTest 巡检，M3 spec §7）。
    pub fn physics_checksum(&self) -> u64 {
        self.physics.checksum()
    }

    pub fn tick(&self) -> u64 {
        self.world.tick
    }

    /// 网格哈希树根（spec §9），单独导出用于 golden 重录取证：证明 Layer G
    /// 在粒子层并入前后逐 tick 位级一致（M1 Task 3 commit 记录了 diff 结果）。
    pub fn grid_hash(&self) -> u64 {
        hash::state_hash(&self.world)
    }

    /// 总哈希 = `combine3(网格哈希树根, 粒子层, 刚体层)`（M1 spec §9 + M3 spec §7）。
    pub fn state_hash(&self) -> u64 {
        hash::combine3(self.grid_hash(), self.particles.hash_into(), self.bodies.hash_into(&self.physics))
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn table(&self) -> &MaterialTable {
        &self.table
    }
}
