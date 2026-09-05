//! 生物：模板表 + 运行时状态（spec §3.2、§4）。M4 Task 2 填运动学——扫掠碰撞
//! （刚体盖章格天然可站立）、跨台阶、起跳闸门；排开/游泳/接触伤害留 Task 3。
//!
//! **AoS**（`particle.rs` 的 SoA 相反）：生物数量个位数，AoS 比 SoA 清楚（spec
//! §3.2）。**id = 下标，永不回收**——`InputFrame` 按 controller 序号索引、
//! loadout/cooldown 按 id 关联，压缩会错位；死亡（Task 3）走 `alive = false`
//! 墓碑，不做保序压缩（与 `Particles::compact` 刻意不同体例）。

use crate::fixed::{Bam, Fx};
use crate::input::{InputFrame, BTN_JUMP, BTN_LEFT, BTN_RIGHT, MAX_SLOTS};
use crate::material::{self, MaterialTable};
use crate::particle::GRAVITY;
use crate::world::World;
use xxhash_rust::xxh3::Xxh3;

/// 生物数上限（spec §3.2）：超限 `Op::SpawnCreature` 确定性拒绝，不排队。
pub const MAX_CREATURES: usize = 16;

/// 单 tick 单轴最大整格步数（spec §4.2）：防高速穿透（如强击退后的一次性大
/// 位移）+ 界定 `aabb_blocked` 单 tick 最坏调用次数的上界。
pub const CREATURE_MAX_STEP: i32 = 8;

/// 生物模板（M4 spec §3.5 的运动学子集，R5 裁决）：本 Task 只建运动学需要的
/// 字段——`swim_*`/`damage_from`/`min_cell_count`/`max_displace_per_tick`/
/// `muzzle_offset`/`mana_*` 留 Task 3 追加，RON 加载器同留 Task 3。
#[derive(Clone, Copy, Debug)]
pub struct CreatureTpl {
    pub half_w: i32,
    pub half_h: i32,
    /// 地面/空中共用的水平速度上限（格/tick）。
    pub run_speed: Fx,
    /// 起跳瞬间竖直速度**大小**（格/tick）；施加时取负号（向上，spec §4.1）。
    pub jump_speed: Fx,
    /// 地面水平加速度（格/tick²）。
    pub accel_ground: Fx,
    /// 空中水平加速度（格/tick²）。
    pub accel_air: Fx,
    /// 自动跨台阶的最大高度（格，spec §4.2 Noita `climb_over_y`）。
    pub climb_over_y: i32,
    pub hp_max: i32,
}

/// 生物模板表（与 `MaterialTable` 同体例：加载期构造、只读）。
#[derive(Clone, Debug, Default)]
pub struct CreatureTable {
    tpls: Vec<CreatureTpl>,
}

impl CreatureTable {
    pub fn empty() -> CreatureTable {
        CreatureTable::default()
    }

    pub fn from_tpls(tpls: Vec<CreatureTpl>) -> CreatureTable {
        CreatureTable { tpls }
    }

    /// 模板号越界即调用方漏配置（与 `MaterialTable::category` 等同一体例：
    /// 模板号来自数据表，加载期校验负责挡脏值，这里直接索引）。
    pub fn get(&self, template: u8) -> &CreatureTpl {
        &self.tpls[template as usize]
    }

    /// 测试/harness 起步用默认玩家模板（R5：数值取 spec §3.5 的起步猜测值，
    /// 与 `data/creatures.ron`——Task 3 起才建——的 `player` 条目同源，
    /// 但本 Task 尚无 RON 加载器，直接在 Rust 侧构造）。
    pub fn default_player() -> CreatureTable {
        CreatureTable::from_tpls(vec![CreatureTpl {
            half_w: 2,
            half_h: 5,
            run_speed: Fx::from_ratio(67, 100),
            jump_speed: Fx::from_ratio(29, 10),
            accel_ground: Fx::from_ratio(5, 100),
            accel_air: Fx::from_ratio(5, 1000),
            climb_over_y: 3,
            hp_max: 100,
        }])
    }
}

/// 生物运行时状态（spec §3.2）。`mana`/`cooldowns` 本 Task 尚不消费（Task 5
/// 起施法结算才会读写），提前建好字段是为了哈希结构一次到位——避免后续 Task
/// 再变更实体层哈希布局（M4 spec §1.3 已把 golden 重录钉在 Task 2 这一次）。
#[derive(Clone, Copy, Debug)]
pub struct Creature {
    /// AABB 中心（连续坐标，Q16.16）。
    pub x: Fx,
    pub y: Fx,
    pub vx: Fx,
    pub vy: Fx,
    pub half_w: i32,
    pub half_h: i32,
    pub hp: i32,
    pub mana: i32,
    pub cooldowns: [u16; MAX_SLOTS],
    pub loadout: [u8; MAX_SLOTS],
    pub aim: Bam,
    pub team: u8,
    /// controller 序号（inputs 切片下标）；`255` = 不吃输入（NPC/测试假人）。
    pub controller: u8,
    /// 指回 `CreatureTable`。
    pub template: u8,
    pub on_ground: bool,
    pub facing_right: bool,
    pub alive: bool,
}

/// 生物表：AoS，下标即 id。
#[derive(Clone, Debug, Default)]
pub struct Creatures {
    list: Vec<Creature>,
}

impl Creatures {
    pub fn new() -> Creatures {
        Creatures::default()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn get(&self, id: u8) -> Option<&Creature> {
        self.list.get(id as usize)
    }

    /// 死亡/伤害设置（供测试与诊断，Task 3 起的接触伤害/死亡逻辑经此落地；
    /// id 越界静默忽略——与 `Sim::queue_spawn` 一样不做脏值防御，调用方保证）。
    pub fn set_hp(&mut self, id: u8, hp: i32) {
        if let Some(c) = self.list.get_mut(id as usize) {
            c.hp = hp;
        }
    }

    /// 供测试与诊断，同 [`Creatures::set_hp`]。
    pub fn set_mana(&mut self, id: u8, mana: i32) {
        if let Some(c) = self.list.get_mut(id as usize) {
            c.mana = mana;
        }
    }

    /// 生成一个生物（`Op::SpawnCreature` 的落点，与 `Bodies::spawn_rect` 同
    /// 体例）。`x`/`y` 是 AABB 中心的整格坐标（与 `Op::Fill` 等一致，不做
    /// 半格居中——生物半宽高本就是整数格，中心落在格线上是自然的）。
    /// 容量满即确定性拒绝（`MAX_CREATURES`，粒子池同一口径），不排队。
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        &mut self,
        tpl: &CreatureTable,
        template: u8,
        x: i32,
        y: i32,
        team: u8,
        controller: u8,
        loadout: [u8; MAX_SLOTS],
    ) -> Option<u8> {
        if self.list.len() >= MAX_CREATURES {
            return None;
        }
        let t = tpl.get(template);
        let id = self.list.len() as u8;
        self.list.push(Creature {
            x: Fx::from_int(x),
            y: Fx::from_int(y),
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            half_w: t.half_w,
            half_h: t.half_h,
            hp: t.hp_max,
            mana: 0,
            cooldowns: [0; MAX_SLOTS],
            loadout,
            aim: 0,
            team,
            controller,
            template,
            on_ground: false,
            facing_right: true,
            alive: true,
        });
        Some(id)
    }

    /// 生物 `i`（下标，非 controller 值）本 tick 生效的输入：`controller ==
    /// 255` 或越界 → `InputFrame::default()`（全键松开）——NPC/测试假人与
    /// bridge 尚未接线时的安全缺省（spec Interfaces）。
    pub fn input_of(&self, i: usize, inputs: &[InputFrame]) -> InputFrame {
        let controller = self.list[i].controller;
        if controller == 255 {
            return InputFrame::default();
        }
        inputs.get(controller as usize).copied().unwrap_or_default()
    }

    /// 生物运动学（架构 §4 第 2 步的 2a+2b，spec §4.1/§4.2）：按 id 序（不得
    /// 用迭代器打乱，架构 §7.1 定序铁律）—— ① 水平意图转加速度／减速度
    /// （地面 `accel_ground`、空中 `accel_air`）；② 重力（与网格同源，
    /// `particle::GRAVITY`）+ 起跳闸门（仅 `on_ground` 时生效，不做二段跳）；
    /// ③ 逐轴分离扫掠，**先 x 后 y**（顺序即协议，spec §4.2）。
    pub fn step_kinematics(&mut self, world: &World, table: &MaterialTable, tpl: &CreatureTable, inputs: &[InputFrame]) {
        for i in 0..self.list.len() {
            if !self.list[i].alive {
                continue;
            }
            let inp = self.input_of(i, inputs);
            let t = *tpl.get(self.list[i].template);
            let c = &mut self.list[i];

            // ① 水平意图：按住方向键朝 run_speed 加速；无意图（或两键同按，
            // 视为无意图——不引入"最后一键优先"这类需要额外状态定序的规则）
            // 按同一 accel 幅度向 0 收敛，即"减速"（brief 用语的字面含义）。
            let accel = if c.on_ground { t.accel_ground } else { t.accel_air };
            let left = inp.held(BTN_LEFT);
            let right = inp.held(BTN_RIGHT);
            if right && !left {
                c.vx = clamp_toward(c.vx + accel, t.run_speed);
            } else if left && !right {
                c.vx = clamp_toward(c.vx - accel, -t.run_speed);
            } else {
                c.vx = decay_toward_zero(c.vx, accel);
            }

            // ② 重力恒定施加（与网格同源，不因 on_ground 而跳过——落地瞬间的
            // "多余"竖直速度由 sweep_y 的碰撞响应清零，不在这里特殊处理，
            // 避免起跳/落地两条路径各写一遍重力开关）；起跳只在 on_ground
            // 时生效，直接把 vy 置为起跳速度（覆盖而非叠加——脉冲语义）。
            c.vy = c.vy + GRAVITY;
            if inp.held(BTN_JUMP) && c.on_ground {
                c.vy = -t.jump_speed;
            }

            // ③ 逐轴分离扫掠：先 x 后 y。
            sweep_x(c, world, table, &t);
            sweep_y(c, world, table);
            c.aim = inp.aim;
        }
    }

    /// 实体层哈希的生物部分（架构 spec §1.3/§7.1）：空表恒 0（早退，Task 1
    /// 零行为变化的依据），非空按 id 序折叠全字段（含 `cooldowns`/`mana`/
    /// `hp`/`aim`，即便本 Task 尚不写它们，也一并入哈希——避免哈希结构在
    /// 后续 Task 又变一次）。
    pub fn hash_into(&self) -> u64 {
        if self.list.is_empty() {
            return 0;
        }
        let mut h = Xxh3::new();
        for c in &self.list {
            h.update(&c.x.0.to_le_bytes());
            h.update(&c.y.0.to_le_bytes());
            h.update(&c.vx.0.to_le_bytes());
            h.update(&c.vy.0.to_le_bytes());
            h.update(&c.half_w.to_le_bytes());
            h.update(&c.half_h.to_le_bytes());
            h.update(&c.hp.to_le_bytes());
            h.update(&c.mana.to_le_bytes());
            for cd in c.cooldowns {
                h.update(&cd.to_le_bytes());
            }
            h.update(&c.loadout);
            h.update(&c.aim.to_le_bytes());
            h.update(&[c.team, c.controller, c.template, c.on_ground as u8, c.facing_right as u8, c.alive as u8]);
        }
        h.update(&(self.list.len() as u64).to_le_bytes());
        h.digest()
    }
}

/// `v` 向 `limit`（可正可负）方向加速，但不得越过 `limit`——`limit` 与 `v` 移
/// 动方向的加速同号时才有意义，调用点保证。用 `min`/`max` 而非先判方向再夹，
/// 避免为符号分支多写一套逻辑。
fn clamp_toward(v: Fx, limit: Fx) -> Fx {
    if limit.0 >= 0 {
        v.min(limit)
    } else {
        v.max(limit)
    }
}

/// 无水平意图时，`v` 按 `step` 幅度向 0 收敛，不过冲（到 0 就停，不反向）。
fn decay_toward_zero(v: Fx, step: Fx) -> Fx {
    if v.0 > 0 {
        (v - step).max(Fx::ZERO)
    } else if v.0 < 0 {
        (v + step).min(Fx::ZERO)
    } else {
        Fx::ZERO
    }
}

/// AABB `[cx-hw, cx+hw] × [cy-hh, cy+hh]`（格）内是否存在硬格。`include_bodies
/// = true`——刚体盖章格对生物就是地形（M3 木箱免费变平台，M4 spec §2/§4.2）。
fn aabb_blocked(world: &World, table: &MaterialTable, x: Fx, y: Fx, hw: i32, hh: i32) -> bool {
    let (cx, cy) = (x.to_cell(), y.to_cell());
    for gy in (cy - hh)..=(cy + hh) {
        for gx in (cx - hw)..=(cx + hw) {
            if material::is_solid(world.cell(gx, gy), table, true) {
                return true;
            }
        }
    }
    false
}

/// 沿 x 轴扫掠，撞硬格即停并清零该轴速度；被挡时尝试抬高 1..=`climb_over_y`
/// 格重试（Noita `climb_over_y`，spec §4.2）。抬高判定按固定升序、无掷骰。
///
/// **本 tick 实际要跨越的整格边界数**由「起点格 → 终点格」的格差决定
/// （`crossing`），而不是 `|vx|` 本身的整数部分——任务书原始伪代码用后者当
/// 步数，在 `|vx| < 1` 格/tick（典型走速 0.67、重力 0.25 都 < 1）时恒为 0，
/// 会让碰撞检测彻底失效：亚格位移逐 tick 累加、从不触发边界检测，生物会
/// 直接滑穿墙体/地板——任务书给的阻挡/跨台阶两条测试若照字面实现，实测
/// 会失败（非纸面推测，调试过程见 commit）。改用「起点格 → 终点格」的
/// 格差，天然把跨多 tick 累积的亚格位移一并算进去，撞墙/触地判定不再
/// 依赖单 tick 速度是否 ≥ 1 格。`crossing` 按 `CREATURE_MAX_STEP` 截断——
/// 超出的余量本 tick 不走，下一 tick 用新位置重新算（不做位移欠账，跨
/// tick 排队会把限流变成状态机，同 M1 溅射限流第 ② 条先例）。
fn sweep_x(c: &mut Creature, world: &World, table: &MaterialTable, t: &CreatureTpl) {
    let (hw, hh) = (c.half_w, c.half_h);
    let dir: i32 = if c.vx.0 > 0 {
        1
    } else if c.vx.0 < 0 {
        -1
    } else {
        return;
    };
    c.facing_right = dir > 0;
    let target = c.x + c.vx;
    let crossing =
        if dir > 0 { target.to_cell() - c.x.to_cell() } else { c.x.to_cell() - target.to_cell() }.max(0);
    let steps = crossing.min(CREATURE_MAX_STEP);
    for _ in 0..steps {
        let nx = c.x + Fx::from_int(dir);
        if !aabb_blocked(world, table, nx, c.y, hw, hh) {
            c.x = nx;
            continue;
        }
        let mut climbed = false;
        for up in 1..=t.climb_over_y {
            let ny = c.y - Fx::from_int(up);
            if !aabb_blocked(world, table, nx, ny, hw, hh) {
                c.x = nx;
                c.y = ny;
                climbed = true;
                break;
            }
        }
        if !climbed {
            c.vx = Fx::ZERO;
            return; // 撞停：本 tick 剩余位移（含未达 target 的亚格余量）整体丢弃
        }
    }
    if steps == crossing {
        // 已验证的整格边界全部通过、且未被 CREATURE_MAX_STEP 截断——最后一段
        // 亚格位移落在最后已验证的格内（`to_cell()` 与该格边界代表点相同，
        // 见 aabb_blocked 的粗粒度假设），直接落到精确 target。
        c.x = target;
    }
    // else：本 tick 被步数上限截断，多出的位移作废（见函数头注）。
}

/// 沿 y 轴扫掠，同 `sweep_x` 但无跨台阶分支。撞到即 `vy = 0`；**向下**撞停
/// （`dir > 0`）时 `on_ground = true`，其余情况（含向上撞顶、未撞）置
/// `false`（spec §4.2）。
fn sweep_y(c: &mut Creature, world: &World, table: &MaterialTable) {
    let (hw, hh) = (c.half_w, c.half_h);
    let dir: i32 = if c.vy.0 > 0 {
        1
    } else if c.vy.0 < 0 {
        -1
    } else {
        return;
    };
    let target = c.y + c.vy;
    let crossing =
        if dir > 0 { target.to_cell() - c.y.to_cell() } else { c.y.to_cell() - target.to_cell() }.max(0);
    let steps = crossing.min(CREATURE_MAX_STEP);
    for _ in 0..steps {
        let ny = c.y + Fx::from_int(dir);
        if aabb_blocked(world, table, c.x, ny, hw, hh) {
            c.vy = Fx::ZERO;
            c.on_ground = dir > 0;
            return;
        }
        c.y = ny;
    }
    if steps == crossing {
        c.y = target;
    }
    c.on_ground = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_creatures_hash_into_is_zero() {
        assert_eq!(Creatures::new().hash_into(), 0);
    }

    #[test]
    fn spawn_returns_stable_ids_and_rejects_beyond_capacity() {
        let tpl = CreatureTable::default_player();
        let mut cs = Creatures::new();
        for i in 0..MAX_CREATURES {
            let id = cs.spawn(&tpl, 0, 10, 10, 0, 255, [255; MAX_SLOTS]);
            assert_eq!(id, Some(i as u8));
        }
        assert_eq!(cs.spawn(&tpl, 0, 10, 10, 0, 255, [255; MAX_SLOTS]), None, "超限必须确定性拒绝");
        assert_eq!(cs.len(), MAX_CREATURES);
    }

    #[test]
    fn input_of_defaults_when_controller_is_255_or_out_of_range() {
        let tpl = CreatureTable::default_player();
        let mut cs = Creatures::new();
        cs.spawn(&tpl, 0, 0, 0, 0, 255, [255; MAX_SLOTS]); // i=0，不吃输入
        cs.spawn(&tpl, 0, 0, 0, 0, 3, [255; MAX_SLOTS]); // i=1，controller=3，越界
        assert_eq!(cs.input_of(0, &[InputFrame::new(BTN_RIGHT, 0, 0)]), InputFrame::default());
        assert_eq!(cs.input_of(1, &[InputFrame::new(BTN_RIGHT, 0, 0)]), InputFrame::default());
    }

    #[test]
    fn hash_into_changes_when_a_field_changes() {
        let tpl = CreatureTable::default_player();
        let mut cs = Creatures::new();
        cs.spawn(&tpl, 0, 10, 10, 0, 255, [255; MAX_SLOTS]);
        let h0 = cs.hash_into();
        cs.set_hp(0, 50);
        assert_ne!(cs.hash_into(), h0, "hp 变化必须反映进哈希");
    }
}
