//! 占位 GIF 渲染器（spec §6）。表现层：只消费 `Sim::world()` 只读视图
//! （Channel A 雏形），调色板来自 materials.ron 的 color 字段。
//! 不进确定性执法范围；渲染器变化不影响任何哈希。

use std::fs::File;

use sand_core::{MaterialTable, Sim};

use crate::scenario::Scenario;

pub struct RenderOpts {
    pub every: u64,
    pub scale: usize,
    /// 回放帧率覆盖（帧/秒）。None = 墙钟等速回放，但延迟封顶 MAX_DELAY_CS
    /// （稀疏采样时自动退化为延时摄影，避免 --every 100 变 1.67 秒/帧的幻灯片）。
    pub fps: Option<u32>,
    pub out: String,
}

/// GIF 单位 = 1/100 秒；多数播放器对 <2cs 的延迟按 10cs 处理，故下限 2。
const MIN_DELAY_CS: u16 = 2;
const MAX_DELAY_CS: u16 = 10;

/// 帧延迟（centiseconds）。60Hz 模拟，每 every tick 采一帧。
fn frame_delay(every: u64, fps: Option<u32>) -> u16 {
    match fps {
        Some(f) => ((100.0 / f.max(1) as f64).round() as u16).max(MIN_DELAY_CS),
        None => ((every as f64 * 100.0 / 60.0).round() as u16)
            .clamp(MIN_DELAY_CS, MAX_DELAY_CS),
    }
}

pub fn render_gif(
    sc: &Scenario,
    table: &MaterialTable,
    sim: &mut Sim,
    ticks: u64,
    opts: &RenderOpts,
) -> Result<usize, String> {
    let w = sc.world.0 * 64;
    let h = sc.world.1 * 64;
    let (ow, oh) = (w * opts.scale, h * opts.scale);
    if ow > u16::MAX as usize || oh > u16::MAX as usize {
        return Err(format!("输出尺寸 {ow}x{oh} 超过 GIF 上限"));
    }

    // 调色板：index = material id；空位填黑
    let mut palette = vec![0u8; 256 * 3];
    for id in 0..table.len() {
        let (r, g, b) = table.color(id as u8);
        palette[id * 3] = r;
        palette[id * 3 + 1] = g;
        palette[id * 3 + 2] = b;
    }

    let mut file = File::create(&opts.out).map_err(|e| format!("建 {} 失败：{e}", opts.out))?;
    let mut enc = gif::Encoder::new(&mut file, ow as u16, oh as u16, &palette)
        .map_err(|e| format!("GIF 编码器：{e}"))?;
    enc.set_repeat(gif::Repeat::Infinite).map_err(|e| format!("GIF repeat：{e}"))?;
    let delay = frame_delay(opts.every, opts.fps);

    let mut frames = 0usize;
    let mut buf = vec![0u8; ow * oh];
    for t in 0..ticks {
        sim.step(&sc.ops_for_tick(t));
        if t % opts.every == 0 || t + 1 == ticks {
            fill_frame(sim, w, h, opts.scale, &mut buf);
            draw_particles(sim, w, h, opts.scale, &mut buf);
            let frame = gif::Frame {
                width: ow as u16,
                height: oh as u16,
                buffer: std::borrow::Cow::Borrowed(&buf),
                delay,
                ..Default::default()
            };
            enc.write_frame(&frame).map_err(|e| format!("写帧失败：{e}"))?;
            frames += 1;
        }
    }
    Ok(frames)
}

/// 粒子层目检叠加：Layer P 每颗粒子按当前格坐标画一个 scale×scale 像素块
/// （同 fill_frame 的最近邻放大规则），材质色沿用调色板。只读消费
/// `sim.particles()`（Channel A 雏形），不影响任何哈希——纯渲染面。
fn draw_particles(sim: &Sim, w: usize, h: usize, scale: usize, buf: &mut [u8]) {
    let ow = w * scale;
    let particles = sim.particles();
    for i in 0..particles.len() {
        let cx = particles.x(i).to_cell();
        let cy = particles.y(i).to_cell();
        if cx < 0 || cy < 0 || cx as usize >= w || cy as usize >= h {
            continue;
        }
        let (cx, cy) = (cx as usize, cy as usize);
        let id = particles.material(i);
        for sy in 0..scale {
            let row_start = (cy * scale + sy) * ow;
            for sx in 0..scale {
                buf[row_start + cx * scale + sx] = id;
            }
        }
    }
}

fn fill_frame(sim: &Sim, w: usize, h: usize, scale: usize, buf: &mut [u8]) {
    let ow = w * scale;
    for y in 0..h {
        // 先铺一行像素，再整行复制 scale 次（最近邻整数放大）
        let row_start = y * scale * ow;
        for x in 0..w {
            let id = sim.world().cell(x as i32, y as i32).material();
            for sx in 0..scale {
                buf[row_start + x * scale + sx] = id;
            }
        }
        let (head, tail) = buf[row_start..(y + 1) * scale * ow].split_at_mut(ow);
        for chunk in tail.chunks_exact_mut(ow) {
            chunk.copy_from_slice(head);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_realtime_dense_sampling() {
        assert_eq!(frame_delay(1, None), 2); // 1.67cs → 下限 2
        assert_eq!(frame_delay(4, None), 7); // 6.67 → 7，M0 mixed 口径不变
    }

    #[test]
    fn delay_sparse_sampling_caps_to_timelapse() {
        assert_eq!(frame_delay(100, None), 10); // 修复前 167cs 幻灯片
        assert_eq!(frame_delay(6, None), 10); // 恰好 10cs 边界
    }

    #[test]
    fn delay_fps_override_decouples_from_every() {
        assert_eq!(frame_delay(100, Some(25)), 4);
        assert_eq!(frame_delay(1, Some(100)), 2); // 1cs → 下限 2
        assert_eq!(frame_delay(50, Some(0)), 100); // fps=0 防除零按 1 处理
    }
}
