//! 弹体行为集成测试（M4 Task 4 spec §5）：经 `Sim` 公共 API 驱动。范围：直线
//! 飞行、DDA 命中判定（先到者优先：生物 vs 硬格）、grace 防自伤、同队免疫、
//! 出界/寿命耗尽销毁、容量限流、逐 tick 恰好一次积分。侵彻/弹跳/阻力/穿透/
//! 排开/冲量/定时爆（Task 6）与 `Blast`/`Spray`/施法闸门（Task 5）不在本文件
//! 范围内。

mod common;

use sand_core::{spell::*, Fx, Sim, MAX_PROJECTILES};

/// 本 Task 的测试法术表：0 号 = 普通直射弹（life 长），1 号 = 短命弹（life 5）。
fn bolt_table() -> SpellTable {
    SpellTable::from_defs(vec![
        SpellDef::test_bolt(
            "bolt", /* damage_milli */ 5_000, /* knockback */ Fx::from_int(2),
            /* speed */ Fx::from_int(8), /* life */ 120, /* grace */ 4,
        ),
        SpellDef::test_bolt("shortlived", 5_000, Fx::ZERO, Fx::from_int(8), 5, 4),
    ])
}

/// 弹体注入：走 `Sim::queue_projectile`（内部就是 `Projectiles::spawn`，测试
/// 跑的就是产品代码），速度由调用方直接给格/tick。出生点取格心（+半格）——
/// 与 `rules.rs`/`explode.rs`/`creature.rs` 里其它脱格出生点同一惯例
/// （`Fx::from_ratio(1, 2)` 与 `fixed::HALF_CELL` 位模式相同，见 `fixed.rs`
/// 金值测试；用 `from_ratio` 而非导入 `HALF_CELL` 是因为该常量是
/// `pub(crate)`，不对集成测试开放，没必要为一个测试专门放宽核心可见性）。
fn shoot(sim: &mut Sim, spell: u8, x: i32, y: i32, vx: Fx, vy: Fx, owner: u8) {
    let half = Fx::from_ratio(1, 2);
    let (px, py) = (Fx::from_int(x) + half, Fx::from_int(y) + half);
    sim.queue_projectile(spell, px, py, vx, vy, owner);
}

#[test]
fn projectile_flies_straight_and_dies_on_wall() {
    let mut sim = common::arena_wide_open(bolt_table());
    shoot(&mut sim, 0, 10, 64, Fx::from_int(8), Fx::ZERO, 255);
    for _ in 0..60 {
        sim.step(&[], &[]);
    }
    assert_eq!(sim.projectiles().len(), 0, "撞墙后必须销毁");
}

#[test]
fn projectile_damages_a_creature_it_hits() {
    let mut sim = common::arena_with_two_creatures(bolt_table()); // id 0 在左，id 1 在右
    let hp0 = sim.creatures().get(1).unwrap().hp;
    shoot(&mut sim, 0, 30, 64, Fx::from_int(8), Fx::ZERO, 0);
    for _ in 0..40 {
        sim.step(&[], &[]);
    }
    assert!(sim.creatures().get(1).unwrap().hp < hp0, "命中应当扣血");
    assert_eq!(sim.projectiles().len(), 0, "命中生物后弹体销毁");
}

#[test]
fn projectile_knockback_pushes_the_target() {
    let mut sim = common::arena_with_two_creatures(bolt_table());
    let x0 = sim.creatures().get(1).unwrap().x;
    shoot(&mut sim, 0, 30, 64, Fx::from_int(8), Fx::ZERO, 0);
    for _ in 0..40 {
        sim.step(&[], &[]);
    }
    assert!(sim.creatures().get(1).unwrap().x > x0, "击退应把目标推开");
}

#[test]
fn projectile_does_not_hit_its_owner_during_grace() {
    let mut sim = common::arena_with_two_creatures(bolt_table());
    let hp0 = sim.creatures().get(0).unwrap().hp;
    // 出生就在 owner 身上、速度指向它自己：grace = 4，前 3 tick 不得自伤。
    shoot(&mut sim, 0, 20, 64, Fx::from_int(-1), Fx::ZERO, 0);
    for _ in 0..3 {
        sim.step(&[], &[]);
    }
    assert_eq!(sim.creatures().get(0).unwrap().hp, hp0, "grace 帧内不得自伤");
}

#[test]
fn projectile_skips_same_team() {
    let mut sim = common::arena_with_two_creatures_same_team(bolt_table());
    let hp0 = sim.creatures().get(1).unwrap().hp;
    shoot(&mut sim, 0, 30, 64, Fx::from_int(8), Fx::ZERO, 0);
    for _ in 0..40 {
        sim.step(&[], &[]);
    }
    assert_eq!(sim.creatures().get(1).unwrap().hp, hp0, "同队不得命中");
}

#[test]
fn projectile_dies_when_lifetime_runs_out() {
    let mut sim = common::arena_wide_open(bolt_table());
    shoot(&mut sim, 1, 100, 64, Fx::ZERO, Fx::ZERO, 255); // 1 号：life 5，静止不撞墙
    for _ in 0..6 {
        sim.step(&[], &[]);
    }
    assert_eq!(sim.projectiles().len(), 0, "寿命耗尽即销毁");
}

#[test]
fn spawn_beyond_capacity_is_rejected_deterministically() {
    let mut sim = common::arena_wide_open(bolt_table());
    for _ in 0..MAX_PROJECTILES + 10 {
        shoot(&mut sim, 0, 100, 64, Fx::ZERO, Fx::ZERO, 255);
    }
    assert_eq!(sim.projectiles().len(), MAX_PROJECTILES, "超限必须确定性拒绝");
}

#[test]
fn projectile_moves_exactly_once_per_tick() {
    let mut sim = common::arena_wide_open(bolt_table());
    shoot(&mut sim, 0, 10, 64, Fx::from_int(8), Fx::ZERO, 255);
    let x0 = sim.projectiles().x(0);
    sim.step(&[], &[]);
    let x1 = sim.projectiles().x(0);
    assert_eq!(x1 - x0, Fx::from_int(8), "每 tick 恰好走一次速度，不多不少");
}
