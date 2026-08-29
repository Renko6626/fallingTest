//! 状态哈希（spec §5.1）。per-chunk xxh3 为哈希树叶层，
//! desync 定位（M5）与大世界 per-chunk 序列化（charter §9 缝 2）复用。

use xxhash_rust::xxh3::Xxh3;

use crate::chunk::Chunk;
use crate::world::World;

pub fn chunk_hash(chunk: &Chunk, cx: u32, cy: u32) -> u64 {
    let mut h = Xxh3::new();
    for cell in &chunk.cells {
        h.update(&cell.0.to_le_bytes());
    }
    h.update(&cx.to_le_bytes());
    h.update(&cy.to_le_bytes());
    h.digest()
}

pub fn state_hash(world: &World) -> u64 {
    let mut h = Xxh3::new();
    h.update(&world.tick.to_le_bytes());
    for cy in 0..world.height_chunks {
        for cx in 0..world.width_chunks {
            let ch = chunk_hash(&world.chunks[world.chunk_index(cx, cy)], cx as u32, cy as u32);
            h.update(&ch.to_le_bytes());
        }
    }
    h.digest()
}

/// 首个哈希不一致的 chunk 坐标（desync 定位）。
pub fn first_diverging_chunk(a: &World, b: &World) -> Option<(usize, usize)> {
    for cy in 0..a.height_chunks {
        for cx in 0..a.width_chunks {
            let i = a.chunk_index(cx, cy);
            if chunk_hash(&a.chunks[i], cx as u32, cy as u32)
                != chunk_hash(&b.chunks[i], cx as u32, cy as u32)
            {
                return Some((cx, cy));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::Cell;

    #[test]
    fn stable_and_sensitive() {
        let mut a = World::new(2, 2, 7);
        let b = World::new(2, 2, 7);
        assert_eq!(state_hash(&a), state_hash(&b), "同状态双算必须同值");
        a.chunks[3].cells[0] = Cell::pack(2, 0);
        assert_ne!(state_hash(&a), state_hash(&b), "单 cell 差异必须可见");
        assert_eq!(first_diverging_chunk(&a, &b), Some((1, 1)));
    }
}
