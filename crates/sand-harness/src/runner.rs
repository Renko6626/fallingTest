//! 场景驱动：replay / hashrun / synctest（spec §5.3）。

use std::time::Instant;

use sand_core::{hash, InitConfig, MaterialTable, ScanMode, Sim};

use crate::scenario::Scenario;

/// 哈希流采样间隔（golden 口径，改动即口径变更）。
pub const HASH_EVERY: u64 = 256;

pub fn build_sim(
    sc: &Scenario,
    table: &MaterialTable,
    threads: usize,
    scan: ScanMode,
) -> Result<Sim, String> {
    let cfg = InitConfig {
        width_chunks: sc.world.0,
        height_chunks: sc.world.1,
        seed: sc.seed,
        threads,
        scan,
    };
    let mut sim = Sim::new(&cfg, table.clone())?;
    sim.apply_setup(&sc.setup);
    Ok(sim)
}

pub struct RunReport {
    /// 确定性输出（golden 比对对象）：头部 + 周期哈希 + 终态哈希。
    pub lines: Vec<String>,
    /// 非确定性统计（只进 stderr，不进 golden）。
    pub avg_ms: f64,
    pub max_ms: f64,
}

/// 跑完场景，产出哈希流报告。
pub fn run(
    sc: &Scenario,
    table: &MaterialTable,
    materials_fp: u64,
    threads: usize,
    scan: ScanMode,
    ticks: u64,
) -> Result<RunReport, String> {
    let mut sim = build_sim(sc, table, threads, scan)?;
    let mut lines = vec![
        format!("scenario {}", sc.name),
        format!("scenario_fp {:016x}", sc.fingerprint),
        format!("materials_fp {materials_fp:016x}"),
        format!("world {}x{} seed {} ticks {}", sc.world.0 * 64, sc.world.1 * 64, sc.seed, ticks),
    ];
    let mut total = 0.0f64;
    let mut max_ms = 0.0f64;
    for t in 0..ticks {
        let ops = sc.ops_for_tick(t);
        let t0 = Instant::now();
        sim.step(&ops);
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        total += ms;
        max_ms = max_ms.max(ms);
        if (t + 1) % HASH_EVERY == 0 {
            lines.push(format!("tick {:>8} hash {:016x}", t + 1, sim.state_hash()));
        }
    }
    lines.push(format!("final {:016x}", sim.state_hash()));
    Ok(RunReport { lines, avg_ms: total / ticks.max(1) as f64, max_ms })
}

/// 六配置 SyncTest（O1 spec §3.2）：{1, N 线程} × {Full, ChunkSleep, LiveRect}，
/// 逐 tick 全局哈希比对。分叉即返回 Err（含 tick 与首个不一致 chunk 坐标）。
pub fn synctest(
    sc: &Scenario,
    table: &MaterialTable,
    threads_n: usize,
    ticks: u64,
) -> Result<(), String> {
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
        .map(|&(t, sk)| build_sim(sc, table, t, sk))
        .collect::<Result<Vec<_>, _>>()?;
    let t0 = Instant::now();
    for tick in 0..ticks {
        let ops = sc.ops_for_tick(tick);
        for sim in &mut sims {
            sim.step(&ops);
        }
        let h0 = sims[0].state_hash();
        for (i, sim) in sims.iter().enumerate().skip(1) {
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
