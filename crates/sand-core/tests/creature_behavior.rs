//! 生物行为集成测试（M4 Task 2 spec §4.1/§4.2）：经 `Sim` 公共 API 驱动。
//! 范围：逐轴扫掠碰撞、跨台阶、踩得住刚体盖章格、起跳闸门、id 稳定与容量拒绝。
//! 材质接触伤害 / 排开 / 游泳留 Task 3。

mod common;

use common::floor_world_with_creature;
use sand_core::{
    input::*, world::Op, CreatureTable, Fx, InitConfig, ReactionTable, ScanMode, Sim, SpellTable, MAX_CREATURES,
    MAT_WALL,
};

/// 4×2 chunk 的世界，底行 wall；(32, 100) 一个 controller 0 的生物，模板走
/// `CreatureTable::default_player()`（R5：本 Task 建的测试专用模板）。
fn floor_world() -> (sand_core::Sim, u8) {
    floor_world_with_creature(CreatureTable::default_player(), SpellTable::empty())
}

/// 造一个有台沿的世界（`on_ground_is_false_whenever_the_aabb_has_fully_left_the_ledge`
/// 专用）：台面只铺到 x=32，x>=33 悬空；生物出生在台面正上方很高处
/// （y=5），先长距离自由落体再走出台沿。
///
/// **这组具体数值不是随手取的**：bug 只在"横向刚好在离台的那个 tick 跨过
/// 整格边界"与"竖向刚好处在落地后的 stale 窗口内"两件事重合时才会现形
/// （见 `sweep_y` 头注——旧代码在 `crossing == 0` 时原样保留旧
/// `on_ground`，而 `crossing` 完全独立于横向位置，只由竖向速度的累积历史
/// 决定），二者本无必然联系。从出生点直接贴着台面（如 `floor_world` 的
/// (32,100)/row 127）落地几乎不产生水平位移（`accel_air` 很弱，短距离
/// 下坠攒不出可观的 `vx`），bug 窗口和"横向跨格"这两件事永远碰不上；
/// 从很高处（y=5）一路按右下坠，`accel_air` 有更长时间积累水平速度，
/// 落地时已经带着可观的横向动量，紧邻台沿的最后一次跨格更可能落进落地
/// 后的短暂 stale 窗口——2026-09-05 评审复审第二轮通过遍历出生高度/台沿
/// 位置实测搜出这组数值，用 `git stash` 反复对照旧代码确认稳定复现。
///
/// 世界比 `floor_world` 高得多（4×6 chunk = 384 行）：`floor_world` 的
/// 128 行世界里地板贴着世界竖直边界，生物真掉出台面后下坠一两格就会先
/// 撞上**世界边界哨兵**（`World::cell` 越界读返回 WALL，`world.rs` 头注
/// "越界读返回 WALL 哨兵"）这个隐式地板，把"离台悬空"和"摔到世界盒子底"
/// 这两件不同的事混在一起。
fn ledge_world() -> (Sim, u8) {
    let table = common::materials();
    let wall = table.id_by_name("wall").unwrap();
    let cfg = InitConfig { width_chunks: 4, height_chunks: 6, seed: 42, threads: 1, scan: ScanMode::LiveRect };
    let reactions = ReactionTable::empty(&table);
    let mut sim = Sim::new(&cfg, table, reactions, CreatureTable::default_player(), SpellTable::empty()).unwrap();
    sim.apply_setup(&[
        Op::Fill { material: wall, x0: 0, y0: 127, x1: 32, y1: 127 },
        Op::SpawnCreature { x: 32, y: 5, template: 0, team: 0, controller: 0, loadout: [255; MAX_SLOTS] },
    ]);
    (sim, 0)
}

#[test]
fn creature_falls_and_lands_on_floor() {
    let (mut sim, id) = floor_world();
    for _ in 0..120 {
        sim.step(&[], &[]);
    }
    let c = sim.creatures().get(id).unwrap();
    assert!(c.on_ground, "该落地了");
    assert!(c.vy == Fx::ZERO, "落地后竖直速度清零");
}

/// 回归测试（评审 Important #1）：落地后 `on_ground` 不得再抖回 `false`。
/// `step_kinematics` 每 tick 无条件加重力，`sweep_y` 在"本 tick 位移不足一格、
/// 未跨越格边界"时会跳过 `aabb_blocked` 检测——这本身没问题，但若这种情况下
/// 仍无条件把 `on_ground` 置 `false`，就是在没有任何新证据的前提下推翻上一
/// tick 的落地判定，静止在地面上的生物会瞬时误报"悬空"，起跳判定（仅
/// `on_ground` 时生效）会在这个窗口内静默吃掉跳跃输入。逐 tick 检查：一旦
/// 观察到 `on_ground == true`，后续任何一 tick 都不得再变回 `false`。
#[test]
fn on_ground_does_not_flicker_after_landing() {
    let (mut sim, id) = floor_world();
    let mut landed = false;
    for tick in 0..150 {
        sim.step(&[], &[]);
        let on_ground = sim.creatures().get(id).unwrap().on_ground;
        if on_ground {
            landed = true;
        } else if landed {
            panic!("tick {tick}: on_ground 落地后又抖回 false");
        }
    }
    assert!(landed, "150 tick 内应该已经落地");
}

/// 回归测试（评审复审：Important #1 修复引入的镜像回归）：`sweep_y` 在
/// "未跨格因而跳过检测"时原样保留上一 tick 的 `on_ground`——这个等价只在
/// **纯竖直运动**时成立。`sweep_x` 先于 `sweep_y` 执行，只改 `c.x`，从不碰
/// `on_ground`：生物若在这个"未跨格"的窗口里水平走出台阶边缘，footprint
/// 其实已经变了，旧代码却继续沿用"站在旧 x 位置时"验证过的 `true`。
///
/// 挖空地板右半制造台沿，从高处落体按右走出台沿后**逐 tick**（不止看第
/// 一次翻转的那一 tick——bug 窗口只在竖向 stale 期间才存在，只看单点容易
/// 漏判）检查一条不变量：**AABB 已经完全脱离台面时，`on_ground` 不得为
/// `true`**——这与"起跳闸门会不会在这个窗口内误放行"是同一件事的另一种
/// （更直接、不依赖具体按键时机）表述，覆盖面比"当场按一下跳"更彻底。
#[test]
fn on_ground_is_false_whenever_the_aabb_has_fully_left_the_ledge() {
    let (mut sim, id) = ledge_world();

    let half_w = 2; // CreatureTable::default_player() 的 half_w
    // 台沿在 x=33（`ledge_world` 只铺到 x=32），AABB 左边缘越过它才是彻底
    // 脱离台面——半只脚还搭在台沿上时 on_ground=true 依然合法。
    let ledge_x = 33;
    let mut fully_cleared_at_least_once = false;
    // 离台后只再观察 20 tick：生物竖直速度不设上限，久等只会等到它摔穿
    // `ledge_world` 的净空触底（那也会被判定成 on_ground=true，但那是"摔
    // 到世界盒子底"，不是本测试要抓的"离台瞬间"那类 bug），窗口需要卡在
    // 离台事件附近而不是放到底。
    let mut ticks_since_cleared: Option<u32> = None;
    for tick in 0..80 {
        sim.step(&[], &[InputFrame::new(BTN_RIGHT, 0, 0)]);
        let c = sim.creatures().get(id).unwrap();
        if c.x.to_cell() - half_w >= ledge_x {
            fully_cleared_at_least_once = true;
            assert!(!c.on_ground, "tick {tick}: AABB 已完全离开台面，on_ground 却仍是 true");
            let elapsed = *ticks_since_cleared.get_or_insert(0);
            if elapsed >= 20 {
                break;
            }
            ticks_since_cleared = Some(elapsed + 1);
        }
    }
    assert!(fully_cleared_at_least_once, "80 tick 内应该已经完全走出台沿");
}

/// 回归测试（防止本轮修复引入的另一坑：`on_ground` 若改成"纯查询"，起跳
/// 那一 tick 生物仍紧贴地面，查询会继续读到 `true`，`held(BTN_JUMP)` 是电平
/// 触发，按住不放会每 tick 重新施加起跳速度 → 悬浮/连跳）。落地后持续按住
/// 跳跃键：起跳后 `vy` 应该只受重力单调回升，不得被跳跃键在仍处于本次跳跃
/// 弧线中途时重新拍回 `-jump_speed` 附近的大冲量。
#[test]
fn holding_jump_launches_only_once_not_every_tick() {
    let (mut sim, id) = floor_world();
    for _ in 0..120 {
        sim.step(&[], &[]);
    } // 先落地站稳
    let mut prev_vy: Option<Fx> = None;
    for tick in 0..15 {
        // 15 tick 远小于 jump_speed=2.9 / GRAVITY=0.25 折返地面所需的 tick 数，
        // 全程应保持在同一次跳跃弧线内，不涉及"落地后按住是否该再跳一次"
        // 这类另有取舍的场景。
        sim.step(&[], &[InputFrame::new(BTN_JUMP, 0, 0)]); // 持续按住
        let vy = sim.creatures().get(id).unwrap().vy;
        if let Some(p) = prev_vy {
            assert!(vy >= p, "tick {tick}: vy 从 {p:?} 变成 {vy:?}——疑似悬空中被跳跃键重新施加起跳速度");
        }
        prev_vy = Some(vy);
    }
}

#[test]
fn creature_walks_right_when_right_is_held() {
    let (mut sim, id) = floor_world();
    let x0 = sim.creatures().get(id).unwrap().x;
    for _ in 0..60 {
        sim.step(&[], &[InputFrame::new(BTN_RIGHT, 0, 0)]);
    }
    assert!(sim.creatures().get(id).unwrap().x > x0, "按右应该往右走");
}

#[test]
fn creature_is_blocked_by_a_wall_column() {
    // 右侧竖一道 wall，走 300 tick 也不能穿过去
    let (mut sim, id) = floor_world();
    sim.apply_setup(&[Op::Fill { material: MAT_WALL, x0: 40, y0: 0, x1: 40, y1: 127 }]);
    for _ in 0..300 {
        sim.step(&[], &[InputFrame::new(BTN_RIGHT, 0, 0)]);
    }
    assert!(sim.creatures().get(id).unwrap().x.to_cell() < 40, "撞墙不得穿过");
}

#[test]
fn creature_climbs_over_a_three_cell_step_but_not_four() {
    // climb_over_y = 3：3 格台阶自动跨上去，4 格挡住
    for (h, should_pass) in [(3i32, true), (4, false)] {
        let (mut sim, id) = floor_world();
        // 台阶顶面比地板本身的实心行（127，见 floor_world）高 h 格：
        // top = 127 - h，而不是 126 - h——126 是平地站立时脚底恰好贴着的
        // 那一格（开放，非实心），拿它当基准会让整个台阶多算一格高度。
        let top = 127 - h;
        sim.apply_setup(&[Op::Fill { material: MAT_WALL, x0: 40, y0: top, x1: 41, y1: 126 }]);
        for _ in 0..300 {
            sim.step(&[], &[InputFrame::new(BTN_RIGHT, 0, 0)]);
        }
        let passed = sim.creatures().get(id).unwrap().x.to_cell() > 41;
        assert_eq!(passed, should_pass, "台阶高 {h} 的跨越结果不符");
    }
}

#[test]
fn jump_only_works_on_ground() {
    let (mut sim, id) = floor_world();
    for _ in 0..120 {
        sim.step(&[], &[]);
    } // 先落地
    sim.step(&[], &[InputFrame::new(BTN_JUMP, 0, 0)]);
    assert!(sim.creatures().get(id).unwrap().vy < Fx::ZERO, "起跳应有向上速度");
    let vy_air = sim.creatures().get(id).unwrap().vy;
    sim.step(&[], &[InputFrame::new(BTN_JUMP, 0, 0)]); // 空中再按
    assert!(sim.creatures().get(id).unwrap().vy > vy_air, "空中按跳不得二段跳");
}

#[test]
fn creature_stands_on_a_stamped_rigid_body() {
    // M3 木箱盖章格对生物就是地形（spec §4.2）：箱子本身无支撑，会在引擎重力下
    // 一路沉到地板（row 127），生物随箱顶一起被"驮"下去，最终落点比出生点
    // （y=100）更低——这是箱子自身物理的正常结果，不代表"穿透"。用绝对
    // 出生高度断言不可靠（实测：稳定后箱顶约 row 111，站上去 y.to_cell()
    // 约 107，永远达不到 < 100）。改用相对判据：与平地落地高度（121，见
    // `creature_falls_and_lands_on_floor` 同一套物理）比——明显更高（更小），
    // 证明生物全程被箱顶盖章格接住，从未真正落到地板上。
    let (mut sim, id) = floor_world();
    let wood = sim.table().id_by_name("wood").unwrap();
    sim.apply_setup(&[Op::SpawnBody { material: wood, x: 30, y: 100, w: 16, h: 16, angle_deg: 0 }]);
    for _ in 0..200 {
        sim.step(&[], &[]);
    }
    let c = sim.creatures().get(id).unwrap();
    assert!(c.on_ground, "应踩在箱顶上：on_ground 应为真");
    assert!(c.y.to_cell() < 121, "应停在箱顶（明显高于平地落地高度 121），实际 {}", c.y.to_cell());
}

#[test]
fn creature_id_is_stable_and_never_recycled() {
    let (mut sim, _) = floor_world();
    let n = sim.creatures().len();
    sim.step(
        &[Op::SpawnCreature { x: 10, y: 10, template: 0, team: 1, controller: 255, loadout: [255; MAX_SLOTS] }],
        &[],
    );
    assert_eq!(sim.creatures().len(), n + 1, "新生物追加在末尾，id = 下标");
}

#[test]
fn spawn_beyond_capacity_is_rejected_deterministically() {
    let (mut sim, _) = floor_world();
    for _ in 0..MAX_CREATURES + 5 {
        sim.step(
            &[Op::SpawnCreature { x: 10, y: 10, template: 0, team: 1, controller: 255, loadout: [255; MAX_SLOTS] }],
            &[],
        );
    }
    assert_eq!(sim.creatures().len(), MAX_CREATURES, "超限必须确定性拒绝");
}

// ==================== M4 Task 3：世界互动（排开 / 游泳 / 接触伤害 / HP）====================
// spec §4.3–§4.5。材质 id 一律经 `table.id_by_name` 现查（R6：测试里不许硬编码材质
// id）——`common::materials()` 里 `water`/`fire` 的具体数值参见该函数文档。

#[test]
fn running_through_water_displaces_it_into_particles() {
    let (mut sim, _id) = floor_world();
    let water = sim.table().id_by_name("water").unwrap();
    sim.apply_setup(&[Op::Fill { material: water, x0: 40, y0: 120, x1: 80, y1: 126 }]);
    let before = sim.world().count_material(water);
    for _ in 0..200 {
        sim.step(&[], &[InputFrame::new(BTN_RIGHT, 0, 0)]);
    }
    // 排开的水成粒子后仍会落回网格：总量（网格 + 在飞粒子）不得凭空减少。
    // **这不是一条普适保证**——`Particles::spawn` 在池满时确定性拒绝
    // （`rejected_total`），落地时"埋住"（四邻都非空）会被删除且不写回网格
    // （`buried_total`，M1 已知的质量非守恒口子）。本场景规模（浅水洼、
    // 200 tick）远低于触发这两条的门槛，故此处成立；不代表任意规模下都成立。
    let after = sim.world().count_material(water) + sim.particles().len();
    assert!(after >= before, "排开不得损失水量：{before} → {after}");
    assert!(!sim.particles().is_empty() || after == before, "应当产生过水花");
}

#[test]
fn displacement_is_capped_per_tick() {
    // 整个身子泡在水里，单 tick 排开数不得超过模板上限
    let (mut sim, _id) = floor_world();
    let water = sim.table().id_by_name("water").unwrap();
    sim.apply_setup(&[Op::Fill { material: water, x0: 0, y0: 90, x1: 255, y1: 126 }]);
    let before = sim.world().count_material(water);
    sim.step(&[], &[InputFrame::new(BTN_RIGHT, 0, 0)]);
    let removed = before - sim.world().count_material(water);
    // CreatureTable::default_player() 的 max_displace_per_tick = 24（与
    // data/creatures.ron 的 player 条目同源，R5）。
    assert!(removed <= 24, "单 tick 排开 {removed} 超过模板上限 24");
}

#[test]
fn creature_floats_in_deep_water_instead_of_sinking_to_bottom() {
    let (mut sim, id) = floor_world();
    let water = sim.table().id_by_name("water").unwrap();
    sim.apply_setup(&[Op::Fill { material: water, x0: 0, y0: 64, x1: 255, y1: 126 }]);
    for _ in 0..600 {
        sim.step(&[], &[]);
    }
    let y = sim.creatures().get(id).unwrap().y.to_cell();
    assert!(y < 120, "浮力应当托住，不该沉到池底：y={y}");
}

#[test]
fn standing_in_fire_kills_the_creature() {
    let (mut sim, id) = floor_world();
    let fire = sim.table().id_by_name("fire").unwrap();
    // 生物脚下持续供火（fire 有 lifetime，用 t%4 的脚本补给，防止它衰变熄灭）。
    for t in 0..1200 {
        if t % 4 == 0 {
            sim.step(&[Op::Fill { material: fire, x0: 28, y0: 118, x1: 36, y1: 126 }], &[]);
        } else {
            sim.step(&[], &[]);
        }
        if !sim.creatures().get(id).unwrap().alive {
            break;
        }
    }
    assert!(!sim.creatures().get(id).unwrap().alive, "站火里应当被烧死");
}

#[test]
fn contact_damage_below_min_cell_count_is_ignored() {
    // 只有 2 格火（< min_cell_count = 4），泡 3600 tick 也不掉血
    let (mut sim, id) = floor_world();
    let fire = sim.table().id_by_name("fire").unwrap();
    let hp0 = sim.creatures().get(id).unwrap().hp;
    for t in 0..3600 {
        if t % 4 == 0 {
            sim.step(&[Op::Fill { material: fire, x0: 32, y0: 124, x1: 33, y1: 124 }], &[]);
        } else {
            sim.step(&[], &[]);
        }
    }
    assert_eq!(sim.creatures().get(id).unwrap().hp, hp0, "不足 4 格接触必须整项忽略");
}

#[test]
fn dead_creature_keeps_its_id_and_stops_moving() {
    // R7 裁决：brief 原始伪代码写的 `kill_for_test` 不存在——改用
    // `set_hp(id, 0)` + 一次 `step`，让死亡在这一 tick 的 `step_world_interaction`
    // 里落地，再以落地后的坐标为墓碑基准，比对后续 tick 是否纹丝不动。
    let (mut sim, id) = floor_world();
    sim.creatures_mut().set_hp(id, 0);
    sim.step(&[], &[]);
    assert!(!sim.creatures().get(id).unwrap().alive, "hp<=0 应已在这一 step 内落地为墓碑");
    let (x, y) = {
        let c = sim.creatures().get(id).unwrap();
        (c.x, c.y)
    };
    for _ in 0..120 {
        sim.step(&[], &[InputFrame::new(BTN_RIGHT, 0, 0)]);
    }
    let c = sim.creatures().get(id).unwrap();
    assert!(!c.alive && c.x == x && c.y == y, "墓碑不动，且 id 仍在原位");
}

// ==================== 评审修复：游泳 up/down 方向语义（Important #2）====================
// spec 原样抄 Noita 的 idle 1.2 / up 0.9 / down 0.7，但漏了 Noita 玩家水里还有一份独立
// 喷射推力（那三个系数只调被动浮力）；我们没有那份推力，直接照抄会让"按住上"反而比
// 什么都不按沉得更快。裁决：`swim_buoyancy_up` 提到 1.4（> idle 的 1.2），方向语义
// up > idle > down 成立。这两条测试要能真正卡住方向——分别独立验证"按上净上浮、
// 且比 idle 快"与"按下净下沉"，不能只测"不沉底"（那条 idle 早就测过）。

/// 深水中持续按住 `BTN_JUMP`：净上浮，且比什么都不按（idle）浮得更快。
/// 用两个独立 `Sim` 各跑 `ticks`，比较终态 `y`（`y` 越小 = 越靠水面）。
#[test]
fn holding_swim_up_floats_faster_than_idle() {
    let ticks = 30;

    let (mut sim_idle, id_idle) = floor_world();
    let water = sim_idle.table().id_by_name("water").unwrap();
    sim_idle.apply_setup(&[Op::Fill { material: water, x0: 0, y0: 0, x1: 255, y1: 126 }]);
    for _ in 0..ticks {
        sim_idle.step(&[], &[]);
    }
    let y_idle = sim_idle.creatures().get(id_idle).unwrap().y.to_cell();

    let (mut sim_up, id_up) = floor_world();
    let water = sim_up.table().id_by_name("water").unwrap();
    sim_up.apply_setup(&[Op::Fill { material: water, x0: 0, y0: 0, x1: 255, y1: 126 }]);
    for _ in 0..ticks {
        sim_up.step(&[], &[InputFrame::new(BTN_JUMP, 0, 0)]);
    }
    let y_up = sim_up.creatures().get(id_up).unwrap().y.to_cell();

    assert!(y_up < y_idle, "按住上应比 idle 浮得更快（y 更小）：idle={y_idle} up={y_up}");
}

/// 深水中持续按住 `BTN_DOWN`：净下沉（`y` 增大），不应该反而浮起来。
#[test]
fn holding_swim_down_sinks_instead_of_floating() {
    let (mut sim, id) = floor_world();
    let water = sim.table().id_by_name("water").unwrap();
    sim.apply_setup(&[Op::Fill { material: water, x0: 0, y0: 0, x1: 255, y1: 126 }]);
    let y0 = sim.creatures().get(id).unwrap().y.to_cell();
    for _ in 0..30 {
        sim.step(&[], &[InputFrame::new(BTN_DOWN, 0, 0)]);
    }
    let y1 = sim.creatures().get(id).unwrap().y.to_cell();
    assert!(y1 > y0, "按住下应该下沉（y 增大）：{y0} → {y1}");
}
