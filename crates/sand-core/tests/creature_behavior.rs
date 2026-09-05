//! 生物行为集成测试（M4 Task 2 spec §4.1/§4.2）：经 `Sim` 公共 API 驱动。
//! 范围：逐轴扫掠碰撞、跨台阶、踩得住刚体盖章格、起跳闸门、id 稳定与容量拒绝。
//! 材质接触伤害 / 排开 / 游泳留 Task 3。

mod common;

use common::floor_world_with_creature;
use sand_core::{input::*, world::Op, CreatureTable, Fx, MAX_CREATURES, MAT_WALL};

/// 4×2 chunk 的世界，底行 wall；(32, 100) 一个 controller 0 的生物，模板走
/// `CreatureTable::default_player()`（R5：本 Task 建的测试专用模板）。
fn floor_world() -> (sand_core::Sim, u8) {
    floor_world_with_creature(CreatureTable::default_player())
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
