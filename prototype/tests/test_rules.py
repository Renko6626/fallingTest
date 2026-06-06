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

[materials.steam]
cell_type = "gas"
density = 1
color = [200, 200, 255]
lifetime = 300
tags = ["gas"]

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
    assert r[1] == 6
    assert r[0] in (2, 4)


def test_gas_rises(env):
    reg, _ = env
    grid = make_grid(env)
    steam_id = reg.get_by_name("steam").type_id
    grid.set_cell(3, 5, steam_id)
    from core.rules import try_move
    result = try_move(grid, 3, 5)
    assert result is not None
    assert result[1] < 5


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
