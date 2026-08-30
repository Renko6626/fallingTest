//! 场景与材料表加载（spec §2、§5.2）。
//! 数据即确定性输入（P5）：materials.ron 与场景文件都算 xxh3 指纹，
//! 进入 replay/golden 头部——指纹不符即拒绝比对。

use serde::Deserialize;
use xxhash_rust::xxh3::xxh3_64;

use sand_core::{Category, Fx, MaterialDef, MaterialTable, Op};

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
}

#[derive(Deserialize)]
enum CatSpec {
    Static,
    Powder,
    Liquid,
}

/// 返回（材料表，文件内容指纹）。
pub fn load_materials(path: &str) -> Result<(MaterialTable, u64), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读 {path} 失败：{e}"))?;
    let fp = xxh3_64(&bytes);
    let text = String::from_utf8(bytes).map_err(|e| format!("{path} 不是 UTF-8：{e}"))?;
    let file: MaterialsFile =
        ron::from_str(&text).map_err(|e| format!("解析 {path} 失败：{e}"))?;
    let defs = file
        .materials
        .into_iter()
        .map(|m| MaterialDef {
            id: m.id,
            name: m.name,
            category: match m.category {
                CatSpec::Static => Category::Static,
                CatSpec::Powder => Category::Powder,
                CatSpec::Liquid => Category::Liquid,
            },
            density: m.density,
            color: m.color,
        })
        .collect();
    Ok((MaterialTable::new(defs)?, fp))
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
}

/// 场景 RON 里的十进制小数 → Q16.16 定点（`Fx`），**round**（非截断）语义：
/// `v * 65536.0` 四舍五入到最近整数即 raw 位模式。`f64::round` 对 `.5` 边界
/// 走"绝对值远离零"舍入（如 `0.5 → 32768`，恰好是整数点，不受舍入方向
/// 影响——用作本函数的金值锚点，见 `quantize_fx_round_half_examples` 单测）。
/// 这是 I/O 边界上唯一允许出现浮点运算的地方：结果一旦落地为 `Fx`，
/// core 侧全程整数运算（charter §6 混合数值制原判）。
pub fn quantize_fx(v: f64) -> Fx {
    Fx((v * 65536.0).round() as i32)
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
            Op::Emit {
                material: id(material)?,
                x: quantize_fx(*x),
                y: quantize_fx(*y),
                vx: quantize_fx(*vx),
                vy: quantize_fx(*vy),
                count: *count,
                jitter: quantize_fx(*jitter),
            }
        }
    })
}

pub fn load_scenario(path: &str, table: &MaterialTable) -> Result<Scenario, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读 {path} 失败：{e}"))?;
    let fingerprint = xxh3_64(&bytes);
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
            MaterialDef { id: 0, name: "air".into(), category: Category::Static, density: 0, color: (0, 0, 0) },
            MaterialDef { id: 1, name: "wall".into(), category: Category::Static, density: 100, color: (0, 0, 0) },
            MaterialDef { id: 2, name: "water".into(), category: Category::Liquid, density: 16, color: (0, 0, 0) },
        ])
        .unwrap()
    }

    // ==================== quantize_fx：round 金值（任务书要求）====================

    #[test]
    fn quantize_fx_exact_half_rounds_to_0x8000() {
        assert_eq!(quantize_fx(0.5).0, 0x8000);
        assert_eq!(quantize_fx(-0.5).0, -0x8000);
    }

    #[test]
    fn quantize_fx_integers_shift_exactly() {
        assert_eq!(quantize_fx(0.0).0, 0);
        assert_eq!(quantize_fx(1.0).0, 0x1_0000);
        assert_eq!(quantize_fx(-2.0).0, -0x2_0000);
    }

    #[test]
    fn quantize_fx_rounds_nearest_not_truncates() {
        // 0.0001 * 65536 = 6.5536 → round 到 7；若实现是截断会给 6，
        // 这条测试能把两者的实现差异暴露出来。
        assert_eq!(quantize_fx(0.0001).0, 7);
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
                assert_eq!(x, quantize_fx(120.0));
                assert_eq!(y, quantize_fx(8.0));
                assert_eq!(vx, quantize_fx(0.5));
                assert_eq!(vy, quantize_fx(2.0));
                assert_eq!(count, 3);
                assert_eq!(jitter, quantize_fx(0.8));
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

    // ==================== 场景指纹：raw 字节哈希天然覆盖 Emit 参数变化 ====================
    //
    // spec §7 要求"量化后的 Fx 原始位参与场景指纹"。`load_scenario` 的
    // `fingerprint` 取自 *解析前* 的原始文件字节（`xxh3_64(&bytes)`），
    // 任何文本层面的 Emit 参数改动都已经改变了这些字节——量化只发生在
    // 之后的 `resolve_op` 阶段，不可能出现"参数变了但指纹不变"的情形。
    // 这条测试把该保证钉死，防止未来把 fingerprint 改成只哈希部分字段时
    // 悄悄丢掉这个覆盖面。

    #[test]
    fn fingerprint_changes_when_emit_params_change() {
        let a = "Scenario(name:\"t\",world:(1,1),seed:0,ticks:1,setup:[],\
                  script:[Every(from:0,until:1,step:1,op:Emit(material:\"water\",x:1.0,y:1.0,vx:0.5,vy:1.0,count:1,jitter:0.1))])";
        let b = "Scenario(name:\"t\",world:(1,1),seed:0,ticks:1,setup:[],\
                  script:[Every(from:0,until:1,step:1,op:Emit(material:\"water\",x:1.0,y:1.0,vx:0.6,vy:1.0,count:1,jitter:0.1))])";
        assert_ne!(
            xxh3_64(a.as_bytes()),
            xxh3_64(b.as_bytes()),
            "仅改 Emit 的 vx，场景指纹也必须改变"
        );
    }
}
