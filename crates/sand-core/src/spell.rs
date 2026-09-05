//! 法术：表 + loadout + 施法闸门（spec §3.4、§6）。本 Task 只建空壳
//! （零行为变化）——字段与施法结算留到 Task 5 起逐步填。

/// 法术表（Task 5 填字段）。与 `MaterialTable` 同体例：加载期构造、只读。
#[derive(Clone, Debug, Default)]
pub struct SpellTable {
    // Task 5 起填
}

impl SpellTable {
    pub fn empty() -> SpellTable {
        SpellTable::default()
    }
}
