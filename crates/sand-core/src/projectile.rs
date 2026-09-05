//! 弹体：SoA 表（spec §3.3、§5）。本 Task 只建空壳（零行为变化）——
//! 字段与积分/DDA/命中结算留到 Task 4 起逐步填。

/// 弹体表（Task 4 填字段）。
#[derive(Clone, Debug, Default)]
pub struct Projectiles {
    // Task 4 起填
}

impl Projectiles {
    pub fn new() -> Projectiles {
        Projectiles::default()
    }

    /// 实体层哈希的弹体部分。空表时恒返回 0——Task 1 的"零行为变化"依赖此
    /// （早退，不跑空 fold；后续 Task 填字段时必须替换本实现，而非在此基础上
    /// 叠加，否则哈希结构变了却看不出来）。
    pub fn hash_into(&self) -> u64 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_projectiles_hash_into_is_zero() {
        assert_eq!(Projectiles::new().hash_into(), 0);
    }
}
