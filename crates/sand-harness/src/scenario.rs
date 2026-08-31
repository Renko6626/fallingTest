//! 场景与材料表加载（spec §2、§5.2）。
//! 数据即确定性输入（P5）：materials.ron 与场景文件都算 xxh3 指纹，
//! 进入 replay/golden 头部——指纹不符即拒绝比对。

use serde::Deserialize;
use xxhash_rust::xxh3::{xxh3_64, Xxh3};

use sand_core::{
    Category, Fx, MaterialDef, MaterialTable, Op, ReactionRule, ReactionTable, DISPERSION_MAX,
    MAX_EMIT_JITTER_RAW,
};

/// `MatSpec::hp` 的 serde 缺省值（原 `blast_cost`，M2 spec §2.2："RON 缺省 1"）。
fn default_hp() -> u32 {
    1
}

/// `MatSpec::vaporize_threshold` 的 serde 缺省值（spec §6 汽化小节，用户裁决
/// 2026-08-30）：RON 缺省 1.0 = 永不汽化（量化后 255）。
fn default_vaporize_threshold() -> f64 {
    1.0
}

/// 缺省 0.0 = 永不溅射 ⇒ 未声明该字段的材质与 Layer G Task 3 之前逐位相同。
fn default_splash_chance() -> f64 {
    0.0
}

/// M2 spec §2.1 缺省：着火点 100（缺省火温 10 点不着任何未声明材质）。
fn default_ignition_temp() -> u8 {
    100
}

/// M2 spec §2.1 缺省：火温 10。
fn default_fire_temp() -> u8 {
    10
}

/// M2 spec §2.1 缺省：requires_oxygen = true。
fn default_requires_oxygen() -> bool {
    true
}

/// spec §5.3.1 第 4 条缺省：恒上浮（= 改动前行为）。
fn default_rise_chance() -> f64 {
    1.0
}

// ---------- materials.ron ----------

#[derive(Deserialize)]
struct MaterialsFile {
    materials: Vec<MatSpec>,
}

#[derive(Deserialize)]
struct MatSpec {
    id: u8,
    name: String,
    category: CatSpec,
    density: u16,
    color: (u8, u8, u8),
    /// 爆炸射线逐格能量消耗（spec §6）。缺省 1——已入库的 `materials.ron`
    /// 若不显式声明该字段仍能解析，但当前版本已给全部材料显式赋值（见该
    /// 文件注释：字段新增非语义变更，materials_fp 因内容变化而改变，golden
    /// 的 materials_fp 行按 spec §9 程序重录）。
    #[serde(default = "default_hp")]
    hp: u32,
    /// 破坏门槛（M2 spec §2.2）：`durability > 操作侧 max_durability` ⇒ 免疫。
    /// 缺省 0 = 谁都打得动；wall 声明 15（高于任何法术上限，哨兵退役）。
    #[serde(default)]
    durability: u8,
    /// 近心汽化阈值（spec §6 汽化小节，用户裁决 2026-08-30）：RON 写
    /// `0.0..=1.0` 十进制，缺省 1.0（永不汽化）。加载期经
    /// [`quantize_vaporize_threshold`] 一次性 round 量化为 u8——core 边界
    /// 只见量化后的整数，同 `blast_cost`/`Op::Emit` 的 `Fx` 先例。
    #[serde(default = "default_vaporize_threshold")]
    vaporize_threshold: f64,
    /// 液体单 tick 横移（色散）距离，单位格（Layer G Task 1，spec §3）。
    /// 缺省 1 = 改动前语义。取值域 `1..=DISPERSION_MAX`，越界在
    /// [`load_materials`] 报错——**不是**手感旋钮而是 P4 写域论证的输入
    /// （见 `sand_core::DISPERSION_MAX` 文档），故不走 `blast_cost` 那条
    /// "core 侧不校验"的先例。
    ///
    /// **`Option`（M2 Task 1）**：Gas 材质**禁止声明**本字段（气体水平扩散
    /// 恒 1 格、不读 dispersion——spec §3.1 审阅补漏，r_gas = 1 依赖这一条），
    /// 需要区分"未声明"与"声明了 1"，故不再走 serde 缺省函数。
    dispersion: Option<u8>,
    /// 撞击溅射概率（Layer G Task 3，spec §6.2）：RON 写 `0.0..=1.0` 十进制，
    /// 缺省 **0.0 = 永不溅射**（故未声明该字段的材质行为与 Task 3 之前逐位
    /// 相同）。加载期经 [`quantize_splash_chance`] 一次性 `×255 round` 量化
    /// 为 u8——完全照 `vaporize_threshold` 的体例，core 边界只见整数。
    #[serde(default = "default_splash_chance")]
    splash_chance: f64,
    // ---------- M2 反应/燃烧字段（spec §2.1，全部缺省安全）----------
    /// 反应匹配 tag，加载期由反应表展开器消费（Task 2）。
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default = "default_ignition_temp")]
    ignition_temp: u8,
    #[serde(default = "default_fire_temp")]
    fire_temp: u8,
    /// 燃料池初值，0 = 不可燃。与 `lifetime` 至多其一（core `MaterialTable::new` 校验）。
    #[serde(default)]
    fire_hp: u8,
    /// 寿命初值（fire/smoke 类），出生即装填。
    #[serde(default)]
    lifetime: u8,
    /// counter 归零后的转化目标**材质名**——此处是全 core 边界唯一的字符串引用，
    /// 在 [`load_materials`] 二遍解析成 id（引用不存在材质 ⇒ 加载失败，
    /// spec §2.4 契约 1 同款纪律）。缺省 air。
    #[serde(default)]
    decay_to: Option<String>,
    #[serde(default = "default_requires_oxygen")]
    requires_oxygen: bool,
    #[serde(default)]
    extinguisher: bool,
    /// 产火概率（Noita `generates_flames`，spec §5.3）：`0.0..=1.0`，
    /// 经 [`quantize_fire_chance`] ×255 round 量化。缺省 0 = 不产火。
    #[serde(default)]
    fire_chance: f64,
    /// 产火产物**材质名**（spec §5.3 实施补记）：`fire_chance > 0` 时必须
    /// 声明；与 `decay_to` 同样在加载期解析成 id。
    #[serde(default)]
    flame_to: Option<String>,
    /// Gas 每 tick 尝试上浮的概率（spec §5.3.1 第 4 条）：`0.0..=1.0`，
    /// 缺省 1.0 = 恒上浮 = 改动前行为。仅 Gas 消费。
    #[serde(default = "default_rise_chance")]
    rise_chance: f64,
}

#[derive(Deserialize)]
enum CatSpec {
    Static,
    Powder,
    Liquid,
    Gas,
}

/// 指纹归一化：剥掉所有 CR，使内容指纹只依赖**文件内容本身**，而不依赖
/// "文件是怎么落到磁盘上的"（2026-08-31 双机 hashrun 发现，charter §11
/// 实施期决策第 4 条）。
///
/// **为什么必须归一化**：指纹的语义是 P5 的"两端数据表不一致视同版本不一致"。
/// 裸字节哈希让它对行尾敏感 —— Windows 侧 `core.autocrlf=true` 检出成 CRLF
/// 后，**同一个 commit** 在两台机器上算出不同指纹，握手被拒，而两端的仿真
/// 其实逐字一致（实测：Linux rustc 1.89 与 Windows rustc 1.97 跨 9 场景、
/// 最长 2 万 tick，全部 tick 哈希与 final **逐位相同**，只有 fp 行不同）。
/// 这是**假阳性**：语义相同的数据被判为不同版本。
///
/// **为什么不改成哈希解析后的结构**（更"正确"的做法）：裸字节哈希有一个
/// 宝贵性质 —— **自动完备**，任何字段新增都自动进指纹。结构化哈希需要逐字段
/// 折叠，漏折一个字段就是一个静默的覆盖漏洞（`fold_fx_fields` 已经是这种
/// 局部结构哈希，它的存在正是因为字节哈希覆盖不到"解析结果"这一层）。
/// 归一化字节哈希保住了自动完备性，同时消掉唯一已知的假阳性来源。
///
/// **值不变**：仓库内所有 `.ron` 均为 LF（CR=0），故本函数在规范内容上是
/// 恒等变换 —— 既有 golden 的 `materials_fp`/`scenario_fp` 行**无需重录**，
/// CRLF 机器是向既有 LF 值收敛。
pub fn normalize_for_fingerprint(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().copied().filter(|&b| b != b'\r').collect()
}

/// 返回（材料表，文件内容指纹）。
pub fn load_materials(path: &str) -> Result<(MaterialTable, u64), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读 {path} 失败：{e}"))?;
    let fp = xxh3_64(&normalize_for_fingerprint(&bytes));
    let text = String::from_utf8(bytes).map_err(|e| format!("{path} 不是 UTF-8：{e}"))?;
    // IMPLICIT_SOME（M2 Task 1）：`dispersion`/`decay_to` 为区分"未声明"用了
    // `Option`，RON 默认要求写 `Some(...)`——在解析端开扩展，数据文件照旧写
    // 裸值（`dispersion: 5`），作者格式不变。
    let file: MaterialsFile = ron::Options::default()
        .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
        .from_str(&text)
        .map_err(|e| format!("解析 {path} 失败：{e}"))?;
    // 二遍解析（M2 Task 1）：先建 名→id 映射，供 `decay_to` 回填——core 边界
    // 只见 id，字符串引用全部死在加载期（spec §2.4 契约 2 同款纪律）。
    let name_to_id: std::collections::BTreeMap<&str, u8> =
        file.materials.iter().map(|m| (m.name.as_str(), m.id)).collect();
    let defs = file
        .materials
        .iter()
        .map(|m| -> Result<MaterialDef, String> {
            let category = match m.category {
                CatSpec::Static => Category::Static,
                CatSpec::Powder => Category::Powder,
                CatSpec::Liquid => Category::Liquid,
                CatSpec::Gas => Category::Gas,
            };
            // Gas 禁声明 dispersion（M2 spec §3.1 审阅补漏）：气体水平扩散恒
            // 1 格、根本不读该字段，声明了即配置错误——r_gas = 1 的写域论证
            // （spec §6.1）依赖这一条，体例同 fire_hp/lifetime 互斥。
            if category == Category::Gas && m.dispersion.is_some() {
                return Err(format!(
                    "材料 '{}'（id={}）是 Gas 却声明了 dispersion——气体水平扩散恒 1 格，不读该字段",
                    m.name, m.id
                ));
            }
            let fire_chance = quantize_fire_chance(m.fire_chance).map_err(|e| {
                format!("材料 '{}'（id={}）的 fire_chance 非法：{e}", m.name, m.id)
            })?;
            let flame_to = match &m.flame_to {
                None if fire_chance > 0 => {
                    return Err(format!(
                        "材料 '{}'（id={}）声明了 fire_chance 却没有 flame_to——产火产物必须显式声明（加载期契约）",
                        m.name, m.id
                    ));
                }
                None => sand_core::MAT_AIR,
                Some(name) => *name_to_id.get(name.as_str()).ok_or(format!(
                    "材料 '{}'（id={}）的 flame_to 引用不存在的材质 '{name}'（加载期显式报错）",
                    m.name, m.id
                ))?,
            };
            let decay_to = match &m.decay_to {
                None => sand_core::MAT_AIR,
                Some(name) => *name_to_id.get(name.as_str()).ok_or(format!(
                    "材料 '{}'（id={}）的 decay_to 引用不存在的材质 '{name}'（加载期显式报错，不静默丢弃）",
                    m.name, m.id
                ))?,
            };
            Ok(MaterialDef {
                id: m.id,
                name: m.name.clone(),
                category,
                density: m.density,
                color: m.color,
                hp: m.hp,
                durability: m.durability,
                vaporize_threshold: quantize_vaporize_threshold(m.vaporize_threshold).map_err(|e| {
                    format!("材料 '{}'（id={}）的 vaporize_threshold 非法：{e}", m.name, m.id)
                })?,
                dispersion: validate_dispersion(m.dispersion.unwrap_or(1)).map_err(|e| {
                    format!("材料 '{}'（id={}）的 dispersion 非法：{e}", m.name, m.id)
                })?,
                splash_chance: quantize_splash_chance(m.splash_chance).map_err(|e| {
                    format!("材料 '{}'（id={}）的 splash_chance 非法：{e}", m.name, m.id)
                })?,
                tags: m.tags.clone(),
                ignition_temp: m.ignition_temp,
                fire_temp: m.fire_temp,
                fire_hp: m.fire_hp,
                lifetime: m.lifetime,
                decay_to,
                requires_oxygen: m.requires_oxygen,
                extinguisher: m.extinguisher,
                fire_chance,
                flame_to,
                rise_chance: quantize_rise_chance(m.rise_chance).map_err(|e| {
                    format!("材料 '{}'（id={}）的 rise_chance 非法：{e}", m.name, m.id)
                })?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok((MaterialTable::new(defs)?, fp))
}

/// 液体色散距离的加载期取值域校验（Layer G Task 1，spec §3.1）：
/// `1 <= d <= DISPERSION_MAX`。
///
/// 下界 1 而非 0——0 意为"一格都不许横移"，与缺省语义（1）差一格却毫无用途，
/// 出现即配置事故。上界是 [`DISPERSION_MAX`]：越界会撑爆 spec §5 的
/// `r <= HALO` 不等式。
///
/// **与 `blast_cost`/`vaporize_threshold` 的先例不同**，core 侧并非"不做取值
/// 校验"：`rules::side` 另有 clamp 兜底（见 [`DISPERSION_MAX`] 文档）。两道
/// 防线分工明确——这里给用户可读的报错（配错了要知道错在哪），core 那道保证
/// 即便绕过本函数直接构表也不会破坏 P4 写域论证。
pub fn validate_dispersion(d: u8) -> Result<u8, String> {
    if !(1..=DISPERSION_MAX).contains(&d) {
        return Err(format!(
            "dispersion={d} 超出取值域 [1, {DISPERSION_MAX}]（0 无意义；\
             超上界会撑爆 r <= HALO 影响半径契约）"
        ));
    }
    Ok(d)
}

/// 近心汽化阈值 RON 表面值（`0.0..=1.0` 十进制）→ 量化 u8（spec §6 汽化
/// 小节，用户裁决 2026-08-30）：`v * 255.0` 四舍五入到最近整数即目标字节。
/// **round 语义与范围校验**均照抄 [`quantize_fx`] 的先例（同一份"I/O 边界
/// 唯一允许浮点运算"纪律）：`f64::round` 对 `.5` 边界走"绝对值远离零"，
/// 结果必须落在 `[0, 255]`（对应输入必须落在约 `[-0.00196, 1.00196]`，
/// 但校验对象是 **round 之后的值**而非输入本身——同 `quantize_fx` 文档
/// 阐述的理由：避免"输入在近似阈值附近、round 后才越界"的边界疏漏）。
/// 撞击溅射概率的加载期量化（Layer G Task 3，spec §6.2）：`0.0..=1.0` →
/// `0..=255`，`×255 round`。与 [`quantize_vaporize_threshold`] 同一套数学，
/// 只是报错文案与语义不同——**没有合并成一个泛用函数**是有意的：两者的取值
/// 域含义（阈值 vs 概率）与缺省端点（1.0 = 永不汽化 vs 0.0 = 永不溅射）都
/// 相反，合并后报错信息会退化成"某个 0..1 字段错了"，排查成本反而更高。
pub fn quantize_splash_chance(v: f64) -> Result<u8, String> {
    if !v.is_finite() {
        return Err(format!("splash_chance 量化失败：{v} 不是有限数"));
    }
    let raw = (v * 255.0).round();
    if !(0.0..=255.0).contains(&raw) {
        return Err(format!(
            "splash_chance 量化失败：{v} 超出 [0.0, 1.0] 可表示范围（四舍五入后 raw={raw}，需落在 [0, 255]）"
        ));
    }
    Ok(raw as u8)
}

/// 产火概率的加载期量化（M2 spec §5.3）：`0.0..=1.0` → `0..=255`，×255 round。
/// 与 splash/vaporize 同一套数学、不合并——理由见 [`quantize_splash_chance`]
/// 上方的文档（报错文案与缺省端点语义各不相同）。
pub fn quantize_fire_chance(v: f64) -> Result<u8, String> {
    if !v.is_finite() {
        return Err(format!("fire_chance 量化失败：{v} 不是有限数"));
    }
    let raw = (v * 255.0).round();
    if !(0.0..=255.0).contains(&raw) {
        return Err(format!(
            "fire_chance 量化失败：{v} 超出 [0.0, 1.0] 可表示范围（四舍五入后 raw={raw}，需落在 [0, 255]）"
        ));
    }
    Ok(raw as u8)
}

/// 气体上浮概率的加载期量化（spec §5.3.1 第 4 条）：同数学、不合并，理由同上。
pub fn quantize_rise_chance(v: f64) -> Result<u8, String> {
    if !v.is_finite() {
        return Err(format!("rise_chance 量化失败：{v} 不是有限数"));
    }
    let raw = (v * 255.0).round();
    if !(0.0..=255.0).contains(&raw) {
        return Err(format!(
            "rise_chance 量化失败：{v} 超出 [0.0, 1.0] 可表示范围（四舍五入后 raw={raw}，需落在 [0, 255]）"
        ));
    }
    Ok(raw as u8)
}

pub fn quantize_vaporize_threshold(v: f64) -> Result<u8, String> {
    if !v.is_finite() {
        return Err(format!("vaporize_threshold 量化失败：{v} 不是有限数"));
    }
    let raw = (v * 255.0).round();
    if !(0.0..=255.0).contains(&raw) {
        return Err(format!(
            "vaporize_threshold 量化失败：{v} 超出 [0.0, 1.0] 可表示范围（四舍五入后 raw={raw}，需落在 [0, 255]）"
        ));
    }
    Ok(raw as u8)
}

// ---------- reactions.ron（M2 spec §2.4）----------

#[derive(Deserialize)]
struct ReactionsFile {
    reactions: Vec<ReactionSpec>,
}

/// 反应的 RON 表面形式：`input` 两项可写材质名或 `[tag]`，`output` 只许具体
/// 材质名（词缀展开列入 Non-goals：解析失败是静默的，违反"加载期显式报错"
/// 纪律，spec §1.4）。
#[derive(Deserialize)]
struct ReactionSpec {
    /// `Vec` 而非 `[String; 2]`：serde 定长数组在 RON 里是元组语法 `(..)`，
    /// 而作者格式定的是列表 `["water", "fire"]`（spec §2.4）——长度在
    /// [`load_reactions`] 显式校验为 2。
    input: Vec<String>,
    output: Vec<String>,
    probability: f64,
}

/// 反应概率的加载期量化（spec §2.4 契约 4）：`0.0..=1.0` → `0..=255`，
/// ×255 round。与 splash/vaporize/fire_chance 同数学、不合并（同一条理由：
/// 报错文案与语义各不相同）。
pub fn quantize_reaction_probability(v: f64) -> Result<u8, String> {
    if !v.is_finite() {
        return Err(format!("probability 量化失败：{v} 不是有限数"));
    }
    let raw = (v * 255.0).round();
    if !(0.0..=255.0).contains(&raw) {
        return Err(format!(
            "probability 量化失败：{v} 超出 [0.0, 1.0] 可表示范围（四舍五入后 raw={raw}，需落在 [0, 255]）"
        ));
    }
    Ok(raw as u8)
}

/// input 一侧展开为 id 集合：材质名 → 单元素；`[tag]` → 成员 id **升序**
/// （定序，spec §6.2：tag 展开顺序按 id）。引用不存在的材质或 tag ⇒ Err
/// （spec §2.4 契约 1——Noita 对 unknown 是静默丢弃整条反应，那是给 mod 的
/// 容错，对我们是双端反应表不一致 → 分叉（P5），必须反着抄）。
fn expand_reaction_side(name: &str, table: &MaterialTable) -> Result<Vec<u8>, String> {
    if let Some(tag) = name.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
        let members: Vec<u8> = (0..table.len() as u8)
            .filter(|&id| table.tags_of(id).iter().any(|t| t == tag))
            .collect();
        if members.is_empty() {
            return Err(format!("反应引用的 tag '[{tag}]' 没有任何材质成员（加载期显式报错）"));
        }
        Ok(members)
    } else {
        Ok(vec![
            table.id_by_name(name).ok_or(format!("反应引用不存在的材质 '{name}'（加载期显式报错）"))?,
        ])
    }
}

/// 返回（反应表，文件内容指纹）。加载期完成全部四条契约（spec §2.4）：
/// ① 未知引用报错；② tag 展开成扁平 id 表，core 侧零字符串；③ 发起方规范化
/// `id_a < id_b` 且正反只注册一次（原型 `reaction.py:44-46` 正反双向注册是
/// 总纲警告的双结算来源，此处显式修正）；④ 概率一次性量化为整数阈值。
pub fn load_reactions(path: &str, table: &MaterialTable) -> Result<(ReactionTable, u64), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读 {path} 失败：{e}"))?;
    let fp = xxh3_64(&normalize_for_fingerprint(&bytes));
    let text = String::from_utf8(bytes).map_err(|e| format!("{path} 不是 UTF-8：{e}"))?;
    let file: ReactionsFile =
        ron::from_str(&text).map_err(|e| format!("解析 {path} 失败：{e}"))?;
    let mut rules = Vec::new();
    for (si, spec) in file.reactions.iter().enumerate() {
        let ctx = |e: String| format!("反应条目 #{si}：{e}");
        if spec.input.len() != 2 || spec.output.len() != 2 {
            return Err(ctx(format!(
                "input/output 必须各恰好 2 项（实际 {}/{}）——三元反应列入 Non-goals（spec §1.4）",
                spec.input.len(),
                spec.output.len()
            )));
        }
        for o in &spec.output {
            if o.starts_with('[') {
                return Err(ctx(format!("output 不许写 tag（'{o}'）——产物必须是具体材质名（spec §1.4）")));
            }
        }
        let out_a = table.id_by_name(&spec.output[0]).ok_or(ctx(format!(
            "output 引用不存在的材质 '{}'（加载期显式报错）",
            spec.output[0]
        )))?;
        let out_b = table.id_by_name(&spec.output[1]).ok_or(ctx(format!(
            "output 引用不存在的材质 '{}'（加载期显式报错）",
            spec.output[1]
        )))?;
        let threshold = quantize_reaction_probability(spec.probability).map_err(ctx)?;
        let side_a = expand_reaction_side(&spec.input[0], table).map_err(ctx)?;
        let side_b = expand_reaction_side(&spec.input[1], table).map_err(ctx)?;
        if side_a.len() == 1 && side_b.len() == 1 && side_a[0] == side_b[0] {
            return Err(ctx(format!(
                "自反应（'{}' + 自身）不受支持——发起方约定 id_a < id_b 天然排除（spec §1.4）",
                spec.input[0]
            )));
        }
        // 展开序确定：side_a 外层、side_b 内层，两侧都按 id 升序；tag 自交
        // （a == b）静默跳过——那是 [burnable]×[burnable] 这类笛卡尔积的预期
        // 副产物，与上面"显式同名对报错"不同。
        for &a in &side_a {
            for &b in &side_b {
                if a == b {
                    continue;
                }
                rules.push(if a < b {
                    ReactionRule { a, b, out_a, out_b, threshold }
                } else {
                    ReactionRule { a: b, b: a, out_a: out_b, out_b: out_a, threshold }
                });
            }
        }
    }
    Ok((ReactionTable::new(table, rules)?, fp))
}

// ---------- 场景文件 ----------

#[derive(Deserialize)]
#[serde(rename = "Scenario")]
pub struct ScenarioFile {
    pub name: String,
    /// (width_chunks, height_chunks)
    pub world: (usize, usize),
    pub seed: u64,
    pub ticks: u64,
    #[serde(default)]
    pub setup: Vec<OpSpec>,
    #[serde(default)]
    pub script: Vec<ScriptEntry>,
}

#[derive(Deserialize, Clone)]
pub enum OpSpec {
    Brush { material: String, x: i32, y: i32, r: i32 },
    Fill { material: String, x0: i32, y0: i32, x1: i32, y1: i32 },
    /// `Op::Emit` 的 RON 表面形式（spec §7）：坐标/速度/抖动幅度写十进制小数，
    /// 加载期经 [`quantize_fx`] 一次性 round 量化为 Q16.16——core 边界只见
    /// `Fx`，量化在 I/O 层完成，不碰核心零浮点红线。
    Emit { material: String, x: f64, y: f64, vx: f64, vy: f64, count: u16, jitter: f64 },
    /// `Op::Explode` 的 RON 表面形式（spec §6）：`x/y/r/power` 全部是整数
    /// （圆心格坐标、半径格数、初始能量），无需量化——core 侧 `Op::Explode`
    /// 本就是纯整数签名，这里原样透传。`max_durability` 缺省 **10**（对齐
    /// Noita `ConfigExplosion.max_durability_to_destroy` 默认值，M2 spec §2.2）。
    Explode {
        x: i32,
        y: i32,
        r: i32,
        power: u32,
        #[serde(default = "default_max_durability")]
        max_durability: u8,
    },
}

/// 场景 RON 里的十进制小数 → Q16.16 定点（`Fx`），**round**（非截断）语义：
/// `v * 65536.0` 四舍五入到最近整数即 raw 位模式。`f64::round` 对 `.5` 边界
/// 走"绝对值远离零"舍入（如 `0.5 → 32768`，恰好是整数点，不受舍入方向
/// 影响——用作本函数的金值锚点，见 `quantize_fx_exact_half_rounds_to_0x8000`
/// 单测）。这是 I/O 边界上唯一允许出现浮点运算的地方：结果一旦落地为
/// `Fx`，core 侧全程整数运算（charter §6 混合数值制原判）。
///
/// **范围校验**（Task 5 修复轮 1 Minor 3）：`v` 必须是有限数，且
/// `v * 65536.0` 四舍五入后必须落在 `i32` 可表示范围内——否则 `as i32`
/// 转换会静默 wrapping，产出一个符号/量级都错误的 `Fx` 而不报错，把配置
/// 错误伪装成合法输入，一路腐化到后续所有确定性计算。直接对 `round()`
/// 之后的值判界（而非对 `v` 本身做近似的"±32768"判界），避免"`v` 在近似
/// 阈值附近、`round` 后才越界"的边界疏漏。
pub fn quantize_fx(v: f64) -> Result<Fx, String> {
    if !v.is_finite() {
        return Err(format!("Fx 量化失败：{v} 不是有限数"));
    }
    let raw = (v * 65536.0).round();
    if raw < i32::MIN as f64 || raw > i32::MAX as f64 {
        return Err(format!(
            "Fx 量化失败：{v} 超出 Q16.16 可表示范围（四舍五入后 raw={raw}，\
             需落在 [{}, {}]）",
            i32::MIN,
            i32::MAX
        ));
    }
    Ok(Fx(raw as i32))
}

/// `OpSpec::Explode::max_durability` 的 serde 缺省（M2 spec §2.2）。
fn default_max_durability() -> u8 {
    10
}

#[derive(Deserialize, Clone)]
pub enum ScriptEntry {
    At { tick: u64, op: OpSpec },
    Every { from: u64, until: u64, step: u64, op: OpSpec },
}

pub struct Scenario {
    pub name: String,
    pub world: (usize, usize),
    pub seed: u64,
    pub ticks: u64,
    pub setup: Vec<Op>,
    pub script: Vec<(ScheduleKind, Op)>,
    /// 场景文件内容指纹。
    pub fingerprint: u64,
}

pub enum ScheduleKind {
    At(u64),
    Every { from: u64, until: u64, step: u64 },
}

fn resolve_op(spec: &OpSpec, table: &MaterialTable) -> Result<Op, String> {
    let id = |name: &str| {
        table.id_by_name(name).ok_or_else(|| format!("场景引用未知材料 '{name}'"))
    };
    Ok(match spec {
        OpSpec::Brush { material, x, y, r } => {
            Op::Brush { material: id(material)?, x: *x, y: *y, r: *r }
        }
        OpSpec::Fill { material, x0, y0, x1, y1 } => {
            Op::Fill { material: id(material)?, x0: *x0, y0: *y0, x1: *x1, y1: *y1 }
        }
        OpSpec::Emit { material, x, y, vx, vy, count, jitter } => {
            if *jitter < 0.0 {
                return Err(format!("Emit jitter 必须非负，实际 {jitter}"));
            }
            let jitter_fx = quantize_fx(*jitter)?;
            // core 的 emit_jitter 定点重缩放只在 |raw| <= MAX_EMIT_JITTER_RAW
            // 时安全（见该常量文档：越界会静默 wrapping）；在加载期挡住而不是
            // 留到运行期靠 debug_assert 兜底——release 构建没有 debug_assert，
            // 场景配置错误应该在这里报错，不该悄悄腐化到仿真里（Task 5
            // 修复轮 1 Minor 1）。
            if jitter_fx.0 > MAX_EMIT_JITTER_RAW {
                return Err(format!(
                    "Emit jitter 超出安全范围：{jitter} 量化后 raw={}，上限 {MAX_EMIT_JITTER_RAW}\
                     （emit_jitter 的定点重缩放会溢出）",
                    jitter_fx.0
                ));
            }
            Op::Emit {
                material: id(material)?,
                x: quantize_fx(*x)?,
                y: quantize_fx(*y)?,
                vx: quantize_fx(*vx)?,
                vy: quantize_fx(*vy)?,
                count: *count,
                jitter: jitter_fx,
            }
        }
        OpSpec::Explode { x, y, r, power, max_durability } => {
            // 加载期范围校验（终审修复波）：`Op::Explode` 是纯整数签名，不经
            // quantize_fx，但 core 侧仍会静默腐化越界输入——`fire_ray` 用
            // `Fx::from_int(x)`/`Fx::from_int(y)` 把爆心转成 Fx（world.rs
            // :269），`|v| >= 32768` 时左移 16 位溢出 i32、静默 wrapping；
            // `power as i32`（world.rs :289 `speed_ratio` 分母）在
            // `power > i32::MAX` 时静默翻号。两端一致腐化不破坏确定性
            // （SyncTest 不会报警），但把配置错误伪装成合法输入一路带进
            // 仿真，属静默垃圾语义——体例仿 MAX_EMIT_JITTER_RAW，在加载期
            // 拦住而非留给运行期。
            if x.unsigned_abs() >= 32768 || y.unsigned_abs() >= 32768 {
                return Err(format!(
                    "Explode 坐标越界：(x={x}, y={y})，需满足 |x|<32768 且 |y|<32768\
                     （Fx::from_int 的安全域）"
                ));
            }
            if *r < 1 || *r > 32767 {
                return Err(format!("Explode 半径越界：r={r}，需 ∈ [1, 32767]"));
            }
            if *power < 1 || *power > i32::MAX as u32 {
                return Err(format!(
                    "Explode power 越界：power={power}，需 ∈ [1, {}]",
                    i32::MAX as u32
                ));
            }
            Op::Explode { x: *x, y: *y, r: *r, power: *power, max_durability: *max_durability }
        }
    })
}

/// 折叠"全部已解析 `Op` 中 `Fx` 字段"的原始位（spec §7：量化后的数值必须
/// 入场景指纹）。目前只有 `Op::Emit` 携带 `Fx` 字段，其余变体（`Brush`/
/// `Fill`/`Explode`）是纯整数、不折叠——它们的文本已经被 [`load_scenario`]
/// 里的源字节哈希覆盖，这里不重复。
///
/// **为什么不能只靠源字节哈希**（Task 5 修复轮 1 I2）：`load_scenario` 原先
/// 的 `fingerprint = xxh3_64(原始文件字节)` 只保证"文本变了指纹就变"，但
/// spec §7 的意图是握手指纹要能拦住"两端实际喂给仿真的 `Fx` 不同"——如果
/// 未来 RON/serde 的浮点解析行为在不同版本/平台间出现哪怕极细微的差异
/// （即使文本字节完全相同），源字节哈希对此完全不可见，两端会各自量化出
/// 不同的 `Fx` 却顶着相同指纹握手通过。折叠已解析的 `Fx` raw 位是对
/// "仿真实际消费的值"的直接断言，而不是对"喂给解析器的文本"的间接断言。
fn fold_fx_fields<'a>(ops: impl Iterator<Item = &'a Op>) -> u64 {
    let mut h = Xxh3::new();
    for op in ops {
        if let Op::Emit { x, y, vx, vy, jitter, .. } = op {
            h.update(&x.0.to_le_bytes());
            h.update(&y.0.to_le_bytes());
            h.update(&vx.0.to_le_bytes());
            h.update(&vy.0.to_le_bytes());
            h.update(&jitter.0.to_le_bytes());
        }
    }
    h.digest()
}

pub fn load_scenario(path: &str, table: &MaterialTable) -> Result<Scenario, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读 {path} 失败：{e}"))?;
    let source_fp = xxh3_64(&normalize_for_fingerprint(&bytes));
    let text = String::from_utf8(bytes).map_err(|e| format!("{path} 不是 UTF-8：{e}"))?;
    let file: ScenarioFile =
        ron::from_str(&text).map_err(|e| format!("解析 {path} 失败：{e}"))?;
    let setup = file
        .setup
        .iter()
        .map(|s| resolve_op(s, table))
        .collect::<Result<Vec<_>, _>>()?;
    let script = file
        .script
        .iter()
        .map(|e| {
            Ok(match e {
                ScriptEntry::At { tick, op } => (ScheduleKind::At(*tick), resolve_op(op, table)?),
                ScriptEntry::Every { from, until, step, op } => (
                    ScheduleKind::Every { from: *from, until: *until, step: (*step).max(1) },
                    resolve_op(op, table)?,
                ),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    // 指纹 = combine(源字节哈希, 全部已解析 Emit 的 Fx raw 位定序折叠)
    // （spec §7；Task 5 修复轮 1 I2）。定序：setup 先、script 后，各自按
    // 声明序——与 `ops_for_tick` 的确定性遍历序一致，不引入新的排序歧义。
    let fx_fp = fold_fx_fields(setup.iter().chain(script.iter().map(|(_, op)| op)));
    let mut combined = Xxh3::new();
    combined.update(&source_fp.to_le_bytes());
    combined.update(&fx_fp.to_le_bytes());
    let fingerprint = combined.digest();

    Ok(Scenario {
        name: file.name,
        world: file.world,
        seed: file.seed,
        ticks: file.ticks,
        setup,
        script,
        fingerprint,
    })
}

impl Scenario {
    /// 本 tick 的输入操作，按脚本声明序（确定性）。
    pub fn ops_for_tick(&self, tick: u64) -> Vec<Op> {
        self.script
            .iter()
            .filter(|(k, _)| match k {
                ScheduleKind::At(t) => *t == tick,
                ScheduleKind::Every { from, until, step } => {
                    tick >= *from && tick < *until && (tick - from) % step == 0
                }
            })
            .map(|(_, op)| op.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sand_core::{Category, MaterialDef, MaterialTable};

    fn table_with_water() -> MaterialTable {
        MaterialTable::new(vec![
            MaterialDef { hp: 0, ..MaterialDef::base(0, "air", Category::Static, 0) },
            MaterialDef { hp: 100, durability: 15, ..MaterialDef::base(1, "wall", Category::Static, 100) },
            MaterialDef { hp: 1, ..MaterialDef::base(2, "water", Category::Liquid, 16) },
        ])
        .unwrap()
    }

    // ==================== quantize_fx：round 金值（任务书要求）====================

    #[test]
    fn quantize_fx_exact_half_rounds_to_0x8000() {
        assert_eq!(quantize_fx(0.5).unwrap().0, 0x8000);
        assert_eq!(quantize_fx(-0.5).unwrap().0, -0x8000);
    }

    #[test]
    fn quantize_fx_integers_shift_exactly() {
        assert_eq!(quantize_fx(0.0).unwrap().0, 0);
        assert_eq!(quantize_fx(1.0).unwrap().0, 0x1_0000);
        assert_eq!(quantize_fx(-2.0).unwrap().0, -0x2_0000);
    }

    #[test]
    fn quantize_fx_rounds_nearest_not_truncates() {
        // 0.0001 * 65536 = 6.5536 → round 到 7；若实现是截断会给 6，
        // 这条测试能把两者的实现差异暴露出来。
        assert_eq!(quantize_fx(0.0001).unwrap().0, 7);
    }

    // ==================== quantize_fx：范围/有限性校验（修复轮 1 Minor 3）====================

    #[test]
    fn quantize_fx_rejects_non_finite() {
        assert!(quantize_fx(f64::NAN).is_err());
        assert!(quantize_fx(f64::INFINITY).is_err());
        assert!(quantize_fx(f64::NEG_INFINITY).is_err());
    }

    #[test]
    fn quantize_fx_accepts_value_just_inside_i32_range() {
        // (i32::MAX as f64) / 65536.0 四舍五入后应恰好落在 i32::MAX，不报错。
        let v = i32::MAX as f64 / 65536.0;
        assert_eq!(quantize_fx(v).unwrap().0, i32::MAX);
    }

    #[test]
    fn quantize_fx_rejects_value_overflowing_i32_after_round() {
        // 32768.0 * 65536.0 = 2147483648.0 = i32::MAX + 1，越界。
        assert!(quantize_fx(32768.0).is_err());
        assert!(quantize_fx(-32769.0).is_err());
        assert!(quantize_fx(1.0e30).is_err());
    }

    // ==================== resolve_op：Explode 整数透传（M1 Task 6）====================

    #[test]
    fn resolve_op_explode_passes_integers_through_unquantized() {
        let t = table_with_water();
        let spec = OpSpec::Explode { x: 100, y: 50, r: 12, power: 40, max_durability: 10 };
        let op = resolve_op(&spec, &t).unwrap();
        match op {
            Op::Explode { x, y, r, power, max_durability } => {
                assert_eq!((x, y, r, power, max_durability), (100, 50, 12, 40, 10));
            }
            other => panic!("期望 Op::Explode，实际 {other:?}"),
        }
    }

    // ==================== resolve_op：Explode 加载期范围校验（终审修复波）====================

    #[test]
    fn resolve_op_rejects_explode_radius_out_of_range() {
        let t = table_with_water();
        assert!(
            resolve_op(&OpSpec::Explode { x: 0, y: 0, r: 0, power: 10, max_durability: 10 }, &t).is_err(),
            "r=0（低于下限 1）必须被拒绝"
        );
        assert!(
            resolve_op(&OpSpec::Explode { x: 0, y: 0, r: 32768, power: 10, max_durability: 10 }, &t).is_err(),
            "r=32768（超出上限 32767）必须被拒绝"
        );
    }

    #[test]
    fn resolve_op_rejects_explode_power_out_of_range() {
        let t = table_with_water();
        assert!(
            resolve_op(&OpSpec::Explode { x: 0, y: 0, r: 1, power: 0, max_durability: 10 }, &t).is_err(),
            "power=0（低于下限 1）必须被拒绝"
        );
        assert!(
            resolve_op(
                &OpSpec::Explode { x: 0, y: 0, r: 1, power: i32::MAX as u32 + 1, max_durability: 10 },
                &t
            )
            .is_err(),
            "power > i32::MAX（`power as i32` 会静默翻号）必须被拒绝"
        );
    }

    // ==================== MatSpec：hp 缺省 1（原 blast_cost，M2 spec §2.2）====================

    #[test]
    fn materials_ron_without_hp_field_defaults_to_one() {
        let ron = "(materials:[\
            (id:0,name:\"air\",category:Static,density:0,color:(0,0,0)),\
            (id:1,name:\"wall\",category:Static,density:100,color:(0,0,0)),\
        ])";
        let path = std::env::temp_dir().join(format!(
            "sand_harness_test_materials_default_blast_cost_{}.ron",
            std::process::id()
        ));
        std::fs::write(&path, ron).unwrap();
        let (t, _fp) = load_materials(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(t.hp(0), 1, "未声明 hp 的材料应缺省为 1");
        assert_eq!(t.hp(1), 1);
        assert_eq!(t.durability(0), 0, "未声明 durability 缺省 0");
    }

    // ==================== MatSpec：vaporize_threshold 缺省 1.0（spec §6 汽化小节，用户裁决 2026-08-30）====================

    #[test]
    fn materials_ron_without_vaporize_threshold_field_defaults_to_255() {
        let ron = "(materials:[\
            (id:0,name:\"air\",category:Static,density:0,color:(0,0,0)),\
            (id:1,name:\"wall\",category:Static,density:100,color:(0,0,0)),\
        ])";
        let path = std::env::temp_dir().join(format!(
            "sand_harness_test_materials_default_vaporize_threshold_{}.ron",
            std::process::id()
        ));
        std::fs::write(&path, ron).unwrap();
        let (t, _fp) = load_materials(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(t.vaporize_threshold(0), 255, "未声明 vaporize_threshold 的材料应缺省量化为 255（1.0=永不汽化）");
        assert_eq!(t.vaporize_threshold(1), 255);
    }

    // ==================== quantize_vaporize_threshold：round 金值（materials.ron 初值）====================

    #[test]
    fn quantize_vaporize_threshold_gold_values_match_materials_ron_seed_data() {
        // data/materials.ron 初值：water 0.4、sand 0.7（用户裁决 2026-08-30）。
        assert_eq!(quantize_vaporize_threshold(0.4).unwrap(), 102);
        assert_eq!(quantize_vaporize_threshold(0.7).unwrap(), 179);
    }

    #[test]
    fn quantize_vaporize_threshold_endpoints() {
        assert_eq!(quantize_vaporize_threshold(0.0).unwrap(), 0);
        assert_eq!(quantize_vaporize_threshold(1.0).unwrap(), 255, "缺省值 1.0 必须量化为 255（永不汽化）");
    }

    #[test]
    fn quantize_vaporize_threshold_rejects_non_finite() {
        assert!(quantize_vaporize_threshold(f64::NAN).is_err());
        assert!(quantize_vaporize_threshold(f64::INFINITY).is_err());
        assert!(quantize_vaporize_threshold(f64::NEG_INFINITY).is_err());
    }

    #[test]
    fn quantize_vaporize_threshold_rejects_negative_and_above_one() {
        assert!(quantize_vaporize_threshold(-0.01).is_err(), "负值必须被拒绝");
        assert!(quantize_vaporize_threshold(1.01).is_err(), "超过 1.0（四舍五入后 > 255）必须被拒绝");
    }

    // ==================== 指纹行尾无关性（2026-08-31 双机 hashrun 回归）====================

    /// 双机 hashrun 暴露的缺陷的回归测试：同一份内容，LF 与 CRLF 两种落盘方式
    /// **必须**算出同一个 materials_fp。改动前这条必挂（裸字节哈希）。
    #[test]
    fn materials_fingerprint_is_line_ending_agnostic() {
        let lf = "(materials:[\n                  (id:0,name:\"air\",category:Static,density:0,color:(0,0,0)),\n                  (id:1,name:\"wall\",category:Static,density:100,color:(0,0,0)),\n                  ])";
        let crlf = lf.replace('\n', "\r\n");
        assert_ne!(lf.as_bytes(), crlf.as_bytes(), "两份输入的字节必须真的不同，否则测试没意义");

        let write = |tag: &str, body: &str| {
            let p = std::env::temp_dir()
                .join(format!("sand_harness_fp_eol_{}_{tag}.ron", std::process::id()));
            std::fs::write(&p, body).unwrap();
            p
        };
        let (pa, pb) = (write("lf", lf), write("crlf", &crlf));
        let (_, fp_lf) = load_materials(pa.to_str().unwrap()).unwrap();
        let (_, fp_crlf) = load_materials(pb.to_str().unwrap()).unwrap();
        std::fs::remove_file(&pa).ok();
        std::fs::remove_file(&pb).ok();

        assert_eq!(
            fp_lf, fp_crlf,
            "materials_fp 必须与行尾无关——否则同一 commit 在 CRLF 平台上握手指纹不同 \
             （2026-08-31 双机 hashrun 实测缺陷：仿真逐位一致、只有 fp 对不上）"
        );
    }

    /// 同上，覆盖 `load_scenario` 那条独立的指纹路径（`source_fp`）。
    #[test]
    fn scenario_fingerprint_is_line_ending_agnostic() {
        let t = table_with_water();
        let lf = "Scenario(name:\"t\",world:(1,1),seed:0,ticks:1,setup:[],\n                  script:[At(tick:0,op:Brush(material:\"water\",x:1,y:1,r:1))])";
        let crlf = lf.replace('\n', "\r\n");
        let write = |tag: &str, body: &str| {
            let p = std::env::temp_dir()
                .join(format!("sand_harness_scfp_eol_{}_{tag}.ron", std::process::id()));
            std::fs::write(&p, body).unwrap();
            p
        };
        let (pa, pb) = (write("lf", lf), write("crlf", &crlf));
        let a = load_scenario(pa.to_str().unwrap(), &t).unwrap();
        let b = load_scenario(pb.to_str().unwrap(), &t).unwrap();
        std::fs::remove_file(&pa).ok();
        std::fs::remove_file(&pb).ok();
        assert_eq!(a.fingerprint, b.fingerprint, "scenario_fp 必须与行尾无关");
    }

    /// **值不变**断言：仓库内 `.ron` 全为 LF，故归一化在规范内容上是恒等变换。
    /// 这条守住"CRLF 机器向既有 LF 值收敛，而不是双方都换新值"——它正是
    /// golden 无需重录的依据。
    #[test]
    fn normalize_is_identity_on_lf_content() {
        let lf = b"a\nb\nc\n";
        assert_eq!(normalize_for_fingerprint(lf), lf.to_vec());
        assert_eq!(
            normalize_for_fingerprint(b"a\r\nb\r\nc\r\n"),
            lf.to_vec(),
            "CRLF 应归一化到与 LF 完全相同的字节序列"
        );
    }

    // ==================== dispersion 加载期校验（Layer G Task 1，spec §3.1）====================

    fn write_temp_materials(tag: &str, water_line: &str) -> std::path::PathBuf {
        let ron = format!(
            "(materials:[\
             (id:0,name:\"air\",category:Static,density:0,color:(0,0,0)),\
             (id:1,name:\"wall\",category:Static,density:100,color:(0,0,0)),\
             {water_line}\
             ])"
        );
        let path = std::env::temp_dir()
            .join(format!("sand_harness_test_dispersion_{}_{tag}.ron", std::process::id()));
        std::fs::write(&path, ron).unwrap();
        path
    }

    #[test]
    fn materials_ron_without_dispersion_field_defaults_to_one() {
        let path = write_temp_materials("default", "");
        let (t, _fp) = load_materials(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(t.dispersion(0), 1, "未声明 dispersion 的材料应缺省为 1（= 改动前语义）");
        assert_eq!(t.dispersion(1), 1);
    }

    #[test]
    fn materials_ron_accepts_dispersion_within_range() {
        let path = write_temp_materials(
            "ok",
            "(id:2,name:\"water\",category:Liquid,density:16,color:(0,0,0),dispersion:5),",
        );
        let (t, _fp) = load_materials(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(t.dispersion(2), 5);
    }

    #[test]
    fn load_materials_rejects_dispersion_zero() {
        // 0 = "一格都不许走"，是配置事故而非合法语义（缺省是 1）。
        let path = write_temp_materials(
            "zero",
            "(id:2,name:\"water\",category:Liquid,density:16,color:(0,0,0),dispersion:0),",
        );
        let r = load_materials(path.to_str().unwrap());
        std::fs::remove_file(&path).ok();
        assert!(r.is_err(), "dispersion=0 必须在加载期被拒绝");
    }

    // ==================== M2 Task 1：新字段加载契约（spec §2.1/§3.1）====================

    #[test]
    fn m2_fields_default_safe_on_legacy_materials_ron() {
        // 未声明任何 M2 字段的旧式表：全部字段退化为"改动前行为"的缺省值。
        let path = write_temp_materials("m2_defaults", "");
        let (t, _fp) = load_materials(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(&path).ok();
        for id in [0u8, 1u8] {
            assert_eq!(t.fire_hp(id), 0, "缺省不可燃");
            assert_eq!(t.lifetime(id), 0);
            assert_eq!(t.ignition_temp(id), 100);
            assert_eq!(t.fire_temp(id), 10);
            assert_eq!(t.decay_to(id), 0, "缺省 decay_to = air");
            assert!(t.requires_oxygen(id));
            assert!(!t.extinguisher(id));
            assert_eq!(t.fire_chance(id), 0);
            assert!(t.tags_of(id).is_empty());
        }
    }

    #[test]
    fn gas_material_declaring_dispersion_is_rejected() {
        // spec §3.1 审阅补漏：气体水平扩散恒 1 格、不读 dispersion——声明即
        // 配置错误（r_gas = 1 的写域论证依赖这一条）。
        let path = write_temp_materials(
            "gas_disp",
            "(id:2,name:\"smoke\",category:Gas,density:2,color:(0,0,0),dispersion:3),",
        );
        let r = load_materials(path.to_str().unwrap());
        std::fs::remove_file(&path).ok();
        assert!(r.is_err(), "Gas + dispersion 必须在加载期被拒绝");
    }

    #[test]
    fn gas_material_without_dispersion_loads() {
        let path = write_temp_materials(
            "gas_ok",
            "(id:2,name:\"smoke\",category:Gas,density:2,color:(0,0,0),lifetime:200),",
        );
        let (t, _fp) = load_materials(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(t.category(2), sand_core::Category::Gas);
        assert_eq!(t.lifetime(2), 200);
        assert_eq!(t.dispersion(2), 1, "未声明吃缺省 1（气体运行期不读它）");
    }

    #[test]
    fn decay_to_resolves_to_id_and_rejects_unknown() {
        // 解析成 id：fire 衰变到 smoke
        let path = write_temp_materials(
            "decay_ok",
            "(id:2,name:\"smoke\",category:Gas,density:2,color:(0,0,0),lifetime:200),\
             (id:3,name:\"fire\",category:Gas,density:1,color:(0,0,0),lifetime:40,decay_to:\"smoke\"),",
        );
        let (t, _fp) = load_materials(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(t.decay_to(3), 2, "decay_to 必须在加载期解析成 id");
        // 引用不存在的材质 ⇒ 加载失败（spec §2.4 契约 1：与 Noita 的静默丢弃反着抄）
        let path = write_temp_materials(
            "decay_bad",
            "(id:2,name:\"fire\",category:Gas,density:1,color:(0,0,0),lifetime:40,decay_to:\"steam\"),",
        );
        let r = load_materials(path.to_str().unwrap());
        std::fs::remove_file(&path).ok();
        assert!(r.is_err(), "decay_to 引用不存在材质必须报错，不得静默丢弃");
    }

    #[test]
    fn declaring_both_fire_hp_and_lifetime_is_rejected_through_load() {
        // 校验在 core 的 MaterialTable::new（直接构表同样被拦），这里验证
        // harness 加载路径把错误透传出来。
        let path = write_temp_materials(
            "counter_clash",
            "(id:2,name:\"weird\",category:Static,density:5,color:(0,0,0),fire_hp:10,lifetime:10),",
        );
        let r = load_materials(path.to_str().unwrap());
        std::fs::remove_file(&path).ok();
        assert!(r.is_err(), "fire_hp 与 lifetime 同时声明必须在加载期被拒绝");
    }

    #[test]
    fn quantize_fire_chance_endpoints_and_rejects() {
        assert_eq!(quantize_fire_chance(0.0).unwrap(), 0);
        assert_eq!(quantize_fire_chance(1.0).unwrap(), 255);
        assert_eq!(quantize_fire_chance(0.6).unwrap(), 153, "materials.ron 初值口径");
        assert!(quantize_fire_chance(f64::NAN).is_err());
        assert!(quantize_fire_chance(-0.01).is_err());
        assert!(quantize_fire_chance(1.01).is_err());
    }

    #[test]
    fn load_materials_rejects_dispersion_above_max() {
        // 越界会撑爆 §5 的 r ≤ HALO 不等式；core 侧另有 clamp 兜底，但用户
        // 可见的报错必须在这里给出（体例同 MAX_EMIT_JITTER_RAW）。
        let path = write_temp_materials(
            "over",
            "(id:2,name:\"water\",category:Liquid,density:16,color:(0,0,0),dispersion:9),",
        );
        let r = load_materials(path.to_str().unwrap());
        std::fs::remove_file(&path).ok();
        assert!(r.is_err(), "dispersion=9（> DISPERSION_MAX=8）必须在加载期被拒绝");
    }

    // ==================== resolve_op：Emit 解析 + 量化落地 + 校验 ====================

    #[test]
    fn resolve_op_emit_quantizes_all_fx_fields() {
        let t = table_with_water();
        let spec = OpSpec::Emit { material: "water".into(), x: 120.0, y: 8.0, vx: 0.5, vy: 2.0, count: 3, jitter: 0.8 };
        let op = resolve_op(&spec, &t).unwrap();
        match op {
            Op::Emit { material, x, y, vx, vy, count, jitter } => {
                assert_eq!(material, 2);
                assert_eq!(x, quantize_fx(120.0).unwrap());
                assert_eq!(y, quantize_fx(8.0).unwrap());
                assert_eq!(vx, quantize_fx(0.5).unwrap());
                assert_eq!(vy, quantize_fx(2.0).unwrap());
                assert_eq!(count, 3);
                assert_eq!(jitter, quantize_fx(0.8).unwrap());
            }
            other => panic!("期望 Op::Emit，实际 {other:?}"),
        }
    }

    #[test]
    fn resolve_op_rejects_negative_jitter() {
        let t = table_with_water();
        let spec =
            OpSpec::Emit { material: "water".into(), x: 0.0, y: 0.0, vx: 0.0, vy: 0.0, count: 1, jitter: -0.1 };
        assert!(resolve_op(&spec, &t).is_err(), "负 jitter 必须被拒绝");
    }

    #[test]
    fn resolve_op_emit_unknown_material_errors() {
        let t = table_with_water();
        let spec =
            OpSpec::Emit { material: "lava".into(), x: 0.0, y: 0.0, vx: 0.0, vy: 0.0, count: 1, jitter: 0.0 };
        assert!(resolve_op(&spec, &t).is_err());
    }

    #[test]
    fn resolve_op_rejects_jitter_above_max_emit_jitter_raw() {
        let t = table_with_water();
        // (MAX_EMIT_JITTER_RAW + 1) 格值：raw 恰好比上限多 1。
        let over = (MAX_EMIT_JITTER_RAW as f64 + 1.0) / 65536.0;
        let spec =
            OpSpec::Emit { material: "water".into(), x: 0.0, y: 0.0, vx: 0.0, vy: 0.0, count: 1, jitter: over };
        assert!(resolve_op(&spec, &t).is_err(), "超过 MAX_EMIT_JITTER_RAW 的 jitter 必须被拒绝");
    }

    #[test]
    fn resolve_op_accepts_jitter_exactly_at_max_emit_jitter_raw() {
        let t = table_with_water();
        let at_bound = MAX_EMIT_JITTER_RAW as f64 / 65536.0;
        let spec =
            OpSpec::Emit { material: "water".into(), x: 0.0, y: 0.0, vx: 0.0, vy: 0.0, count: 1, jitter: at_bound };
        let op = resolve_op(&spec, &t).unwrap();
        match op {
            Op::Emit { jitter, .. } => assert_eq!(jitter.0, MAX_EMIT_JITTER_RAW),
            other => panic!("期望 Op::Emit，实际 {other:?}"),
        }
    }

    // ==================== fold_fx_fields：白盒（修复轮 1 I2）====================

    #[test]
    fn fold_fx_fields_changes_with_emit_fx_values() {
        let mk = |vx| Op::Emit {
            material: 0,
            x: Fx::ZERO,
            y: Fx::ZERO,
            vx,
            vy: Fx::ZERO,
            count: 1,
            jitter: Fx::ZERO,
        };
        let a = [mk(Fx::from_int(1))];
        let b = [mk(Fx::from_int(2))];
        assert_ne!(fold_fx_fields(a.iter()), fold_fx_fields(b.iter()));
    }

    #[test]
    fn fold_fx_fields_ignores_non_emit_ops() {
        // Brush/Fill 没有 Fx 字段，不参与这部分折叠——它们的文本变化由
        // load_scenario 的源字节哈希兜底，这里刻意不重复覆盖。
        let a = [Op::Brush { material: 2, x: 1, y: 1, r: 1 }];
        let b = [Op::Brush { material: 2, x: 9, y: 9, r: 9 }];
        assert_eq!(fold_fx_fields(a.iter()), fold_fx_fields(b.iter()));
    }

    // ==================== 场景指纹：走真实 load_scenario（修复轮 1 I2）====================
    //
    // spec §7 要求"量化后的 Fx 原始位参与场景指纹"。原实现只哈希解析前的
    // 源字节，无法拦住"源字节相同但解析出的 Fx 不同"这一类假设性跨
    // 平台/跨版本解析分叉（评审 I2）。修复后 fingerprint =
    // combine(源字节哈希, 已解析 Emit 的 Fx raw 位折叠)。这条测试写临时
    // RON 文件、走真实 `load_scenario`（而非直接摆弄字符串哈希），断言只改
    // Emit 的 vx 会改变指纹——同时也就验证了"折叠值确实被并入最终
    // fingerprint"，不是只停留在 `fold_fx_fields` 单测的白盒断言里。

    fn write_temp_scenario(tag: &str, vx: f64) -> std::path::PathBuf {
        let path = std::env::temp_dir()
            .join(format!("sand_harness_test_scenario_fp_{}_{tag}.ron", std::process::id()));
        let ron = format!(
            "Scenario(name:\"t\",world:(1,1),seed:0,ticks:1,setup:[],\
             script:[Every(from:0,until:1,step:1,\
             op:Emit(material:\"water\",x:1.0,y:1.0,vx:{vx},vy:1.0,count:1,jitter:0.1))])"
        );
        std::fs::write(&path, ron).unwrap();
        path
    }

    #[test]
    fn fingerprint_changes_when_emit_params_change_via_real_load_scenario() {
        let t = table_with_water();
        let path_a = write_temp_scenario("a", 0.5);
        let path_b = write_temp_scenario("b", 0.6);

        let sc_a = load_scenario(path_a.to_str().unwrap(), &t).unwrap();
        let sc_b = load_scenario(path_b.to_str().unwrap(), &t).unwrap();

        std::fs::remove_file(&path_a).ok();
        std::fs::remove_file(&path_b).ok();

        assert_ne!(
            sc_a.fingerprint, sc_b.fingerprint,
            "仅改 Emit 的 vx（0.5→0.6），走真实 load_scenario 的场景指纹也必须改变"
        );
        // 顺带钉死解析结果本身确实不同（指纹差异不是巧合/碰撞）。
        let vx_of = |sc: &Scenario| match sc.script[0].1 {
            Op::Emit { vx, .. } => vx,
            ref other => panic!("期望 Op::Emit，实际 {other:?}"),
        };
        assert_ne!(vx_of(&sc_a), vx_of(&sc_b));
    }
}
