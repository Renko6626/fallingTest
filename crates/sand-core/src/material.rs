//! 材料表（spec §2）。数据由 harness 从 `data/materials.ron` 加载后注入——
//! Ring 0 不碰文件系统。id 显式声明（load-order 确定性，R1 教训）。

/// 核心语义依赖的两个固定 id（spec §1.2 加载校验强制）。
pub const MAT_AIR: u8 = 0;
pub const MAT_WALL: u8 = 1;

/// 液体单 tick 横移（色散）距离上限（Layer G Task 1，spec §3 / §5）。
///
/// **这个常量是 P4 写域论证的一部分，不是手感旋钮**：`rules::side` 的探测与
/// 写入半径直接等于色散距离，越界即写出 `WriteWindow`——debug 构建撞窗口
/// 断言 panic，release 构建变成同相邻 chunk 的数据竞争 → SyncTest 分叉。
/// 故 `side` 无条件把材料声明值 clamp 到本常量（spec §3.1 评审修订），
/// harness 加载期的 `1..=DISPERSION_MAX` 校验只是给用户的可见报错，不是
/// 唯一防线。改动本常量必须同步复审 `window.rs` 的 r ≤ HALO 编译期断言。
pub const DISPERSION_MAX: u8 = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    Static,
    Powder,
    Liquid,
    /// 气体（M2 spec §3）：走 `rules::gas_step` 上浮 + 水平扩散，恒 1 格/tick、
    /// 不进 substeps、不占速度位段（充分性论证见 charter §11 翻案 6）。
    Gas,
}

#[derive(Clone, Debug)]
pub struct MaterialDef {
    pub id: u8,
    pub name: String,
    pub category: Category,
    pub density: u16,
    pub color: (u8, u8, u8),
    /// 爆炸射线逐格能量消耗（M2 spec §2.2，原 `blast_cost` 改名）：Noita
    /// 双层破坏模型的**能量池**一侧——射线逐格扣减 `hp`，能量不足即断线。
    /// RON 缺省 1；core 侧不做取值校验（错误配置只是手感不对，不碰确定性）。
    pub hp: u32,
    /// 破坏门槛（M2 spec §2.2，Noita 双层破坏模型的**门槛**一侧）：
    /// `durability > 操作侧 max_durability` ⇒ 完全免疫、射线断线。
    /// "我能打穿多硬的东西"是操作侧参数（`Op::Explode::max_durability`），
    /// 门槛不写死在材质侧。RON 缺省 0（谁都打得动）；wall 取 15（高于任何
    /// 法术上限）——原 `BLAST_COST_INFINITE` 哨兵退役，语义比"无限能量消耗"
    /// 直白，且不再依赖"power 不会超过某个界"的隐含假设。
    pub durability: u8,
    /// 近心汽化阈值（spec §6 汽化小节，用户裁决 2026-08-30）：`explode::fire_ray`
    /// 摧毁该材质格子时，若"扣费后剩余能量 / power"的比例**严格超过**此阈
    /// 值，格子直接删除、不生成粒子（质量确定性蒸发，计入
    /// `World::vaporized_total` 诊断计数，不入哈希）。量化域是 u8：RON 里写
    /// `0.0..=1.0` 十进制，`sand-harness::scenario::quantize_vaporize_threshold`
    /// 在加载期一次性 `×255 round`，core 边界只见这个整数——字段本身在 core
    /// 侧不做取值校验，同 `blast_cost` 先例。**255 = RON 缺省 1.0 = 永不
    /// 汽化**（`fire_ray` 里 `remaining <= power` 恒成立，`remaining*255 >
    /// power*255` 即 `remaining > power` 恒假）。
    pub vaporize_threshold: u8,
    /// 液体单 tick 横移（色散）距离，单位格（Layer G Task 1，spec §3）。
    /// RON 缺省 1 = 改动前的单格横移语义；harness 加载期校验
    /// `1..=DISPERSION_MAX`。**与 `blast_cost`/`vaporize_threshold` 不同**：
    /// 越界值不只是手感不对，而会破坏 P4 写域论证，故 `rules::side` 另有
    /// clamp 兜底（见 [`DISPERSION_MAX`]）。只对 `Category::Liquid` 有意义
    /// （粉末不走 `side`，spec §1.3 Non-goals）。
    pub dispersion: u8,
    /// 撞击溅射概率（Layer G Task 3，spec §6.2）：本 tick 撞停且速度达
    /// [`crate::cell::SPLASH_MIN_SPEED`] 时，按此概率把该格脱格成粒子。
    /// 量化域是 u8：RON 里写 `0.0..=1.0` 十进制，
    /// `sand-harness::scenario::quantize_splash_chance` 在加载期一次性
    /// `×255 round`，core 边界只见整数——完全照 `vaporize_threshold` 的体例。
    /// **0 = 缺省 = 永不溅射**，故未声明该字段的材质行为与 Task 3 之前逐位相同。
    ///
    /// 与 `dispersion` 不同，本字段**不进** P4 写域论证：溅射产出的是粒子，
    /// 走 Layer P 自己的 DDA 与串行落格，不经 `WriteWindow`。故沿用
    /// `blast_cost`/`vaporize_threshold` 的"core 侧不校验"先例——配错的后果
    /// 只是水花多寡不对。
    pub splash_chance: u8,
    /// 反应匹配 tag（M2 spec §2.1）：core 侧运行期**不读**——tag 展开在
    /// harness 加载期一次性完成（spec §2.4 契约 2"core 不出现字符串"），
    /// 本字段只是展开器的输入随表存放（`tags_of` 访问器）。
    pub tags: Vec<String>,
    /// 着火点（Noita `autoignition_temperature` 的静态常量版，M2 spec §2.1）：
    /// 点燃判定 `源.fire_temp >= 目标.ignition_temp`。缺省 100 = 缺省火温 10
    /// 点不着任何未声明材质。
    pub ignition_temp: u8,
    /// 火温（Noita `temperature_of_fire`）：本材质**作为燃烧源**时的输出温度。
    /// 只有 `counter > 0` 的格才是燃烧源（spec §5.2 审阅补漏的门），冷材质
    /// 声明高火温不会自燃点邻居。缺省 10。
    pub fire_temp: u8,
    /// 燃料池初值（Noita `fire_hp`，M2 spec §2.1）：**0 = 不可燃**；被点燃时
    /// 装填进 cell 的 counter 位段。与 `lifetime` 至多声明其一
    /// （[`MaterialTable::new`] 校验——两者共用同一个 counter，语义靠装填
    /// 时机区分，同时声明是配置错误）。
    pub fire_hp: u8,
    /// 寿命初值（fire/smoke 类，M2 spec §2.1）：**出生即装填**进 counter
    /// （`world::set_cell_stamped` 统一写入路径）。0 = 无寿命语义。
    pub lifetime: u8,
    /// counter 归零后的转化目标材质 id（M2 spec §5.1"归零即衰变"）。RON 面
    /// 写材质名，harness 加载期解析成 id——core 不见字符串。缺省 air。
    pub decay_to: u8,
    /// 为真：只有邻接 air 的格才推进燃烧（由外向内烧，M2 spec §5.4）；
    /// 四周无 air 即闷熄。缺省 true。
    pub requires_oxygen: bool,
    /// 为真：燃烧格邻接到本材质即清零 counter（灭火，M2 spec §5.5）。
    /// 走数据字段而非反应表——"正在燃烧"是 cell 状态，反应表匹配的是材质，
    /// 表达不了。缺省 false。
    pub extinguisher: bool,
    /// 产火概率（对应 Noita `generates_flames`，M2 spec §5.3）：燃烧中的格子
    /// 每 tick 按此概率向邻接 air 写入 fire。量化域 u8（×255 round，照
    /// `splash_chance` 体例）。缺省 0 = 不产火。
    pub fire_chance: u8,
}

impl MaterialDef {
    /// 缺省底座（M2 spec §2.1"缺省安全"）：除四个身份参数外全部取"RON 未声明"
    /// 的缺省值，与 harness 的 serde 缺省同口径。测试与程序化构表用
    /// `MaterialDef { <覆盖项>, ..MaterialDef::base(..) }`，新增字段不再逐处爆改。
    pub fn base(id: u8, name: &str, category: Category, density: u16) -> MaterialDef {
        MaterialDef {
            id,
            name: name.into(),
            category,
            density,
            color: (0, 0, 0),
            hp: 1,
            durability: 0,
            vaporize_threshold: 255,
            dispersion: 1,
            splash_chance: 0,
            tags: Vec::new(),
            ignition_temp: 100,
            fire_temp: 10,
            fire_hp: 0,
            lifetime: 0,
            decay_to: MAT_AIR,
            requires_oxygen: true,
            extinguisher: false,
            fire_chance: 0,
        }
    }
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
        for d in &defs {
            // M2 spec §2.1 加载期校验：fire_hp 与 lifetime 共用 counter 位段，
            // 语义靠装填时机区分，同时声明是配置错误而非可用组合。放在 core 侧
            // 而非 harness——直接构表的测试/程序化调用方同样必须被拦住。
            if d.fire_hp > 0 && d.lifetime > 0 {
                return Err(format!(
                    "材料 '{}'（id={}）同时声明了 fire_hp 与 lifetime——两者共用 counter 位段，至多其一",
                    d.name, d.id
                ));
            }
            if (d.decay_to as usize) >= defs.len() {
                return Err(format!(
                    "材料 '{}'（id={}）的 decay_to={} 越界（材质数 {}）",
                    d.name, d.id, d.decay_to, defs.len()
                ));
            }
        }
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

    /// 爆炸射线逐格能量消耗（M2 spec §2.2 双层破坏模型的能量池一侧）。
    pub fn hp(&self, id: u8) -> u32 {
        self.defs[id as usize].hp
    }

    /// 破坏门槛（M2 spec §2.2）：超过操作侧 `max_durability` 即完全免疫。
    pub fn durability(&self, id: u8) -> u8 {
        self.defs[id as usize].durability
    }

    /// 近心汽化阈值（spec §6 汽化小节）；量化后的 u8，255 = 永不汽化。
    pub fn vaporize_threshold(&self, id: u8) -> u8 {
        self.defs[id as usize].vaporize_threshold
    }

    /// 液体色散距离（spec §3）。返回材料声明的**原始值**——clamp 由
    /// `rules::side` 在使用点施加（见 [`DISPERSION_MAX`]），这里不做修饰，
    /// 便于加载期校验与诊断读到用户真正写下的值。
    pub fn dispersion(&self, id: u8) -> u8 {
        self.defs[id as usize].dispersion
    }

    pub fn splash_chance(&self, id: u8) -> u8 {
        self.defs[id as usize].splash_chance
    }

    // ---------- M2 反应/燃烧字段访问器（spec §2.1）----------

    /// 反应 tag 列表（仅 harness 加载期展开用，core 运行期不读）。
    pub fn tags_of(&self, id: u8) -> &[String] {
        &self.defs[id as usize].tags
    }

    pub fn ignition_temp(&self, id: u8) -> u8 {
        self.defs[id as usize].ignition_temp
    }

    pub fn fire_temp(&self, id: u8) -> u8 {
        self.defs[id as usize].fire_temp
    }

    pub fn fire_hp(&self, id: u8) -> u8 {
        self.defs[id as usize].fire_hp
    }

    pub fn lifetime(&self, id: u8) -> u8 {
        self.defs[id as usize].lifetime
    }

    pub fn decay_to(&self, id: u8) -> u8 {
        self.defs[id as usize].decay_to
    }

    pub fn requires_oxygen(&self, id: u8) -> bool {
        self.defs[id as usize].requires_oxygen
    }

    pub fn extinguisher(&self, id: u8) -> bool {
        self.defs[id as usize].extinguisher
    }

    pub fn fire_chance(&self, id: u8) -> u8 {
        self.defs[id as usize].fire_chance
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(id: u8, name: &str, category: Category, density: u16) -> MaterialDef {
        MaterialDef::base(id, name, category, density)
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

    // ==================== hp / durability（M2 spec §2.2 双层破坏）====================

    #[test]
    fn hp_and_durability_accessors_return_declared_values() {
        let mk = |id, name, hp, durability| MaterialDef {
            hp,
            durability,
            ..MaterialDef::base(id, name, Category::Static, 0)
        };
        let t = MaterialTable::new(vec![mk(0, "air", 0, 0), mk(1, "wall", 100, 15)]).unwrap();
        assert_eq!(t.hp(0), 0);
        assert_eq!(t.hp(1), 100);
        assert_eq!(t.durability(0), 0);
        assert_eq!(t.durability(1), 15, "wall 门槛 15：高于任何法术上限（哨兵退役）");
    }

    // ==================== vaporize_threshold（近心汽化，用户裁决 2026-08-30）====================

    #[test]
    fn vaporize_threshold_accessor_returns_declared_value() {
        let mk = |id, name, threshold| MaterialDef {
            vaporize_threshold: threshold,
            ..MaterialDef::base(id, name, Category::Static, 0)
        };
        let t = MaterialTable::new(vec![mk(0, "air", 255), mk(1, "wall", 102)]).unwrap();
        assert_eq!(t.vaporize_threshold(0), 255);
        assert_eq!(t.vaporize_threshold(1), 102);
    }

    // ==================== M2 反应/燃烧字段（spec §2.1）====================

    #[test]
    fn m2_fields_default_to_safe_values_and_roundtrip_through_accessors() {
        let t = MaterialTable::new(vec![
            def(0, "air", Category::Static, 0),
            def(1, "wall", Category::Static, 100),
            MaterialDef {
                lifetime: 40,
                decay_to: 0,
                fire_temp: 100,
                ..MaterialDef::base(2, "fire", Category::Gas, 1)
            },
            MaterialDef {
                fire_hp: 90,
                ignition_temp: 40,
                fire_chance: 153,
                tags: vec!["burnable".into()],
                ..MaterialDef::base(3, "oil", Category::Liquid, 12)
            },
        ])
        .unwrap();
        // 缺省安全：未声明的材质拿到的就是"改动前行为"的值
        assert_eq!(t.fire_hp(0), 0, "缺省 fire_hp=0 = 不可燃");
        assert_eq!(t.lifetime(0), 0);
        assert_eq!(t.ignition_temp(0), 100);
        assert_eq!(t.fire_temp(0), 10);
        assert!(t.requires_oxygen(0));
        assert!(!t.extinguisher(0));
        assert_eq!(t.fire_chance(0), 0);
        assert!(t.tags_of(0).is_empty());
        // 声明值原样可读
        assert_eq!(t.category(2), Category::Gas);
        assert!(!t.is_static(2), "Gas 不是 Static");
        assert_eq!(t.lifetime(2), 40);
        assert_eq!(t.fire_temp(2), 100);
        assert_eq!(t.fire_hp(3), 90);
        assert_eq!(t.ignition_temp(3), 40);
        assert_eq!(t.fire_chance(3), 153);
        assert_eq!(t.tags_of(3), ["burnable".to_string()]);
    }

    #[test]
    fn rejects_material_declaring_both_fire_hp_and_lifetime() {
        // 两者共用 counter 位段（spec §2.1 校验），同时声明是配置错误。
        let bad = MaterialDef { fire_hp: 10, lifetime: 10, ..def(2, "weird", Category::Static, 5) };
        let r = MaterialTable::new(vec![
            def(0, "air", Category::Static, 0),
            def(1, "wall", Category::Static, 100),
            bad,
        ]);
        assert!(r.is_err(), "fire_hp 与 lifetime 同时声明必须在构表期被拒绝");
    }

    #[test]
    fn rejects_decay_to_out_of_range() {
        let bad = MaterialDef { lifetime: 5, decay_to: 9, ..def(2, "smoke", Category::Gas, 2) };
        let r = MaterialTable::new(vec![
            def(0, "air", Category::Static, 0),
            def(1, "wall", Category::Static, 100),
            bad,
        ]);
        assert!(r.is_err(), "decay_to 越界必须在构表期被拒绝");
    }
}
