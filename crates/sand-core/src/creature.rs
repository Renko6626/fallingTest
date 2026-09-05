//! 生物：模板表 + 运行时状态（spec §3.2、§4）。本 Task 只建空壳（零行为
//! 变化）——字段与运动学留到 Task 2 起逐步填。

/// 生物模板表（Task 3 填字段）。与 `MaterialTable` 同体例：加载期构造、只读。
#[derive(Clone, Debug, Default)]
pub struct CreatureTable {
    // Task 3 起填
}

impl CreatureTable {
    pub fn empty() -> CreatureTable {
        CreatureTable::default()
    }
}

/// 生物表（Task 2 填字段）。
#[derive(Clone, Debug, Default)]
pub struct Creatures {
    // Task 2 起填
}

impl Creatures {
    pub fn new() -> Creatures {
        Creatures::default()
    }

    /// 实体层哈希的生物部分。空表时恒返回 0——Task 1 的"零行为变化"依赖此
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
    fn empty_creatures_hash_into_is_zero() {
        assert_eq!(Creatures::new().hash_into(), 0);
    }
}
