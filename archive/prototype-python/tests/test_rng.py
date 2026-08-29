from core.rng import (
    MASK32,
    frame_seed,
    order2,
    perm3,
    rng_chance,
    rng_u32,
    squirrel5,
    threshold_u32,
)


def test_u32_range():
    for pos in (0, 1, 12345, 0xFFFFFFFF):
        v = squirrel5(pos, 0)
        assert 0 <= v <= MASK32


def test_deterministic():
    assert squirrel5(42, 7) == squirrel5(42, 7)
    assert rng_u32(99, 0, 3, 5, 1, 0) == rng_u32(99, 0, 3, 5, 1, 0)
    assert frame_seed(1, 2) == frame_seed(1, 2)


def test_key_component_independence():
    base = rng_u32(100, 0, 10, 20, 1, 0)
    assert rng_u32(101, 0, 10, 20, 1, 0) != base   # fseed
    assert rng_u32(100, 1, 10, 20, 1, 0) != base   # pass_id
    assert rng_u32(100, 0, 11, 20, 1, 0) != base   # x
    assert rng_u32(100, 0, 10, 21, 1, 0) != base   # y
    assert rng_u32(100, 0, 10, 20, 2, 0) != base   # salt
    assert rng_u32(100, 0, 10, 20, 1, 1) != base   # attempt


def test_threshold_u32():
    assert threshold_u32(0.0) == 0
    assert threshold_u32(1.0) == 0xFFFFFFFF          # 钳位
    assert threshold_u32(0.5) == 2147483648          # 0.5 × 2^32 精确
    assert rng_chance(1, 0, 0, 0, 1, threshold_u32(1.0)) in (True, False)


def test_order2_and_perm3_values():
    assert order2(5, 0, 1, 2, 1) in ((0, 1), (1, 0))
    assert perm3(5, 0, 1, 2, 3) in (
        (0, 1, 2), (0, 2, 1), (1, 0, 2), (1, 2, 0), (2, 0, 1), (2, 1, 0),
    )


# 金值锚点：防 squirrel5 实现被无声改动（提案 M0 验收②）。
# 生成命令见 docs/superpowers/plans/2026-06-07-m0-determinism-plan.md Step 1.5。
GOLDEN = {
    (0x0, 0x0): 0x16791E00,
    (0x1, 0x0): 0xC895CB1D,
    (0x0, 0x1): 0x23F6C851,
    (0x3039, 0x10932): 0x3D9BAAAB,
    (0xFFFFFFFF, 0xDEADBEEF): 0x42FE06A2,
}


def test_golden_values():
    assert GOLDEN, "金值未生成——按计划 Step 1.5 填入"
    for (pos, seed), expected in GOLDEN.items():
        assert squirrel5(pos, seed) == expected


def test_no_global_random_in_sim_modules():
    """D2 防回归：sim 模块禁止全局 random 顺序流。"""
    import inspect

    import core.grid
    import core.rules

    for mod in (core.grid, core.rules):
        src = inspect.getsource(mod)
        assert "import random" not in src, f"{mod.__name__} 不得使用全局 random（D2）"
