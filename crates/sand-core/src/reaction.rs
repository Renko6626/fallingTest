//! 反应表（M2 spec §2.4/§2.5）：稠密二维索引 + 连续条目存储。
//!
//! **作者格式是稀疏的**（`data/reactions.ron` 一条条反应，人手写人手读）；
//! 本表纯粹是加载期由稀疏条目构建的内部索引。tag 展开、名字解析、概率量化、
//! 发起方规范化全部发生在 harness 加载期（spec §2.4 四条契约）——本模块入参
//! 已是规范化的 id 条目，core 侧不出现任何字符串。
//!
//! **查找热路径**（spec §2.5）：`get(a, b)` 一次索引载入、无分支哈希——这条
//! 查找在全引擎最热的循环里（每活跃 cell 每 tick 对邻居查若干次），按加载期
//! 实际材质数 `n` 开 `n×n` 表（本轮 n=8 ⇒ 128 字节），比稀疏方案的二分 +
//! 分支预测失败划算。**切换判据写死**：材质数越过 64 种，或 bench 显示该表
//! cache 行为成为瓶颈，换"per-material 位掩码提前退出 + 稀疏结果表"——表在
//! `get` 这一个访问器后面，换实现时调用方一行不动。
//!
//! 顺带绕开"禁 std HashMap 默认 hasher"红线（charter §6 第 4 条），且天然定序。

use crate::material::MaterialTable;

/// 一条规范化反应（加载期产物）：`a < b` 恒成立（发起方约定，spec §4.2），
/// 同一对材质的多条按加载序连续存放（spec §4.1：逐条掷骰取第一个命中）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReactionRule {
    pub a: u8,
    pub b: u8,
    /// 发起方（a 侧）的产物。
    pub out_a: u8,
    /// 邻居（b 侧）的产物。
    pub out_b: u8,
    /// ×255 量化概率阈值（`roll % 255 < threshold` 即命中，与
    /// `splash_chance` 同口径：0 = 永不、255 = 必发）。
    pub threshold: u8,
}

/// 反应查找表 + eval 准入预计算（spec §2.5/§2.6）。
#[derive(Clone, Debug)]
pub struct ReactionTable {
    n: usize,
    /// `n×n` 稠密索引，`[a*n + b]`：0 = 无反应，否则 `1 + rules 起始偏移`。
    index: Vec<u16>,
    /// 按 `(a, b, 加载序)` 排序的连续条目（稳定排序保加载序）。
    rules: Vec<ReactionRule>,
    /// per-material：是否存在以该材质为发起方（`id_a`）的条目（spec §2.6）。
    initiates: Vec<bool>,
    /// per-material eval 准入（spec §2.6 审阅补记"一次查表"）：
    /// `!is_static(m) ∨ initiates(m)`。Task 3 的 `counter > 0` 项在 `eval`
    /// 里另做位测试，不进本表——counter 是 cell 状态不是材质属性。
    needs_eval: Vec<bool>,
}

impl ReactionTable {
    /// 空表（无任何反应）：既有测试与"纯运动"场景的零迁移路径。
    pub fn empty(table: &MaterialTable) -> ReactionTable {
        ReactionTable::new(table, Vec::new()).expect("空表构建不可能失败")
    }

    /// 由规范化条目构建。入参契约（违反即 Err，加载期显式报错——spec §2.4
    /// 契约 1，与 Noita 的静默丢弃反着抄）：
    /// - 所有 id（a/b/out_a/out_b）< 材质数；
    /// - `a < b`（正反只注册一次；`a == b` 的自反应被发起方约定天然排除，
    ///   显式传入即错误）。
    pub fn new(table: &MaterialTable, mut rules: Vec<ReactionRule>) -> Result<ReactionTable, String> {
        let n = table.len();
        if n * n > u16::MAX as usize {
            return Err(format!("材质数 {n} 过大：n×n 稠密索引超出 u16 偏移域"));
        }
        for r in &rules {
            for id in [r.a, r.b, r.out_a, r.out_b] {
                if (id as usize) >= n {
                    return Err(format!("反应条目引用越界材质 id {id}（材质数 {n}）：{r:?}"));
                }
            }
            if r.a >= r.b {
                return Err(format!(
                    "反应条目必须规范化为 a < b（自反应不受支持，spec §1.4）：{r:?}"
                ));
            }
        }
        if rules.len() >= u16::MAX as usize {
            return Err(format!("反应条目数 {} 超出 u16 偏移域", rules.len()));
        }
        // 稳定排序：同 (a, b) 的条目保持加载序（spec §4.1"按加载序逐条掷骰"）。
        rules.sort_by_key(|r| (r.a, r.b));
        let mut index = vec![0u16; n * n];
        let mut initiates = vec![false; n];
        for (i, r) in rules.iter().enumerate() {
            let slot = &mut index[r.a as usize * n + r.b as usize];
            if *slot == 0 {
                *slot = (i + 1) as u16;
            }
            initiates[r.a as usize] = true;
        }
        let needs_eval = (0..n).map(|m| !table.is_static(m as u8) || initiates[m]).collect();
        Ok(ReactionTable { n, index, rules, initiates, needs_eval })
    }

    /// 同对材质的全部条目（按加载序）。调用方保证 `a < b`（发起方约定）；
    /// 无反应返回空切片。热路径：一次索引载入 + 连续切片扫描。
    pub fn get(&self, a: u8, b: u8) -> &[ReactionRule] {
        debug_assert!(a < b, "get 要求发起方约定 a < b（a={a}, b={b}）");
        let off = self.index[a as usize * self.n + b as usize];
        if off == 0 {
            return &[];
        }
        let start = (off - 1) as usize;
        let mut end = start + 1;
        while end < self.rules.len() && self.rules[end].a == a && self.rules[end].b == b {
            end += 1;
        }
        &self.rules[start..end]
    }

    /// 是否存在以 `a` 为发起方的条目（加载期预计算，spec §2.6）。
    pub fn initiates(&self, a: u8) -> bool {
        self.initiates[a as usize]
    }

    /// eval 准入（spec §2.6）：`!is_static ∨ initiates`。单次数组载入——
    /// wall 与未点燃的可燃 Static 一个分支就退出，M0 稀疏扫描性能不受影响。
    pub fn needs_eval(&self, m: u8) -> bool {
        self.needs_eval[m as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::{Category, MaterialDef, MaterialTable};

    fn table() -> MaterialTable {
        // 0 air / 1 wall / 2 sand / 3 water / 4 fire / 5 smoke
        MaterialTable::new(vec![
            MaterialDef::base(0, "air", Category::Static, 0),
            MaterialDef::base(1, "wall", Category::Static, 100),
            MaterialDef::base(2, "sand", Category::Powder, 40),
            MaterialDef::base(3, "water", Category::Liquid, 16),
            MaterialDef::base(4, "fire", Category::Gas, 1),
            MaterialDef::base(5, "smoke", Category::Gas, 2),
        ])
        .unwrap()
    }

    fn rule(a: u8, b: u8, out_a: u8, out_b: u8, threshold: u8) -> ReactionRule {
        ReactionRule { a, b, out_a, out_b, threshold }
    }

    #[test]
    fn get_returns_pair_entries_in_load_order() {
        // 同对 (3,4) 两条 + 另一对 (2,3) 一条，交错声明——get 必须按加载序
        // 返回同对的连续切片（spec §4.1）。
        let t = ReactionTable::new(
            &table(),
            vec![rule(3, 4, 3, 5, 204), rule(2, 3, 2, 2, 10), rule(3, 4, 0, 0, 1)],
        )
        .unwrap();
        let got = t.get(3, 4);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].threshold, 204, "第一条必须是先声明的那条");
        assert_eq!(got[1].threshold, 1);
        assert_eq!(t.get(2, 3).len(), 1);
        assert!(t.get(2, 4).is_empty(), "未声明的对必须返回空");
    }

    #[test]
    fn initiates_only_true_for_id_a_side() {
        let t = ReactionTable::new(&table(), vec![rule(3, 4, 3, 5, 204)]).unwrap();
        assert!(t.initiates(3), "water 是发起方");
        assert!(!t.initiates(4), "fire 只是被发起侧");
        assert!(!t.initiates(2));
    }

    #[test]
    fn needs_eval_is_not_static_or_initiates() {
        // 让 Static 的 wall 成为发起方（1 < 2），验证 needs_eval 把它拉进 eval。
        let t = ReactionTable::new(&table(), vec![rule(1, 2, 0, 0, 255)]).unwrap();
        assert!(!t.needs_eval(0), "air：Static 且不发起");
        assert!(t.needs_eval(1), "wall 成为发起方后必须进 eval");
        assert!(t.needs_eval(2), "非 Static 恒进");
        assert!(t.needs_eval(4), "Gas 恒进");
        let e = ReactionTable::empty(&table());
        assert!(!e.needs_eval(1), "空表下 Static 不进 eval——与 M2 之前行为逐位一致");
        assert!(e.needs_eval(3));
    }

    #[test]
    fn rejects_self_reaction_reversed_and_out_of_range() {
        let t = table();
        assert!(ReactionTable::new(&t, vec![rule(3, 3, 0, 0, 255)]).is_err(), "a == b 必须拒绝");
        assert!(ReactionTable::new(&t, vec![rule(4, 3, 0, 0, 255)]).is_err(), "a > b 必须拒绝");
        assert!(ReactionTable::new(&t, vec![rule(3, 9, 0, 0, 255)]).is_err(), "越界 id 必须拒绝");
        assert!(ReactionTable::new(&t, vec![rule(3, 4, 9, 0, 255)]).is_err(), "越界产物必须拒绝");
    }
}
