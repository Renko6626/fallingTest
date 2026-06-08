import pytest
from core.material import MaterialRegistry
from core.reaction import ReactionTable
from core.grid import CellGrid
from core.cell import AIR

TOML = """
[meta]
version = 1

[materials.wall]
cell_type = "solid"
density = 100
color = [128, 128, 128]
tags = ["solid"]

[materials.sand]
cell_type = "powder"
density = 60
color = [194, 178, 128]
tags = ["powder"]

[materials.water]
cell_type = "liquid"
density = 10
color = [48, 96, 255]
tags = ["liquid", "water"]
dispersion = 5

[materials.oil]
cell_type = "liquid"
density = 8
dispersion = 2
color = [80, 60, 30]
tags = ["liquid", "flammable"]

[materials.steam]
cell_type = "gas"
density = 1
color = [200, 200, 255]
lifetime = 300
tags = ["gas"]
dispersion = 3

[materials.fire]
cell_type = "energy"
density = 0
color = [255, 160, 40]
lifetime = 60
tags = ["energy", "hot"]
"""


@pytest.fixture
def env(tmp_path):
    f = tmp_path / "m.toml"
    f.write_text(TOML)
    reg = MaterialRegistry(str(f))
    table = ReactionTable(str(f), reg)
    return reg, table


def make_grid(env, w=8, h=8):
    reg, table = env
    return CellGrid(w, h, reg, table)


def test_solid_does_not_move(env):
    reg, _ = env
    grid = make_grid(env)
    wall_id = reg.get_by_name("wall").type_id
    grid.set_cell(3, 3, wall_id)
    from core.rules import try_move
    result = try_move(grid, 3, 3)
    assert result is None


def test_powder_falls_down(env):
    reg, _ = env
    grid = make_grid(env)
    sand_id = reg.get_by_name("sand").type_id
    grid.set_cell(3, 0, sand_id)
    from core.rules import try_move
    result = try_move(grid, 3, 0)
    assert result == (3, 1)


def test_powder_falls_diagonal_when_blocked(env):
    reg, _ = env
    grid = make_grid(env)
    sand_id = reg.get_by_name("sand").type_id
    wall_id = reg.get_by_name("wall").type_id
    grid.set_cell(3, 3, sand_id)
    grid.set_cell(3, 4, wall_id)
    from core.rules import try_move
    result = try_move(grid, 3, 3)  # counter RNG：固定 (seed, frame, x, y) → 确定性结果
    assert result is not None
    assert result[1] == 4
    assert result[0] in (2, 4)


def test_powder_stops_at_bottom(env):
    reg, _ = env
    grid = make_grid(env)
    sand_id = reg.get_by_name("sand").type_id
    grid.set_cell(3, 7, sand_id)
    from core.rules import try_move
    result = try_move(grid, 3, 7)
    assert result is None


def test_liquid_spreads_horizontally(env):
    reg, _ = env
    grid = make_grid(env)
    water_id = reg.get_by_name("water").type_id
    wall_id = reg.get_by_name("wall").type_id
    grid.set_cell(3, 6, water_id)
    grid.set_cell(3, 7, wall_id)
    grid.set_cell(2, 7, wall_id)
    grid.set_cell(4, 7, wall_id)
    from core.rules import try_move
    r = try_move(grid, 3, 6)  # 下与斜下全被墙挡 → 必走横向（方向记忆确定）
    assert r is not None
    assert r[1] == 6          # 同一行
    assert r[0] != 3          # 发生了横移（dispersion 下可达多格，不再钉 ±1 落点）


def test_gas_rises(env):
    reg, _ = env
    grid = make_grid(env)
    steam_id = reg.get_by_name("steam").type_id
    grid.set_cell(3, 5, steam_id)
    from core.rules import try_move
    result = try_move(grid, 3, 5)
    assert result is not None
    assert result[1] < 5


def water_columns(grid, water_id):
    return [
        sum(1 for y in range(grid.height) if grid.get_type_id(x, y) == water_id)
        for x in range(grid.width)
    ]


def test_liquid_side_move_commits_direction(env):
    """侧移走 -vel 方向后必须翻转方向记忆（承诺），否则下一次决策先试 +vel
    = 刚腾出的格子 → 打乒乓、液面冻结（2026-06-07 根因）。dispersion 引入后
    水一帧横移多格，故此处直接断言方向承诺不变量（源格 VELOCITY 翻转）。"""
    reg, _ = env
    grid = make_grid(env)
    water_id = reg.get_by_name("water").type_id
    wall_id = reg.get_by_name("wall").type_id
    for x in range(8):
        grid.set_cell(x, 7, wall_id)  # 地板：下/斜下全堵
    grid.set_cell(5, 6, wall_id)      # 右侧堵死 → 只能走左（-vel）
    grid.set_cell(4, 6, water_id)     # vel 初始 +1
    from core.rules import try_move
    from core.cell import VELOCITY
    r = try_move(grid, 4, 6)
    assert r is not None and r[0] < 4, "应向左横移"
    assert grid.cells[grid._base(4, 6) + VELOCITY] == -1, "侧移后方向未承诺"


def test_gas_side_move_commits_direction(env):
    """同款 bug 的气体镜像：横移后必须沿同方向继续。"""
    reg, _ = env
    grid = make_grid(env)
    steam_id = reg.get_by_name("steam").type_id
    wall_id = reg.get_by_name("wall").type_id
    for x in range(8):
        grid.set_cell(x, 0, wall_id)  # 天花板
        grid.set_cell(x, 2, wall_id)  # 下方封死，隔离斜上通道
    grid.set_cell(5, 1, wall_id)      # 右侧堵死
    grid.set_cell(4, 1, steam_id)

    grid.update()
    assert grid.get_type_id(3, 1) == steam_id
    grid.update()
    assert grid.get_type_id(2, 1) == steam_id, "气体侧移后方向未承诺：像素打乒乓"


def test_liquid_levels_out(env):
    """盆中一侧水柱最终必须摊平（液面水平），不得冻结成堆。"""
    reg, table = env
    grid = CellGrid(24, 24, reg, table, seed=1)
    wall_id = reg.get_by_name("wall").type_id
    water_id = reg.get_by_name("water").type_id
    for x in range(24):
        grid.set_cell(x, 23, wall_id)
    for y in range(24):
        grid.set_cell(0, y, wall_id)
        grid.set_cell(23, y, wall_id)
    # 左侧 8 宽 × 12 高水柱 = 96 像素；摊平后 22 列每列 4–5
    for y in range(11, 23):
        for x in range(1, 9):
            grid.set_cell(x, y, water_id)

    for _ in range(800):
        grid.update()

    heights = water_columns(grid, water_id)[1:23]
    assert sum(heights) == 96  # 守恒
    spread = max(heights) - min(heights)
    assert spread <= 2, f"液面未摊平：heights={heights}"


def test_liquid_disperses_to_furthest_air(env):
    """水 dispersion=5：右侧 5 格全空 → 一帧落到 x+5。"""
    reg, table = env
    grid = CellGrid(16, 8, reg, table)
    water_id = reg.get_by_name("water").type_id
    wall_id = reg.get_by_name("wall").type_id
    for x in range(16):
        grid.set_cell(x, 7, wall_id)  # 地板：下/斜下全堵
    grid.set_cell(3, 6, water_id)     # vel 初始 +1
    from core.rules import try_move
    assert try_move(grid, 3, 6) == (8, 6)


def test_liquid_probe_stops_at_obstacle(env):
    """探测中途遇墙截停 → 落墙前最远空格。"""
    reg, table = env
    grid = CellGrid(16, 8, reg, table)
    water_id = reg.get_by_name("water").type_id
    wall_id = reg.get_by_name("wall").type_id
    for x in range(16):
        grid.set_cell(x, 7, wall_id)
    grid.set_cell(6, 6, wall_id)      # x+3 是墙
    grid.set_cell(3, 6, water_id)
    from core.rules import try_move
    assert try_move(grid, 3, 6) == (5, 6)


def test_liquid_displacement_only_at_first_cell(env):
    """首格是更轻液体 → 走 ±1 密度置换，不穿透。"""
    reg, table = env
    grid = CellGrid(16, 8, reg, table)
    water_id = reg.get_by_name("water").type_id
    oil_id = reg.get_by_name("oil").type_id
    wall_id = reg.get_by_name("wall").type_id
    for x in range(16):
        grid.set_cell(x, 7, wall_id)
    grid.set_cell(4, 6, oil_id)       # 右首格：油（密度 8 < 10）
    grid.set_cell(3, 6, water_id)
    from core.rules import try_move
    assert try_move(grid, 3, 6) == (4, 6)  # 置换油，而非越过油落远处


def test_liquid_probe_respects_write_rect(env):
    """写域契约直测：探测在 write_rect 边界截断。"""
    from core.chunks import Rect
    reg, table = env
    grid = CellGrid(16, 8, reg, table)
    water_id = reg.get_by_name("water").type_id
    wall_id = reg.get_by_name("wall").type_id
    for x in range(16):
        grid.set_cell(x, 7, wall_id)
    grid.set_cell(3, 6, water_id)
    grid._write_rect = Rect(0, 0, 6, 8)  # 人为收窄：x ≥ 6 不可写
    from core.rules import try_move
    assert try_move(grid, 3, 6) == (5, 6)  # 截断在域内最远空格


def test_density_swap_heavy_sinks(env):
    reg, _ = env
    grid = make_grid(env)
    sand_id = reg.get_by_name("sand").type_id
    water_id = reg.get_by_name("water").type_id
    grid.set_cell(3, 3, sand_id)
    grid.set_cell(3, 4, water_id)
    from core.rules import try_move
    result = try_move(grid, 3, 3)
    assert result == (3, 4)
