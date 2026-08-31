//! sand-harness CLI（spec §5.3）。
//! 用法：
//!   sand-harness synctest <scenario.ron> [--ticks N] [--threads N] [--materials PATH]
//!   sand-harness replay   <scenario.ron> [--golden PATH | --write-golden PATH] [--ticks N] [--grid-only]
//!   sand-harness hashrun  <scenario.ron> [--ticks N] [--grid-only] [--hash-every N]
//!   sand-harness render   <scenario.ron> -o out.gif [--every K] [--scale N] [--ticks N] [--fps F] [--from T]
//!
//! `--grid-only`：哈希流用网格哈希树根（跳过粒子层折叠），M1 golden 重录取证专用
//! （spec §9）——证明粒子层并入前后 Layer G 逐 tick 哈希位级一致。

use std::process::ExitCode;

use sand_harness::render::{render_gif, RenderOpts};
use sand_harness::runner;
use sand_harness::scenario::{load_materials, load_scenario};

struct Args {
    cmd: String,
    scenario: String,
    materials: String,
    ticks: Option<u64>,
    threads: usize,
    golden: Option<String>,
    write_golden: Option<String>,
    out: Option<String>,
    every: u64,
    scale: usize,
    fps: Option<u32>,
    from: u64,
    scan: sand_core::ScanMode,
    grid_only: bool,
    hash_every: u64,
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().ok_or("缺少子命令（synctest/replay/hashrun/render）")?;
    let scenario = it.next().ok_or("缺少场景文件路径")?;
    let mut a = Args {
        cmd,
        scenario,
        materials: "data/materials.ron".into(),
        ticks: None,
        threads: std::thread::available_parallelism().map(|n| n.get().min(8)).unwrap_or(4),
        golden: None,
        write_golden: None,
        out: None,
        every: 4,
        scale: 4,
        fps: None,
        from: 0,
        scan: sand_core::ScanMode::LiveRect,
        grid_only: false,
        hash_every: sand_harness::runner::HASH_EVERY,
    };
    while let Some(flag) = it.next() {
        let mut val = || it.next().ok_or(format!("{flag} 缺参数"));
        match flag.as_str() {
            "--materials" => a.materials = val()?,
            "--ticks" => a.ticks = Some(val()?.parse().map_err(|e| format!("--ticks: {e}"))?),
            "--threads" => a.threads = val()?.parse().map_err(|e| format!("--threads: {e}"))?,
            "--golden" => a.golden = Some(val()?),
            "--write-golden" => a.write_golden = Some(val()?),
            "-o" => a.out = Some(val()?),
            "--every" => a.every = val()?.parse().map_err(|e| format!("--every: {e}"))?,
            "--scale" => a.scale = val()?.parse().map_err(|e| format!("--scale: {e}"))?,
            "--fps" => a.fps = Some(val()?.parse().map_err(|e| format!("--fps: {e}"))?),
            "--from" => a.from = val()?.parse().map_err(|e| format!("--from: {e}"))?,
            "--grid-only" => a.grid_only = true,
            // 取证专用：把哈希流采样间隔改小（默认 256 = golden 格式）。
            "--hash-every" => {
                a.hash_every = val()?.parse().map_err(|e| format!("--hash-every: {e}"))?;
                if a.hash_every == 0 {
                    return Err("--hash-every 必须 >= 1".into());
                }
            }
            "--scan" => {
                a.scan = match val()?.as_str() {
                    "full" => sand_core::ScanMode::Full,
                    "sleep" => sand_core::ScanMode::ChunkSleep,
                    "live" => sand_core::ScanMode::LiveRect,
                    other => return Err(format!("--scan: 未知模式 {other}（full/sleep/live）")),
                }
            }
            other => return Err(format!("未知参数 {other}")),
        }
    }
    Ok(a)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sand-harness: {e}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    // 取证 feature 的可见性闸门（Layer G Task 2，spec §0 验收第 2 项）：
    // `sand-core/zero-gravity` 把重力压成 0，只用于「零加速旁路」逐位回归。
    // 它改变物理 ⇒ 两端 feature 不一致即分叉，而握手指纹只覆盖数据、覆盖不到
    // 代码，故这里在**运行时**直接看常量本身——手改常量同样会被这条逮到。
    if sand_core::G_ACCEL == 0 {
        eprintln!(
            "警告：G_ACCEL == 0（零加速旁路构建）。仅供 Layer G Task 2 取证，\
             产出的哈希与 golden **不代表**产品语义，切勿据此录 golden。"
        );
    }
    let a = parse_args()?;
    let (table, materials_fp) = load_materials(&a.materials)?;
    let sc = load_scenario(&a.scenario, &table)?;
    let ticks = a.ticks.unwrap_or(sc.ticks);

    match a.cmd.as_str() {
        "synctest" => {
            eprintln!(
                "SyncTest：{}（{}x{}）× {ticks} tick，六配置（1/{} 线程 × Full/ChunkSleep/LiveRect）",
                sc.name,
                sc.world.0 * 64,
                sc.world.1 * 64,
                a.threads
            );
            runner::synctest(&sc, &table, a.threads, ticks)?;
            println!("SyncTest 通过：{ticks} tick 零分叉（scenario_fp {:016x}）", sc.fingerprint);
        }
        "replay" | "hashrun" => {
            let report =
                runner::run(
                    &sc,
                    &table,
                    materials_fp,
                    a.threads,
                    a.scan,
                    ticks,
                    runner::HashStream { grid_only: a.grid_only, every: a.hash_every },
                )?;
            let text = report.lines.join("\n") + "\n";
            if let Some(path) = &a.write_golden {
                std::fs::write(path, &text).map_err(|e| format!("写 {path} 失败：{e}"))?;
                eprintln!("golden 已写入 {path}");
            }
            print!("{text}");
            eprintln!("tick 耗时 avg {:.3}ms / max {:.3}ms", report.avg_ms, report.max_ms);
            if let Some(path) = &a.golden {
                let want = std::fs::read_to_string(path).map_err(|e| format!("读 {path} 失败：{e}"))?;
                if want != text {
                    return Err(format!("golden 比对失败：输出与 {path} 不一致"));
                }
                println!("golden 比对通过：{path}");
            }
        }
        "render" => {
            let out = a.out.ok_or("render 需要 -o 输出路径")?;
            let mut sim = runner::build_sim(&sc, &table, a.threads, sand_core::ScanMode::LiveRect)?;
            let opts =
                RenderOpts { every: a.every.max(1), scale: a.scale.max(1), fps: a.fps, from: a.from, out };
            let frames = render_gif(&sc, &table, &mut sim, ticks, &opts)?;
            println!(
                "已渲染 {frames} 帧 → {}（{}x{} ×{}，每 {} tick 一帧）",
                opts.out,
                sc.world.0 * 64,
                sc.world.1 * 64,
                opts.scale,
                opts.every
            );
        }
        other => return Err(format!("未知子命令 {other}")),
    }
    Ok(())
}
