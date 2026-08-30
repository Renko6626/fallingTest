//! 场景与材料表加载（spec §2、§5.2）。
//! 数据即确定性输入（P5）：materials.ron 与场景文件都算 xxh3 指纹，
//! 进入 replay/golden 头部——指纹不符即拒绝比对。

use serde::Deserialize;
use xxhash_rust::xxh3::{xxh3_64, Xxh3};

use sand_core::{Category, Fx, MaterialDef, MaterialTable, Op, MAX_EMIT_JITTER_RAW};

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
    })
}

/// 折叠"全部已解析 `Op` 中 `Fx` 字段"的原始位（spec §7：量化后的数值必须
/// 入场景指纹）。目前只有 `Op::Emit` 携带 `Fx` 字段，其余变体（`Brush`/
/// `Fill`）是纯整数、不折叠——它们的文本已经被 [`load_scenario`] 里的
/// 源字节哈希覆盖，这里不重复。
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
    let source_fp = xxh3_64(&bytes);
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
            MaterialDef { id: 0, name: "air".into(), category: Category::Static, density: 0, color: (0, 0, 0) },
            MaterialDef { id: 1, name: "wall".into(), category: Category::Static, density: 100, color: (0, 0, 0) },
            MaterialDef { id: 2, name: "water".into(), category: Category::Liquid, density: 16, color: (0, 0, 0) },
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
