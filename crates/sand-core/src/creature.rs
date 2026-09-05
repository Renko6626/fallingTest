//! 生物：模板表 + 运行时状态（spec §3.2、§4）。M4 Task 2 填运动学——扫掠碰撞
//! （刚体盖章格天然可站立）、跨台阶、起跳闸门；M4 Task 3 补世界互动——排开
//! 液体/粉末、游泳浮力、材质接触伤害与死亡墓碑（spec §4.3–§4.5）。
//!
//! **AoS**（`particle.rs` 的 SoA 相反）：生物数量个位数，AoS 比 SoA 清楚（spec
//! §3.2）。**id = 下标，永不回收**——`InputFrame` 按 controller 序号索引、
//! loadout/cooldown 按 id 关联，压缩会错位；死亡走 `alive = false` 墓碑，
//! 不做保序压缩（与 `Particles::compact` 刻意不同体例）。

use crate::fixed::{self, Bam, Fx};
use crate::input::{InputFrame, BTN_DOWN, BTN_JUMP, BTN_LEFT, BTN_RIGHT, MAX_SLOTS};
use crate::material::{self, Category, MaterialTable};
use crate::particle::GRAVITY;
use crate::world::{SpawnRequest, World};
use xxhash_rust::xxh3::Xxh3;

/// 生物数上限（spec §3.2）：超限 `Op::SpawnCreature` 确定性拒绝，不排队。
pub const MAX_CREATURES: usize = 16;

/// 单 tick 单轴最大整格步数（spec §4.2）：防高速穿透（如强击退后的一次性大
/// 位移）+ 界定 `aabb_blocked` 单 tick 最坏调用次数的上界。
pub const CREATURE_MAX_STEP: i32 = 8;

/// 生物模板（M4 spec §3.5）：Task 2 建了运动学子集，本 Task（3）补齐世界互动
/// 需要的字段全集——`swim_*`/`damage_from`/`min_cell_count`/
/// `max_displace_per_tick`/`muzzle_offset`/`mana_*`（R5 裁决）。
///
/// **不再 `Copy`**（Task 2 时是）：`damage_from` 是变长 `Vec`，加了它之后
/// `CreatureTpl` 天然只能 `Clone`。唯一受影响的调用点是
/// `step_kinematics` 的 `let t = *tpl.get(..)`——改成持有引用
/// （`let t = tpl.get(..)`），不再解引用拷贝整个模板；`spawn`/`get` 本就只
/// 借用，未受影响。
#[derive(Clone, Debug)]
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
    /// 满血值，千分位整数（spec §3.2："hp 用整数千分位"——`Creature::hp` 出生
    /// 即拷贝本字段，单位必须同源）。RON 侧写十进制点数，加载期经
    /// `sand-harness::scenario::quantize_milli` 一次性 `×1000 round`。
    pub hp_max: i32,
    /// 满蓝值，同 `hp_max` 千分位口径。Task 3 尚不消费（施法结算留 Task 5），
    /// 提前建好字段只为哈希结构/模板结构一次到位。
    pub mana_max: i32,
    /// 每 tick 回蓝量，千分位整数（spec §4"每秒量加载期一次性折成每 tick
    /// 量"）：RON 写点/秒，加载期经 `round(v * 1000.0 / 60.0)` 一次性折算，
    /// 运行时不再做除法。Task 5 前不消费。
    pub mana_regen_per_tick: i32,
    /// 静止（无竖直意图）时的浮力系数（spec §4.4，Noita `swim_idle_buoyancy_coeff`）。
    pub swim_buoyancy_idle: Fx,
    /// 竖直意图向上时的浮力系数（评审 Important #2，**故意偏离 Noita 原值 0.9**）：
    /// Noita 玩家在水里还有一份独立的喷射推力（jetpack/fly）在做上升，`swim_*_coeff`
    /// 三个系数在 Noita 那边只调**被动**浮力，主动上升力另有出处；我们没有那份
    /// 独立推力，若照抄 0.9（< idle 的 1.2）就会净下沉——"按住上"反而比什么都
    /// 不按沉得更快（`holding_swim_up_floats_faster_than_idle` 钉死，2026-09-05
    /// 评审 Important #2 实测复现）。故本字段必须 **> `swim_buoyancy_idle`**
    /// 才能让 up > idle > down 的方向语义成立——1.4 是起步值，日后目检再调
    /// （spec 明列的 A 类手感旋钮）。**不要对着 Noita 数值表把它"修正"回 0.9**：
    /// 那是给"有独立喷射推力"的角色用的系数，我们没有那份推力。
    pub swim_buoyancy_up: Fx,
    /// 竖直意图向下时的浮力系数（Noita `swim_down_buoyancy_coeff`，未偏离原值）。
    pub swim_buoyancy_down: Fx,
    /// 游泳中对 `(vx, vy)` 的统一阻力乘子（每 tick，spec §4.4）。
    pub swim_drag: Fx,
    /// 受害者侧材质接触伤害表（spec §4.5，Noita `DamageModelComponent
    /// .materials_that_damage` 口径：怕什么写在受害者身上，不动材质表）。
    /// `(材质 id, 每 tick 千分位伤害)`，**按材质 id 升序**存放——定序遍历
    /// 红线（CLAUDE.md §5 第 4 条），且与 Noita 源码 `mDamageMaterials`
    /// 注释 "NOTE! Sorted!" 一致。伤害值加载期已把 RON 的 dps 折成
    /// 每 tick 千分位（`round(v * 1000.0 / 60.0)`），运行时纯整数乘加。
    pub damage_from: Vec<(u8, i32)>,
    /// 接触伤害生效的最小格数门槛（spec §4.5，Noita
    /// `material_damage_min_cell_count`）：当帧某材质接触格数低于此值，
    /// 该材质整项伤害忽略（不累加、不四舍五入到 0，是"这一 tick 完全没有
    /// 这项伤害来源"）。
    pub min_cell_count: u16,
    /// 单 tick 单生物最大排开格数（spec §4.3）：超限的软格**不排开、不排队**
    /// ——排队需要跨 tick 状态，会把限流变成状态机（同 M1 溅射限流先例）。
    pub max_displace_per_tick: usize,
    /// 施法口出射点相对生物中心的偏移（格，Task 5 消费）。Task 3 只建字段。
    pub muzzle_offset: i32,
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

    /// 测试/harness 起步用默认玩家模板（R5：数值取 spec §3.5 `data/creatures.ron`
    /// 的 `player` 条目同源，直接在 Rust 侧构造——生产路径经 `load_creatures`
    /// 从 RON 加载，本函数只服务测试/harness 起步）。
    ///
    /// `damage_from` 只镜像 `("fire", 3.0)` 一项，**材质 id 硬编码为 5**：
    /// 本函数不接收 `MaterialTable`（无法经 `id_by_name` 查名），而
    /// `crates/sand-core/tests/common/mod.rs::materials()` 特意把 `fire`
    /// 摆在 id 5 与此对齐——两处都要改就一起改。`data/creatures.ron` 里的
    /// `lava`/`acid` 两项不在这里镜像：它们此刻在任何测试材质表里都不存在，
    /// 硬编码一个凑数 id 只会制造"看着有、其实测不到"的假覆盖；生产路径的
    /// 真实值经 `load_creatures` 按名解析，不吃这个硬编码。
    pub fn default_player() -> CreatureTable {
        CreatureTable::from_tpls(vec![CreatureTpl {
            half_w: 2,
            half_h: 5,
            run_speed: Fx::from_ratio(67, 100),
            jump_speed: Fx::from_ratio(29, 10),
            accel_ground: Fx::from_ratio(5, 100),
            accel_air: Fx::from_ratio(5, 1000),
            climb_over_y: 3,
            hp_max: 100_000,
            mana_max: 100_000,
            mana_regen_per_tick: 333, // round(20.0 * 1000.0 / 60.0)
            swim_buoyancy_idle: Fx::from_ratio(12, 10),
            // 评审 Important #2：14/10 而非 Noita 原值 9/10——见 CreatureTpl::swim_buoyancy_up 文档。
            swim_buoyancy_up: Fx::from_ratio(14, 10),
            swim_buoyancy_down: Fx::from_ratio(7, 10),
            swim_drag: Fx::from_ratio(95, 100),
            damage_from: vec![(5, 50)], // fire(id 5) dps 3.0 → round(3000/60)=50
            min_cell_count: 4,
            max_displace_per_tick: 24,
            muzzle_offset: 3,
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
            // 引用而非拷贝（`CreatureTpl` 自 Task 3 起因 `damage_from: Vec<_>`
            // 不再 `Copy`）：`t` 借自 `tpl`（与 `self` 无关），下面 `&mut
            // self.list[i]` 不与它冲突。
            let t = tpl.get(self.list[i].template);
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
            let jumped = inp.held(BTN_JUMP) && c.on_ground;
            if jumped {
                c.vy = -t.jump_speed;
            }

            // ③ 逐轴分离扫掠：先 x 后 y。`sweep_y` 把 `on_ground` 算成对**当前**
            // footprint 的查询（评审复审第二轮修复，见其头注），天然覆盖
            // "水平走出台面"这类 `sweep_x` 改了 `c.x` 但 `sweep_y` 本 tick
            // 未跨格的情形。唯一的例外是本 tick 起跳：查询是粗粒度的整格
            // 判定，`jump_speed` 若调得很小（< 1 格/tick，本模板的 2.9 不会
            // 触发但不能假设永远如此）理论上查询仍可能读到"脚下还是硬格"，
            // 那样 `held(BTN_JUMP)`（电平触发，不是边沿触发）会在下一 tick
            // 重新满足闸门条件、每 tick 重新施加起跳速度 → 悬浮/连跳
            // （`holding_jump_launches_only_once_not_every_tick` 钉死这一条）。
            // 起跳在物理意图上就是"这一 tick 主动离地"的脉冲，因此只要闸门
            // 触发就无条件把 `on_ground` 收口为 `false`，不依赖查询结果、
            // 不依赖 `jump_speed` 数值——比"存上一 tick 按键做边沿触发"更
            // 简单（不需要新增字段进哈希），也不依赖调参凑巧躲开这个坑。
            sweep_x(c, world, table, t);
            sweep_y(c, world, table);
            if jumped {
                c.on_ground = false;
            }
            c.aim = inp.aim;
        }
    }

    /// 第 2 步后半（架构 §4，spec §4.3–§4.5）：排开液体/粉末 → 游泳 → 材质
    /// 接触伤害 → HP 归零墓碑。按 id 序（架构 §7.1 定序铁律，与
    /// `step_kinematics` 同一遍历口径）；跳过已死亡的墓碑（`alive == false`
    /// 的生物既不再移动也不再受二次伤害，spec §4.5"不做 ragdoll"的直接推论）。
    ///
    /// 读本 tick `step_kinematics` 之后的网格/生物状态——与 `lib.rs::step`
    /// 里紧跟在 `step_kinematics` 之后调用同一口径（spec §1.1）。
    ///
    /// `pub(crate)`（不是 `pub`）：出参 `spawns: &mut Vec<SpawnRequest>` 的
    /// `SpawnRequest` 本身就是 `pub(crate)`（world.rs 文档："外部 crate 拿不到
    /// 能传的实参"），与 `Bodies::stamp_all` 同一体例——`pub` 只会产生
    /// "公开但不可调用"的私有类型泄漏警告（`private_interfaces`）。
    ///
    /// **`inputs` 是对 brief 原始签名（Interfaces 一节）的一处偏离**：brief
    /// 列的签名不带 `inputs`，游泳档位改按"本 tick `c.vy` 的符号"猜竖直意图
    /// ——实测证伪（见 `creature_floats_in_deep_water_instead_of_sinking_to_bottom`
    /// 调试记录）：`step_kinematics` 每 tick 无条件加重力，落地静止后 `vy`
    /// 几乎恒为"刚好非负"，`vy` 符号法会把"完全没按键、纯粹被重力压着"永久
    /// 误判成"竖直意图向下"，`swim_buoyancy_idle`（唯一 >1 的档位，本该让人
    /// 停止不动时也能浮起来）因此实际上永远选不到，泡水 600 tick 只会静静
    /// 沉在池底——与 spec §4.4"档位由本 tick 的竖直意图（上/下/无）选取"的
    /// 字面意思矛盾（意图不该等于"重力刚好把速度压成正数"）。改为直接读
    /// `BTN_JUMP`/`BTN_DOWN`（与 `step_kinematics` 读 `BTN_JUMP` 触发起跳
    /// 同一个按键语义，游泳时复用为"划水向上"；`BTN_DOWN` 现在唯一的消费点）。
    pub(crate) fn step_world_interaction(
        &mut self,
        world: &mut World,
        table: &MaterialTable,
        tpl: &CreatureTable,
        inputs: &[InputFrame],
        stamp: u8,
        spawns: &mut Vec<SpawnRequest>,
    ) {
        for i in 0..self.list.len() {
            if !self.list[i].alive {
                continue;
            }
            let inp = self.input_of(i, inputs);
            // 取一份运行时状态的拷贝（`Creature: Copy`）：本函数全程只需要
            // 这一份"读取时刻快照"，AABB 扫描、`world` 读写都不再牵动
            // `self.list` 的借用，函数末尾一次性写回——比"先拿不可变引用、
            // 中途再拿可变引用"（brief 原始伪代码 `let c = &self.list[i]`
            // 那条路）更简单，也不会让借用检查器在同一循环体里两难。
            let mut c = self.list[i];
            let t = tpl.get(c.template);
            let (cx, cy) = (c.x.to_cell(), c.y.to_cell());

            // ① 扫 AABB 一遍，同时收集"可排开格坐标"与"各材质格数"。格序
            // 自上而下、自左而右——即下面②的排开序，确定；`counts` 是定长
            // 数组（材质数上限 256 = u8 全域）而非 HashMap（红线 4）。
            let mut soft_cells: Vec<(i32, i32, u8)> = Vec::new();
            let mut counts = [0u16; 256];
            let mut submerged: u16 = 0;
            for gy in (cy - c.half_h)..=(cy + c.half_h) {
                for gx in (cx - c.half_w)..=(cx + c.half_w) {
                    if !world.in_bounds(gx, gy) {
                        continue;
                    }
                    let m = world.cell(gx, gy).material();
                    counts[m as usize] = counts[m as usize].saturating_add(1);
                    match table.category(m) {
                        Category::Liquid => {
                            submerged += 1;
                            soft_cells.push((gx, gy, m));
                        }
                        Category::Powder => soft_cells.push((gx, gy, m)),
                        _ => {}
                    }
                }
            }

            // ② 排开：取前 max_displace_per_tick 个，置 air + 脱格成粒子。
            // 复用 M3 被盖液体脱格的同一条路径（`body.rs::stamp_body`）：
            // `set_cell_stamped(AIR)` + 追加 `SpawnRequest` 进同一个
            // `spawn_queue`，本 tick 粒子相按追加序 drain——与 `Op::Emit`/
            // 刚体盖章完全同一通路，脏矩形合并与 chunk 唤醒一视同仁。
            // 超过上限的软格**不排开、不排队**（确定性拒绝，同 M1 溅射限流
            // 先例：排队需要跨 tick 状态，会把限流变成状态机）。
            for &(gx, gy, m) in soft_cells.iter().take(t.max_displace_per_tick) {
                world.set_cell_stamped(table, gx, gy, material::MAT_AIR, stamp);
                spawns.push(SpawnRequest {
                    material: m,
                    x: Fx::from_int(gx) + fixed::HALF_CELL,
                    y: Fx::from_int(gy) + fixed::HALF_CELL,
                    vx: c.vx,
                    vy: c.vy,
                });
            }

            // ③ 游泳（spec §4.4）：AABB 内液体格数 > 0 即视为"在水里"。三档
            // 浮力系数由本 tick 的竖直意图选取——直接读按键（`BTN_JUMP` =
            // 向上划水，复用起跳键；`BTN_DOWN` = 向下潜；都不按 = 空档
            // `_idle`，函数头注解释了为什么不能用 `c.vy` 的符号当代理）。
            // 净竖直加速度 = 本 tick 已加的重力 − `GRAVITY * coeff`：idle
            // 系数（1.2）> 1 时净上浮，其余两档系数 < 1 仍净下沉但比空中慢
            // （潜水比自由落体慢，仍能主动下潜）——`swim_drag` 把本来无界
            // 累积的速度收敛到有限终速度。
            if submerged > 0 {
                let coeff = if inp.held(BTN_JUMP) {
                    t.swim_buoyancy_up
                } else if inp.held(BTN_DOWN) {
                    t.swim_buoyancy_down
                } else {
                    t.swim_buoyancy_idle
                };
                c.vy = c.vy - GRAVITY.mul(coeff);
                c.vx = c.vx.mul(t.swim_drag);
                c.vy = c.vy.mul(t.swim_drag);
            }

            // ④ 材质接触伤害（spec §4.5，Noita 口径：怕什么写在受害者模板上，
            // 不动材质表）。`t.damage_from` 加载期已按材质 id 升序排好——遍历
            // 本身就是定序遍历（红线 4），不需要运行期再排。当帧接触格数
            // `< min_cell_count` 该材质整项忽略（不是"伤害算出来是 0"，是
            // "这项伤害源本身当帧不生效"，Noita `material_damage_min_cell_count`
            // 原意）。伤害值加载期已折成每 tick 千分位，这里是纯整数乘加——
            // 显式走 `wrapping_*`（评审 Important #1）：`n`（AABB 格数）与
            // `dmg_per_tick_milli` 都来自数据驱动配置，加载器对 `half_w`/
            // `half_h` 未做域校验（brief 未要求），较大 AABB 或较高 dps 会让
            // 裸 `*`/`-=` 在 debug 下 panic、release 下静默环绕——正是 Global
            // Constraints「一切算术走 wrapping_*」要防的 dev/release 位级
            // 不一致（`fixed.rs` 的 `Fx::Add`/`Sub`/`mul` 全部内部 wrapping，
            // 是现成先例）。`counts[m as usize].saturating_add(1)`（①）不用
            // 改：`saturating_add` 本身在 debug/release 下行为一致，不违背
            // 这条红线的立意。
            for &(m, dmg_per_tick_milli) in &t.damage_from {
                let n = counts[m as usize];
                if n >= t.min_cell_count {
                    c.hp = c.hp.wrapping_sub((n as i32).wrapping_mul(dmg_per_tick_milli));
                }
            }

            // ⑤ HP 归零 → 墓碑（spec §4.5"不做 ragdoll、不做尸体、不掉落"）：
            // `alive = false`，速度清零使其此刻起不再有位移倾向；`id` 不
            // 回收——`self.list` 从不做保序压缩，下一 tick `step_kinematics`
            // 顶部的 `if !alive { continue; }` 让墓碑连重力都不再吃，位置
            // 因此永久冻结在死亡瞬间（`dead_creature_keeps_its_id_and_stops_moving`
            // 钉死）。
            if c.hp <= 0 {
                c.alive = false;
                c.vx = Fx::ZERO;
                c.vy = Fx::ZERO;
            }

            self.list[i] = c;
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

/// 沿 y 轴扫掠，同 `sweep_x` 但无跨台阶分支；撞到即 `vy = 0`。
///
/// **`on_ground` 是查询，不是"扫掠的副作用"**（评审复审第二轮修复）：第一版
/// 实现（Important #1 修复）在"本 tick 未跨格、跳过检测"时原样保留上一 tick
/// 的 `on_ground`，隐含假设"未跨格 ⇒ footprint 未变化"——这个假设只对**纯
/// 竖直运动**成立。`step_kinematics` 先 `sweep_x` 后 `sweep_y`，`sweep_x`
/// 只改 `c.x`、从不碰 `on_ground`：生物若在"未跨格"的窗口里被 `sweep_x`
/// 水平移出了原来的支撑格（比如走出台阶边缘），旧值就是在描述一个已经不
/// 存在的 footprint，会让起跳闸门在离台后的短暂窗口里误放行一次空中跳
/// （`on_ground_is_false_whenever_the_aabb_has_fully_left_the_ledge` 钉死）。
///
/// 改为：位置积分（跨格检测 + 逐格碰撞响应）与 `on_ground` 判定彻底分离，
/// 后者在函数末尾用**当前**（本函数可能已经更新过的）`c.x`/`c.y` 显式探测
/// "脚下紧邻一格是否为硬格"——与 `aabb_blocked` 在别处的用法同一语义
/// （"再往 `dir=+1` 走一格会不会被挡"），不依赖运动历史，水平/竖直两个方向
/// 的 footprint 变化都天然覆盖。
///
/// 起跳的脉冲语义（"这一 tick 主动离地，即便查询结果还判定脚下贴地也不算
/// 数"）不在本函数处理——查询是粗粒度整格判定，`jump_speed` 若调得很小
/// 理论上本 tick 仍可能查到"脚下是硬格"，那样 `step_kinematics` 里电平触发
/// 的 `held(BTN_JUMP)` 会每 tick 重新满足闸门条件；这一条由调用方
/// `step_kinematics` 在起跳分支里显式收口（见其内联注释），不塞进这里让
/// 本函数身兼"积分"与"起跳特例"两个职责。
fn sweep_y(c: &mut Creature, world: &World, table: &MaterialTable) {
    let (hw, hh) = (c.half_w, c.half_h);
    let dir: i32 = if c.vy.0 > 0 {
        1
    } else if c.vy.0 < 0 {
        -1
    } else {
        0
    };
    if dir != 0 {
        let target = c.y + c.vy;
        let crossing =
            if dir > 0 { target.to_cell() - c.y.to_cell() } else { c.y.to_cell() - target.to_cell() }.max(0);
        let steps = crossing.min(CREATURE_MAX_STEP);
        let mut blocked = false;
        for _ in 0..steps {
            let ny = c.y + Fx::from_int(dir);
            if aabb_blocked(world, table, c.x, ny, hw, hh) {
                blocked = true;
                break;
            }
            c.y = ny;
        }
        if blocked {
            c.vy = Fx::ZERO;
        } else if steps == crossing {
            c.y = target;
        }
    }
    c.on_ground = aabb_blocked(world, table, c.x, c.y + Fx::from_int(1), hw, hh);
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
