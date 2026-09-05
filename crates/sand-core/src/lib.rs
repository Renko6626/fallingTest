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
pub mod creature;
mod dda;
mod emit;
mod explode;
pub mod fixed;
mod geom;
pub mod hash;
pub mod input;
pub mod material;
pub mod particle;
mod physics;
pub mod projectile;
pub mod reaction;
pub mod rng;
mod rules;
pub mod scheduler;
pub(crate) mod sin_table;
pub mod spell;
mod window;
pub mod world;

pub use body::{Bodies, Body, MAX_BODIES, MAX_REEXTRACT_PER_TICK, MIN_BODY_PIXELS};
pub use cell::{Cell, G_ACCEL, VEL_ONE, V_MAX_CELL};
pub use creature::{Creature, CreatureTable, CreatureTpl, Creatures, MAX_CREATURES};
pub use emit::MAX_EMIT_JITTER_RAW;
pub use fixed::Fx;
pub use input::{InputFrame, MAX_SLOTS};
pub use material::{
    Category, MaterialDef, MaterialTable, DISPERSION_MAX, MAT_AIR, MAT_WALL,
};
pub use particle::{Particles, MAX_PARTICLES};
pub use projectile::{Projectiles, MAX_PROJECTILES};
pub use reaction::{ReactionRule, ReactionTable};
pub use spell::{SpellDef, SpellKind, SpellTable};
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
    /// M4 实体层：模板表（加载期构造、只读）+ 运行时状态（Task 2 起生物表接
    /// 运动学，Task 4 起弹体表接直线飞行 + 命中判定）。
    creature_table: creature::CreatureTable,
    spell_table: spell::SpellTable,
    creatures: creature::Creatures,
    projectiles: projectile::Projectiles,
}

/// setup 期世代戳：≠ tick 0 的戳（0），保证 setup 内容从 tick 0 起可动（spec §4.4）。
const SETUP_STAMP: u8 = 255;

/// [`Sim::body_state`] 的返回：`((x, y, angle), ((vx, vy), angvel), sleeping)`。
pub type BodyState = ((f32, f32, f32), ((f32, f32), f32), bool);

impl Sim {
    /// `reactions`：M2 起为必传项（`ReactionTable::empty(&table)` 即无反应，
    /// 与 M2 之前行为逐位一致——golden 取证）。`creature_table`/`spell_table`：
    /// M4 起必传项（`CreatureTable::empty()`/`SpellTable::empty()` 即无生物
    /// 无法术，Task 1 之前行为逐位一致——golden 取证，见 `--grid-only` 程序）。
    pub fn new(
        cfg: &InitConfig,
        table: MaterialTable,
        reactions: ReactionTable,
        creature_table: creature::CreatureTable,
        spell_table: spell::SpellTable,
    ) -> Result<Sim, String> {
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
            creature_table,
            spell_table,
            creatures: creature::Creatures::new(),
            projectiles: projectile::Projectiles::new(),
        })
    }

    /// 应用一个输入 op（第 1 步）：`Op::SpawnBody` 路由到刚体层，`Op::SpawnCreature`
    /// 路由到生物层，`Op::Explode` 网格与刚体两侧都做，其余交给 `World`。
    fn apply_one(&mut self, op: &Op, stamp: u8, fseed: u32, op_idx: usize) {
        match *op {
            Op::SpawnBody { material, x, y, w, h, angle_deg } => {
                self.bodies.spawn_rect(&mut self.physics, &self.table, material, x, y, w, h, angle_deg);
            }
            Op::SpawnCreature { x, y, template, team, controller, loadout } => {
                self.creatures.spawn(&self.creature_table, template, x, y, team, controller, loadout);
            }
            Op::Explode { x, y, r, .. } => {
                // 网格侧：射线删格、碎屑成粒子；刚体侧：记下，第 7 步重提取后再给冲量（决策记录第 17 条）。
                self.world.apply_op(&self.table, op, stamp, fseed, op_idx, &mut self.spawn_queue);
                self.bodies.pending_blasts.push((x, y, r));
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

    /// 压入一个弹体生成请求，直接落地（不像 `queue_spawn` 那样过队列——弹体
    /// 没有粒子池那种"本 tick 统一 drain"的语义要求，`Projectiles::spawn`
    /// 本身就是幂等的确定性写入）。`pub`，文档标注供测试与诊断工具；生产
    /// 路径是 `spell::cast_all`（M4 Task 5 起），它直接调用同一个
    /// `Projectiles::spawn`（不经本方法，因为它已经持有 `&mut Creatures`/
    /// `&mut Projectiles`，没有理由再绕一层 `Sim`）——测试注入与产品代码
    /// 走同一底层入口，不是另开一条平行通路。
    ///
    /// `life`/`energy`/`grace`/`bounces` 四个字段从 `spell_table` 取（法术
    /// 定义了一颗弹该活多久、能打穿多硬的东西、防自伤宽限多长、能弹几次），
    /// 调用方只给出射点与初速——与 brief Interfaces 一节"其余字段从法术表
    /// 取"一致。`spell` 越界即调用方漏配置，与 `SpellTable::get` 同一体例，
    /// 不做脏值防御。
    ///
    /// **`SpellKind::Spray` 显式拒绝、返回 `false`**（M4 Task 5 评审
    /// Important，2026-09-06）：`Spray` 语义上不产生弹体（`cast_all` 直接
    /// 走 `emit::apply_emit`），若在此仍把它塞进 `Projectiles`，下一 tick
    /// `Projectiles::advance` 命中判定会走到 `resolve_hit` 的
    /// `SpellKind::Spray => unreachable!()` 分支直接 panic——而且
    /// `unreachable!()` 在 release 构建同样触发，不像 `debug_assert!` 只在
    /// debug 生效。本方法是 `pub` 且不校验 `kind` 的外部入口（与
    /// `world.rs::apply_op` 内部用 `unreachable!()` 处理
    /// `Op::SpawnBody`/`Op::SpawnCreature` 不同——那两个变体被 `apply_one`
    /// 的私有路由截走，外部根本无法直接触发，风险类别不一样）；不变量必须
    /// 守在这个入口，不能指望结算远端兜底。复用已有的 `bool` 返回值
    /// （容量拒绝同一个信号通道）表达"没有产出弹体"，调用方不需要区分
    /// 原因、不需要改签名——语义上与"这个法术类型压根不产生弹体"完全自洽。
    pub fn queue_projectile(
        &mut self,
        spell: u8,
        x: fixed::Fx,
        y: fixed::Fx,
        vx: fixed::Fx,
        vy: fixed::Fx,
        owner: u8,
    ) -> bool {
        let s = self.spell_table.get(spell);
        if matches!(s.kind, SpellKind::Spray { .. }) {
            return false;
        }
        self.projectiles.spawn(spell, x, y, vx, vy, s.life, s.dig_power, owner, s.grace, s.bounces)
    }

    pub fn particles(&self) -> &Particles {
        &self.particles
    }

    /// 弹体表只读视图（M4 Task 4）：行为测试经此读飞行结果。
    pub fn projectiles(&self) -> &Projectiles {
        &self.projectiles
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

    /// `inputs`：本 tick 生效的玩家意图，按 controller 序号索引（spec §3.1）。
    /// Task 2 起接线其中的 2a+2b（输入应用 + 生物运动学）；Task 4 起接线 2c
    /// （弹体积分）；Task 5 起接线 2d（施法结算）。
    pub fn step(&mut self, ops: &[Op], inputs: &[InputFrame]) {
        let tick = self.world.tick;
        let stamp = (tick % 256) as u8;
        let fseed = rng::frame_seed(self.world.seed, tick);
        // 1. 输入应用（M3 起从 scheduler::step 纯搬移到此；enumerate 下标 = op 序号，
        //    折进 Op::Emit/Explode 的抖动 salt，区分同 tick 内多个同类 op）。
        for (op_idx, op) in ops.iter().enumerate() {
            self.apply_one(op, stamp, fseed, op_idx);
        }
        // 2. 实体与法术（架构 §4，M4 起生效）：2a+2b 生物相——输入应用 + 运动学，
        //    读本 tick 起始网格（与刚体相(3)、网格四相(4)所见的网格状态一致，
        //    spec §1.1 定序理由）；紧接着 2b 后半（Task 3 起）——排开液体/粉末、
        //    游泳、材质接触伤害与 HP 墓碑（spec §4.3–§4.5），复用 ops 阶段同一个
        //    `spawn_queue`，本 tick 粒子相（第 5 步）按追加序统一 drain。
        //    2c 弹体积分（Task 4 起接线）：读本 tick 已移动的生物位置（spec §1.1
        //    "弹体命中的是生物本 tick 移动后的位置"）。
        self.creatures.step_kinematics(&self.world, &self.table, &self.creature_table, inputs);
        self.creatures.step_world_interaction(
            &mut self.world,
            &self.table,
            &self.creature_table,
            inputs,
            stamp,
            &mut self.spawn_queue,
        );
        self.projectiles.advance(
            &mut self.world,
            &self.table,
            &self.spell_table,
            &mut self.creatures,
            &mut self.bodies,
            &mut self.physics,
            stamp,
            fseed,
            &mut self.spawn_queue,
        );
        // 2d 施法结算（Task 5 起接线，spec §6）：读本 tick 已积分完的弹体池
        // （新弹体本 tick 不积分，下 tick 起飞——与 2c 的时序天然一致，不需要
        // 额外规则）。不碰 world/table/bodies（spell::cast_all 文档：`Bolt`/
        // `Blast` 只落弹体，命中结算留在 2c 的 `Projectiles::advance` 里；
        // `Spray` 只读发射点、写 spawn_queue）。
        spell::cast_all(
            &mut self.creatures,
            &mut self.projectiles,
            &self.spell_table,
            &self.creature_table,
            inputs,
            fseed,
            &mut self.spawn_queue,
        );
        // 3. 刚体相（M3 spec §2）：物理步进 → 变换变化者反盖章/盖章（被盖液体/粉末
        //    脱格进 spawn_queue，与 ops 的生成请求同队列、追加序即入队序）。
        //    地形（B′ 按 chunk 缓存）与浮力（水面线阿基米德）在步进前施加。
        self.bodies.refresh_terrain(&self.world, &self.table, &mut self.physics);
        self.bodies.apply_buoyancy(&self.world, &self.table, &mut self.physics);
        self.physics.step();
        self.bodies.stamp_all(&mut self.world, &self.table, &mut self.physics, stamp, &mut self.spawn_queue);
        // 4. 网格四相 + 封帧（M0–M2，不变）。
        scheduler::step(&mut self.world, &self.table, &self.reactions, &self.pool, self.scan, &mut self.spawn_queue);

        // 5. 粒子相（M1 spec §4）：a. 生成（drain 入队序 + 容量拒绝，
        // Particles::spawn 内置）——队列此刻已包含测试代码经 queue_spawn
        // 的历史积压 *与* 本 tick ops 阶段里 Op::Emit 刚追加的请求，追加序
        // 即入队序；b/c/d. 并行积分 → 串行提交 → 保序压缩，整体委托
        // particle::advance（Task 4）。stamp 与网格四相同一口径（本 tick 的
        // tick 值，取自四相调度前，与 scheduler::step 内部一致）。
        for req in self.spawn_queue.drain(..) {
            self.particles.spawn(req.material, req.x, req.y, req.vx, req.vy);
        }
        particle::advance(&mut self.particles, &mut self.world, &self.table, &self.pool, stamp);
        // 7. 刚体对账 + 限额重提取（M3 spec §6）：爆炸/燃烧毁掉的盖章格 → 位图更新 →
        //    分量分解；碎片脱格进 spawn_queue（下一 tick 粒子相 drain）。
        //    （6 号在架构 §4 里已删，不重用，见 lib.rs 头注 / CLAUDE.md 惯例。）
        self.bodies.reconcile(&self.world);
        self.bodies.reextract(&mut self.world, &self.table, &mut self.physics, stamp, &mut self.spawn_queue);
        // 7'. 本 tick 爆炸的刚体冲量（切开之后各半各自受力；入队序）。
        let blasts = std::mem::take(&mut self.bodies.pending_blasts);
        for (x, y, r) in blasts {
            self.bodies.apply_blast(&mut self.physics, x, y, r);
        }
    }

    pub fn bodies(&self) -> &Bodies {
        &self.bodies
    }

    /// 只读诊断视图：刚体 `id` 的 `(x, y, angle)`、`((vx, vy), angvel)`、是否睡眠。
    /// 供行为测试/探针断言姿态与角速度；不进哈希路径。
    pub fn body_state(&self, id: u16) -> Option<BodyState> {
        let b = self.bodies.get(id)?;
        Some((self.physics.transform(b.handle), self.physics.velocity(b.handle), self.physics.is_sleeping(b.handle)))
    }

    /// 引擎快照 checksum（SyncTest 巡检，M3 spec §7）。
    pub fn physics_checksum(&self) -> u64 {
        self.physics.checksum()
    }

    /// 引擎整体快照（serde/bincode，M3 spec §7；M6 rollback 决策门的依据）。
    pub fn physics_snapshot(&self) -> Vec<u8> {
        self.physics.snapshot()
    }

    /// 从快照恢复引擎。M3 只验"恢复后续跑与不恢复逐位相同"（验收 4）；网格/粒子/
    /// 刚体位图的快照是 M6 的事。
    pub fn restore_physics(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.physics.restore(bytes)
    }

    pub fn tick(&self) -> u64 {
        self.world.tick
    }

    /// 网格哈希树根（spec §9），单独导出用于 golden 重录取证：证明 Layer G
    /// 在粒子层并入前后逐 tick 位级一致（M1 Task 3 commit 记录了 diff 结果）。
    pub fn grid_hash(&self) -> u64 {
        hash::state_hash(&self.world)
    }

    /// 总哈希 = `combine4(网格哈希树根, 粒子层, 刚体层, 实体层)`（M1 spec §9 +
    /// M3 spec §7 + M4 spec §1.3）。无生物/弹体的场景实体层仍恒 0
    /// （`Creatures`/`Projectiles::hash_into` 空表早退），与 M4 之前的
    /// `combine3` 输出**不**逐位相同——这是 Task 1 golden 重录一次的唯一
    /// 刻意哈希结构变更（`--grid-only` 取证网格哈希流本身不受影响）；
    /// Task 2 起生物表可非空，实体层随之变化，但既有（无生物）场景不受影响。
    pub fn state_hash(&self) -> u64 {
        hash::combine4(
            self.grid_hash(),
            self.particles.hash_into(),
            self.bodies.hash_into(&self.physics),
            self.entity_hash(),
        )
    }

    /// 实体层哈希 = 生物 + 弹体（两者都为空时恒 0）。
    fn entity_hash(&self) -> u64 {
        let mut h = xxhash_rust::xxh3::Xxh3::new();
        h.update(&self.creatures.hash_into().to_le_bytes());
        h.update(&self.projectiles.hash_into().to_le_bytes());
        h.digest()
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn table(&self) -> &MaterialTable {
        &self.table
    }

    /// 生物模板表只读视图（加载期构造；空表还是有模板由调用方经 `Sim::new`
    /// 决定，`CreatureTable::empty()` 或 `default_player()`/`from_tpls()`）。
    pub fn creature_table(&self) -> &creature::CreatureTable {
        &self.creature_table
    }

    /// 法术表只读视图（加载期构造，Task 5 起可非空）。
    pub fn spell_table(&self) -> &spell::SpellTable {
        &self.spell_table
    }

    /// 测试专用：按名字查法术 id（`SpellTable::id_by_name` 的便捷包装，
    /// brief Interfaces 一节点名）。查不到即测试自身配置错误，直接 panic
    /// ——与 `CreatureTable::get`/`SpellTable::get` 越界即 panic 同一体例，
    /// 生产路径不该依赖名字查找。
    pub fn spell_id(&self, name: &str) -> u8 {
        self.spell_table
            .id_by_name(name)
            .unwrap_or_else(|| panic!("测试法术表没有名为 '{name}' 的法术"))
    }

    /// 生物表只读视图（M4 Task 2）：行为测试经此读运动学结果。
    pub fn creatures(&self) -> &creature::Creatures {
        &self.creatures
    }

    /// 生物表可写视图。与既有 `Sim::queue_spawn` 同体例：`pub`，仅供测试与诊断
    /// （`Creatures::set_hp`/`set_mana` 走这里；生产路径一律经 `Op::SpawnCreature`）。
    pub fn creatures_mut(&mut self) -> &mut creature::Creatures {
        &mut self.creatures
    }
}
