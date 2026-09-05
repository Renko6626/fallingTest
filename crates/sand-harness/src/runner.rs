//! 场景驱动：replay / hashrun / synctest（spec §5.3）。

use std::time::Instant;

use sand_core::{hash, CreatureTable, InitConfig, MaterialTable, ReactionTable, ScanMode, Sim, SpellTable};

use crate::scenario::Scenario;

/// 数据表组：`build_sim`/`run`/`synctest` 三个入口统一收（M4 起四张表，
/// Task 5 之前 harness 只传 `CreatureTable::empty()`/`SpellTable::empty()`，
/// Task 5 接真表时只换调用方实参，本组签名不再改动）。
pub struct Tables<'a> {
    pub materials: &'a MaterialTable,
    pub reactions: &'a ReactionTable,
    pub creatures: &'a CreatureTable,
    pub spells: &'a SpellTable,
}

/// 哈希流采样间隔（golden 口径，改动即口径变更）。
/// 哈希流默认采样间隔（golden 文件的格式由它决定，改动即作废全部 golden）。
/// `hashrun --hash-every N` 可临时改小，用于「逐位回归」取证——例如 Layer G
/// Task 2 的零加速旁路（spec §0 验收第 2 项）要求**每 tick**都对齐，
/// 256 的采样点不足以支撑那句断言。取证专用，不影响 golden 录制默认值。
pub const HASH_EVERY: u64 = 256;

/// 哈希流的取样口径。默认值（`grid_only = false`、`every = HASH_EVERY`）
/// **就是 golden 文件的格式**，改它即作废全部 golden；两个字段都只在取证路径
/// 上才偏离默认。打成一个结构体而非两个形参，是为了让 `run` 的参数表停在
/// clippy 的 7 个上限内，也让"这两件事同属取证口径"在类型上说清楚。
#[derive(Clone, Copy, Debug)]
pub struct HashStream {
    /// 用 `Sim::grid_hash()`（网格哈希树根，跳过粒子层折叠）代替 `state_hash()`。
    pub grid_only: bool,
    /// 采样间隔（tick）。`--hash-every 1` = 逐 tick，用于逐位回归取证。
    pub every: u64,
}

impl Default for HashStream {
    fn default() -> Self {
        HashStream { grid_only: false, every: HASH_EVERY }
    }
}

pub fn build_sim(sc: &Scenario, t: &Tables, threads: usize, scan: ScanMode) -> Result<Sim, String> {
    let cfg = InitConfig {
        width_chunks: sc.world.0,
        height_chunks: sc.world.1,
        seed: sc.seed,
        threads,
        scan,
    };
    let mut sim = Sim::new(&cfg, t.materials.clone(), t.reactions.clone(), t.creatures.clone(), t.spells.clone())?;
    sim.apply_setup(&sc.setup);
    Ok(sim)
}

/// 数据表指纹组（P5 握手语义）：golden 报告头逐行输出。M2 起反应表与材料表
/// 同等待遇（spec §2.4"指纹"条）。`creatures`/`spells`：M4 起随 `Tables` 同步
/// 扩容——**Task 1 里恒 0 且不进 golden 输出行**（两张表恒空，指纹 0 不携带
/// 任何信息；Task 5 接真表时才连指纹一起打印，避免 golden 为一次占位重录
/// 两次）。
#[derive(Clone, Copy, Debug)]
pub struct Fingerprints {
    pub materials: u64,
    pub reactions: u64,
    pub creatures: u64,
    pub spells: u64,
}

pub struct RunReport {
    /// 确定性输出（golden 比对对象）：头部 + 周期哈希 + 终态哈希。
    pub lines: Vec<String>,
    /// 非确定性统计（只进 stderr，不进 golden）。
    pub avg_ms: f64,
    pub max_ms: f64,
}

/// 跑完场景，产出哈希流报告。
///
/// `hs`：哈希流取样口径（见 [`HashStream`]）。`grid_only` 用 `Sim::grid_hash()`
/// （网格哈希树根，跳过粒子层折叠）代替
/// `Sim::state_hash()`。M1 golden 重录取证专用（spec §9 两步程序）——新代码
/// 以 `grid_only=true` 跑旧 golden 场景，逐 tick 序列须与改动前（`state_hash`
/// 即等价于当时的 `grid_hash`，因为彼时尚无粒子）一字不差。
pub fn run(
    sc: &Scenario,
    t: &Tables,
    fps: Fingerprints,
    threads: usize,
    scan: ScanMode,
    ticks: u64,
    hs: HashStream,
) -> Result<RunReport, String> {
    let mut sim = build_sim(sc, t, threads, scan)?;
    let mut lines = vec![
        format!("scenario {}", sc.name),
        format!("scenario_fp {:016x}", sc.fingerprint),
        format!("materials_fp {:016x}", fps.materials),
        format!("reactions_fp {:016x}", fps.reactions),
        format!("world {}x{} seed {} ticks {}", sc.world.0 * 64, sc.world.1 * 64, sc.seed, ticks),
    ];
    let hash_of = |sim: &sand_core::Sim| if hs.grid_only { sim.grid_hash() } else { sim.state_hash() };
    let mut total = 0.0f64;
    let mut max_ms = 0.0f64;
    for tick in 0..ticks {
        let ops = sc.ops_for_tick(tick);
        let t0 = Instant::now();
        sim.step(&ops, sc.inputs_for_tick(tick));
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        total += ms;
        max_ms = max_ms.max(ms);
        if (tick + 1) % hs.every == 0 {
            lines.push(format!("tick {:>8} hash {:016x}", tick + 1, hash_of(&sim)));
        }
    }
    lines.push(format!("final {:016x}", hash_of(&sim)));
    Ok(RunReport { lines, avg_ms: total / ticks.max(1) as f64, max_ms })
}

/// 六配置 SyncTest（O1 spec §3.2）：{1, N 线程} × {Full, ChunkSleep, LiveRect}，
/// 逐 tick 全局哈希比对。分叉即返回 Err（含 tick 与首个不一致 chunk 坐标）。
pub fn synctest(sc: &Scenario, t: &Tables, threads_n: usize, ticks: u64) -> Result<(), String> {
    let configs = [
        (1usize, ScanMode::Full),
        (threads_n, ScanMode::Full),
        (1, ScanMode::ChunkSleep),
        (threads_n, ScanMode::ChunkSleep),
        (1, ScanMode::LiveRect),
        (threads_n, ScanMode::LiveRect),
    ];
    let mut sims = configs
        .iter()
        .map(|&(threads, sk)| build_sim(sc, t, threads, sk))
        .collect::<Result<Vec<_>, _>>()?;
    let t0 = Instant::now();
    for tick in 0..ticks {
        let ops = sc.ops_for_tick(tick);
        let inputs = sc.inputs_for_tick(tick);
        for sim in &mut sims {
            sim.step(&ops, inputs);
        }
        let h0 = sims[0].state_hash();
        // M3 spec §7：每 256 tick 另比对引擎 serde 快照 checksum——刚体层哈希只折
        // 变换/速度/位图，引擎内部（接触缓存等）的分叉靠这条巡检兜底。
        let physics_check = (tick + 1) % 256 == 0;
        let p0 = if physics_check { sims[0].physics_checksum() } else { 0 };
        for (i, sim) in sims.iter().enumerate().skip(1) {
            if physics_check && sim.physics_checksum() != p0 {
                return Err(format!(
                    "tick {tick}: 配置 {:?} 与 {:?} 引擎快照 checksum 分叉（状态哈希未分叉）",
                    configs[i], configs[0]
                ));
            }
            if sim.state_hash() != h0 {
                let (cx, cy) = hash::first_diverging_chunk(sims[0].world(), sim.world())
                    .unwrap_or((usize::MAX, usize::MAX));
                return Err(format!(
                    "tick {tick}: 配置 {:?} 与 {:?} 分叉，首个不一致 chunk = ({cx},{cy})",
                    configs[i], configs[0]
                ));
            }
        }
        if (tick + 1) % 10_000 == 0 {
            eprintln!(
                "  … tick {:>7}/{} 零分叉（{:.1}s）",
                tick + 1,
                ticks,
                t0.elapsed().as_secs_f64()
            );
        }
    }
    Ok(())
}
