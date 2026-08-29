//! 占位 GIF 渲染器（spec §6）。表现层：只消费 `Sim::world()` 只读视图
//! （Channel A 雏形），调色板来自 materials.ron 的 color 字段。
//! 不进确定性执法范围；渲染器变化不影响任何哈希。

use std::fs::File;

use sand_core::{MaterialTable, Sim};

use crate::scenario::Scenario;

pub struct RenderOpts {
    pub every: u64,
    pub scale: usize,
    pub out: String,
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
    // 60Hz 模拟，每 every tick 一帧；GIF delay 单位 = 1/100 秒
    let delay = ((opts.every as f64 * 100.0 / 60.0).round() as u16).max(2);

    let mut frames = 0usize;
    let mut buf = vec![0u8; ow * oh];
    for t in 0..ticks {
        sim.step(&sc.ops_for_tick(t));
        if t % opts.every == 0 || t + 1 == ticks {
            fill_frame(sim, w, h, opts.scale, &mut buf);
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
