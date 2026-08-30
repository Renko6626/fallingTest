//! 粒子池（spec §3，SoA）：脱格自由飞行粒子的确定性状态。
//!
//! **顺序即 id 序**：`spawn` 按调用序 append，下标即遍历序；移除走保序压缩
//! （[`Particles::compact`]，`retain` 语义）——"串行按 id 提交" = 按下标顺序遍历。
//!
//! **容量限流**：`len == MAX_PARTICLES` 时 `spawn` 确定性拒绝（丢弃 + `rejected_total`
//! 计数），计数器**不入哈希**，只供诊断。
//!
//! **无 lifetime 字段**：重力保证要么落格要么出界，出界即确定性销毁（Task 4）。
//! 本任务（Task 3）只提供数据结构本体 + 生成/压缩骨架，运动积分留 Task 4。

use xxhash_rust::xxh3::Xxh3;

use crate::fixed::Fx;

/// 粒子池容量上限（总纲初值，`kernel-charter.md:64`）。
pub const MAX_PARTICLES: usize = 65536;

/// 粒子池：SoA 布局，下标即 id 序（架构 §3 state 条目既定）。
#[derive(Clone, Debug, Default)]
pub struct Particles {
    x: Vec<Fx>,
    y: Vec<Fx>,
    vx: Vec<Fx>,
    vy: Vec<Fx>,
    material: Vec<u8>,
    /// 单调计数：每次成功 `spawn` 递增一次，入状态哈希；不做索引、不回收。
    next_id: u32,
    /// 容量拒绝次数：诊断用，**不入哈希**。
    rejected_total: u64,
}

impl Particles {
    pub fn new() -> Particles {
        Particles::default()
    }

    pub fn len(&self) -> usize {
        self.x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }

    /// 按下标（id 序）只读访问单个粒子字段，供 Task 4 积分/提交阶段复用。
    pub fn x(&self, i: usize) -> Fx {
        self.x[i]
    }
    pub fn y(&self, i: usize) -> Fx {
        self.y[i]
    }
    pub fn vx(&self, i: usize) -> Fx {
        self.vx[i]
    }
    pub fn vy(&self, i: usize) -> Fx {
        self.vy[i]
    }
    pub fn material(&self, i: usize) -> u8 {
        self.material[i]
    }

    /// 容量拒绝诊断计数（不入哈希）。
    pub fn rejected_total(&self) -> u64 {
        self.rejected_total
    }

    /// 追加一个粒子；容量满时确定性拒绝（丢弃 + 计数，返回 `false`）。
    /// 成功追加的粒子下标 = 追加前的 `len()`，即 id 序中的位置。
    pub fn spawn(&mut self, material: u8, x: Fx, y: Fx, vx: Fx, vy: Fx) -> bool {
        if self.len() >= MAX_PARTICLES {
            self.rejected_total += 1;
            return false;
        }
        self.x.push(x);
        self.y.push(y);
        self.vx.push(vx);
        self.vy.push(vy);
        self.material.push(material);
        self.next_id = self.next_id.wrapping_add(1);
        true
    }

    /// 保序压缩：按下标序保留 `keep[i] == true` 的粒子，其余移除
    /// （`retain` 语义，相对顺序不变）。`keep.len()` 必须等于 `len()`。
    /// Task 3 骨架无移除判据（无运动），Task 4 起用 Land/Gone 判定结果驱动。
    pub fn compact(&mut self, keep: &[bool]) {
        debug_assert_eq!(keep.len(), self.len(), "keep 掩码长度必须与粒子数一致");
        let mut w = 0usize;
        for (r, &k) in keep.iter().enumerate() {
            if k {
                if w != r {
                    self.x[w] = self.x[r];
                    self.y[w] = self.y[r];
                    self.vx[w] = self.vx[r];
                    self.vy[w] = self.vy[r];
                    self.material[w] = self.material[r];
                }
                w += 1;
            }
        }
        self.x.truncate(w);
        self.y.truncate(w);
        self.vx.truncate(w);
        self.vy.truncate(w);
        self.material.truncate(w);
    }

    /// 粒子层哈希（spec §9）：xxh3 按下标序（= id 序）折叠 `(x, y, vx, vy, material)`
    /// 原始位，末尾并入 `next_id` 与粒子数。空池也有稳定值（`next_id=0, len=0`）。
    pub fn hash_into(&self) -> u64 {
        let mut h = Xxh3::new();
        for i in 0..self.len() {
            h.update(&self.x[i].0.to_le_bytes());
            h.update(&self.y[i].0.to_le_bytes());
            h.update(&self.vx[i].0.to_le_bytes());
            h.update(&self.vy[i].0.to_le_bytes());
            h.update(&[self.material[i]]);
        }
        h.update(&self.next_id.to_le_bytes());
        h.update(&(self.len() as u64).to_le_bytes());
        h.digest()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx(v: i32) -> Fx {
        Fx::from_int(v)
    }

    #[test]
    fn spawn_order_is_traversal_order() {
        let mut p = Particles::new();
        for i in 0..5 {
            assert!(p.spawn(7, fx(i), fx(i * 2), fx(1), fx(0)));
        }
        assert_eq!(p.len(), 5);
        for i in 0..5 {
            assert_eq!(p.x(i as usize), fx(i));
            assert_eq!(p.y(i as usize), fx(i * 2));
        }
    }

    #[test]
    fn capacity_rejection_is_deterministic_and_repeatable() {
        let run = || {
            let mut p = Particles::new();
            let mut accepted = 0usize;
            let mut last_rejected = false;
            for i in 0..(MAX_PARTICLES + 1) {
                let ok = p.spawn(1, fx(i as i32), Fx::ZERO, Fx::ZERO, Fx::ZERO);
                if ok {
                    accepted += 1;
                } else {
                    last_rejected = true;
                }
            }
            (accepted, last_rejected, p.len(), p.rejected_total())
        };
        let (a1, r1, len1, rej1) = run();
        let (a2, r2, len2, rej2) = run();
        assert_eq!(a1, MAX_PARTICLES, "前 65536 个 spawn 必须全部成功");
        assert!(r1, "第 65537 个 spawn 必须被拒绝");
        assert_eq!(len1, MAX_PARTICLES);
        assert_eq!(rej1, 1);
        assert_eq!((a1, r1, len1, rej1), (a2, r2, len2, rej2), "重跑结果必须一致");
    }

    #[test]
    fn empty_pool_hash_is_stable() {
        let a = Particles::new();
        let b = Particles::new();
        assert_eq!(a.hash_into(), b.hash_into(), "空池哈希必须稳定");
    }

    #[test]
    fn hash_is_sensitive_to_particle_fields() {
        let mut a = Particles::new();
        a.spawn(3, fx(1), fx(2), fx(3), fx(4));
        let mut b = a.clone();
        // 改一个 vx，哈希必须变
        b.compact(&[false]); // 先清空 b……
        assert_eq!(b.len(), 0);
        b.spawn(3, fx(1), fx(2), fx(9), fx(4)); // …重建但 vx 不同
        assert_ne!(a.hash_into(), b.hash_into(), "vx 差异必须反映到哈希");
    }

    #[test]
    fn compact_preserves_order_of_kept_particles() {
        let mut p = Particles::new();
        for i in 0..4 {
            p.spawn(1, fx(i), Fx::ZERO, Fx::ZERO, Fx::ZERO);
        }
        p.compact(&[true, false, true, false]);
        assert_eq!(p.len(), 2);
        assert_eq!(p.x(0), fx(0));
        assert_eq!(p.x(1), fx(2));
    }
}
