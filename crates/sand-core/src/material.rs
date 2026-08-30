//! 材料表（spec §2）。数据由 harness 从 `data/materials.ron` 加载后注入——
//! Ring 0 不碰文件系统。id 显式声明（load-order 确定性，R1 教训）。

/// 核心语义依赖的两个固定 id（spec §1.2 加载校验强制）。
pub const MAT_AIR: u8 = 0;
pub const MAT_WALL: u8 = 1;

/// `blast_cost` 哨兵值：代表"当前简化版爆炸免疫"（spec §6：M1 里 wall 的
/// `blast_cost`），任何有限 `power` 都无法满足 `energy >= cost`——射线撞上
/// 这类材料必然断线，绝不摧毁。`u32::MAX` 而非某个"足够大"的有限值：语义
/// 上就是"无限"，不依赖"场景 power 不会超过某个界"这类隐含假设。
/// M2 反应表引入 durability/hardness 后此字段语义细化替换（设计 §12）。
pub const BLAST_COST_INFINITE: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    Static,
    Powder,
    Liquid,
}

#[derive(Clone, Debug)]
pub struct MaterialDef {
    pub id: u8,
    pub name: String,
    pub category: Category,
    pub density: u16,
    pub color: (u8, u8, u8),
    /// 爆炸射线逐格能量消耗（spec §6）：air 0、water 1、sand 2、wall
    /// [`BLAST_COST_INFINITE`]。RON 缺省 1（`sand-harness::scenario::MatSpec`
    /// 的 serde 默认），字段本身在 core 侧不做取值校验——错误配置的后果只是
    /// 打出手感不对的爆炸，不影响确定性红线。
    pub blast_cost: u32,
    /// 近心汽化阈值（spec §6 汽化小节，用户裁决 2026-08-30）：`world::fire_ray`
    /// 摧毁该材质格子时，若"扣费后剩余能量 / power"的比例**严格超过**此阈
    /// 值，格子直接删除、不生成粒子（质量确定性蒸发，计入
    /// `World::vaporized_total` 诊断计数，不入哈希）。量化域是 u8：RON 里写
    /// `0.0..=1.0` 十进制，`sand-harness::scenario::quantize_vaporize_threshold`
    /// 在加载期一次性 `×255 round`，core 边界只见这个整数——字段本身在 core
    /// 侧不做取值校验，同 `blast_cost` 先例。**255 = RON 缺省 1.0 = 永不
    /// 汽化**（`fire_ray` 里 `remaining <= power` 恒成立，`remaining*255 >
    /// power*255` 即 `remaining > power` 恒假）。
    pub vaporize_threshold: u8,
}

#[derive(Clone, Debug)]
pub struct MaterialTable {
    defs: Vec<MaterialDef>,
}

impl MaterialTable {
    /// 校验：id 连续无重复、air=0 与 wall=1 存在且为 Static。
    pub fn new(mut defs: Vec<MaterialDef>) -> Result<MaterialTable, String> {
        defs.sort_by_key(|d| d.id);
        for (i, d) in defs.iter().enumerate() {
            if d.id as usize != i {
                return Err(format!("material id 必须从 0 连续：位置 {} 的 id 是 {}", i, d.id));
            }
        }
        let check = |id: u8, name: &str| -> Result<(), String> {
            let d = defs.get(id as usize).ok_or(format!("缺少材料 {name}"))?;
            if d.name != name || d.category != Category::Static {
                return Err(format!("id {id} 必须是 Static 的 {name}（实际：{}）", d.name));
            }
            Ok(())
        };
        check(MAT_AIR, "air")?;
        check(MAT_WALL, "wall")?;
        Ok(MaterialTable { defs })
    }

    pub fn id_by_name(&self, name: &str) -> Option<u8> {
        self.defs.iter().find(|d| d.name == name).map(|d| d.id)
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    pub fn category(&self, id: u8) -> Category {
        self.defs[id as usize].category
    }

    pub fn is_static(&self, id: u8) -> bool {
        self.category(id) == Category::Static
    }

    pub fn density(&self, id: u8) -> u16 {
        self.defs[id as usize].density
    }

    pub fn color(&self, id: u8) -> (u8, u8, u8) {
        self.defs[id as usize].color
    }

    /// 爆炸射线逐格能量消耗（spec §6）；[`BLAST_COST_INFINITE`] 代表免疫。
    pub fn blast_cost(&self, id: u8) -> u32 {
        self.defs[id as usize].blast_cost
    }

    /// 近心汽化阈值（spec §6 汽化小节）；量化后的 u8，255 = 永不汽化。
    pub fn vaporize_threshold(&self, id: u8) -> u8 {
        self.defs[id as usize].vaporize_threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(id: u8, name: &str, category: Category, density: u16) -> MaterialDef {
        MaterialDef { id, name: name.into(), category, density, color: (0, 0, 0), blast_cost: 1, vaporize_threshold: 255 }
    }

    #[test]
    fn accepts_valid_table_any_declaration_order() {
        // 声明序打乱，id 显式——加载顺序不影响结果（R1 语义）
        let t = MaterialTable::new(vec![
            def(2, "sand", Category::Powder, 40),
            def(0, "air", Category::Static, 0),
            def(3, "water", Category::Liquid, 16),
            def(1, "wall", Category::Static, 100),
        ])
        .unwrap();
        assert_eq!(t.id_by_name("sand"), Some(2));
        assert_eq!(t.density(3), 16);
    }

    #[test]
    fn rejects_gap_and_wrong_sentinels() {
        assert!(MaterialTable::new(vec![def(0, "air", Category::Static, 0), def(2, "sand", Category::Powder, 40)]).is_err());
        assert!(MaterialTable::new(vec![def(0, "wall", Category::Static, 0), def(1, "air", Category::Static, 0)]).is_err());
    }

    // ==================== blast_cost（M1 Task 6）====================

    #[test]
    fn blast_cost_accessor_returns_declared_value_including_infinite_sentinel() {
        let mk = |id, name, cost| MaterialDef {
            id,
            name: String::from(name),
            category: Category::Static,
            density: 0,
            color: (0, 0, 0),
            blast_cost: cost,
            vaporize_threshold: 255,
        };
        let t = MaterialTable::new(vec![mk(0, "air", 0), mk(1, "wall", BLAST_COST_INFINITE)]).unwrap();
        assert_eq!(t.blast_cost(0), 0);
        assert_eq!(t.blast_cost(1), BLAST_COST_INFINITE);
    }

    // ==================== vaporize_threshold（近心汽化，用户裁决 2026-08-30）====================

    #[test]
    fn vaporize_threshold_accessor_returns_declared_value() {
        let mk = |id, name, threshold| MaterialDef {
            id,
            name: String::from(name),
            category: Category::Static,
            density: 0,
            color: (0, 0, 0),
            blast_cost: 1,
            vaporize_threshold: threshold,
        };
        let t = MaterialTable::new(vec![mk(0, "air", 255), mk(1, "wall", 102)]).unwrap();
        assert_eq!(t.vaporize_threshold(0), 255);
        assert_eq!(t.vaporize_threshold(1), 102);
    }
}
