//! 材料表（spec §2）。数据由 harness 从 `data/materials.ron` 加载后注入——
//! Ring 0 不碰文件系统。id 显式声明（load-order 确定性，R1 教训）。

/// 核心语义依赖的两个固定 id（spec §1.2 加载校验强制）。
pub const MAT_AIR: u8 = 0;
pub const MAT_WALL: u8 = 1;

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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(id: u8, name: &str, category: Category, density: u16) -> MaterialDef {
        MaterialDef { id, name: name.into(), category, density, color: (0, 0, 0) }
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
}
