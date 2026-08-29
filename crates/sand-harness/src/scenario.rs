//! 场景与材料表加载（spec §2、§5.2）。
//! 数据即确定性输入（P5）：materials.ron 与场景文件都算 xxh3 指纹，
//! 进入 replay/golden 头部——指纹不符即拒绝比对。

use serde::Deserialize;
use xxhash_rust::xxh3::xxh3_64;

use sand_core::{Category, MaterialDef, MaterialTable, Op};

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
