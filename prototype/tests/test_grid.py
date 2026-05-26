import pytest
from core.material import MaterialRegistry
from core.reaction import ReactionTable
from core.grid import CellGrid
from core.cell import AIR, STRIDE, TYPE_ID, VELOCITY, LIFETIME, FLAGS


@pytest.fixture
def grid(tmp_path):
    toml_content = """
[meta]
version = 1

[materials.wall]
cell_type = "solid"
density = 10.0
color = [128, 128, 128]
tags = ["solid"]

[materials.sand]
cell_type = "powder"
density = 6.0
color = [194, 178, 128]
tags = ["powder"]

[materials.water]
cell_type = "liquid"
density = 1.0
color = [48, 96, 255]
tags = ["liquid", "water"]
"""
    f = tmp_path / "test_materials.toml"
    f.write_text(toml_content)
    reg = MaterialRegistry(str(f))
    table = ReactionTable(str(f), reg)
    return CellGrid(8, 8, reg, table)


def test_grid_initial_state(grid):
    for y in range(8):
        for x in range(8):
            assert grid.get_type_id(x, y) == AIR


def test_set_and_get(grid):
    sand_id = grid.registry.get_by_name("sand").type_id
    grid.set_cell(x=3, y=4, type_id=sand_id)
    assert grid.get_type_id(3, 4) == sand_id


def test_swap(grid):
    sand_id = grid.registry.get_by_name("sand").type_id
    water_id = grid.registry.get_by_name("water").type_id
    grid.set_cell(3, 3, sand_id)
    grid.set_cell(3, 4, water_id)
    grid.swap(3, 3, 3, 4)
    assert grid.get_type_id(3, 3) == water_id
    assert grid.get_type_id(3, 4) == sand_id


def test_in_bounds(grid):
    assert grid.in_bounds(0, 0)
    assert grid.in_bounds(7, 7)
    assert not grid.in_bounds(-1, 0)
    assert not grid.in_bounds(0, 8)
    assert not grid.in_bounds(8, 0)


def test_get_type_id_array(grid):
    sand_id = grid.registry.get_by_name("sand").type_id
    grid.set_cell(0, 0, sand_id)
    arr = grid.get_type_id_array()
    assert len(arr) == 64  # 8 * 8
    assert arr[0] == sand_id
    assert arr[1] == AIR


def test_set_cell_with_lifetime(grid):
    """set_cell should copy lifetime from MaterialDef."""
    import tempfile, os
    toml_content = """
[meta]
version = 1

[materials.steam]
cell_type = "gas"
density = 0.1
color = [200, 200, 255]
lifetime = 300
tags = ["gas"]
"""
    fd, path = tempfile.mkstemp(suffix=".toml")
    try:
        with os.fdopen(fd, "w") as f:
            f.write(toml_content)
        reg = MaterialRegistry(path)
        table = ReactionTable(path, reg)
        g = CellGrid(4, 4, reg, table)
        steam_id = reg.get_by_name("steam").type_id
        g.set_cell(1, 1, steam_id)
        base = (1 * 4 + 1) * STRIDE
        assert g.cells[base + LIFETIME] == 300
    finally:
        os.unlink(path)


# --- update() 测试 ---

FULL_TOML = """
[meta]
version = 1

[materials.wall]
cell_type = "solid"
density = 10.0
color = [128, 128, 128]
tags = ["solid"]

[materials.sand]
cell_type = "powder"
density = 6.0
color = [194, 178, 128]
tags = ["powder"]

[materials.water]
cell_type = "liquid"
density = 1.0
color = [48, 96, 255]
tags = ["liquid", "water"]

[materials.oil]
cell_type = "liquid"
density = 0.8
color = [80, 60, 30]
tags = ["liquid", "flammable"]

[materials.lava]
cell_type = "liquid"
density = 3.0
color = [255, 96, 0]
tags = ["liquid", "lava", "hot"]

[materials.steam]
cell_type = "gas"
density = 0.1
color = [200, 200, 255]
lifetime = 300
tags = ["gas"]

[materials.fire]
cell_type = "energy"
density = 0.0
color = [255, 160, 40]
lifetime = 3
tags = ["energy", "hot"]

[materials.rock]
cell_type = "solid"
density = 9.0
color = [100, 100, 100]
tags = ["solid"]

[[reactions]]
input = ["lava", "water"]
output = ["rock", "steam"]
probability = 1.0
"""


@pytest.fixture
def full_env(tmp_path):
    f = tmp_path / "full.toml"
    f.write_text(FULL_TOML)
    reg = MaterialRegistry(str(f))
    table = ReactionTable(str(f), reg)
    return reg, table


def test_sand_falls_to_bottom(full_env):
    import random
    random.seed(42)
    reg, table = full_env
    grid = CellGrid(8, 8, reg, table)
    sand_id = reg.get_by_name("sand").type_id
    grid.set_cell(3, 0, sand_id)
    for _ in range(20):
        grid.update()
    assert grid.get_type_id(3, 7) == sand_id
    assert grid.get_type_id(3, 0) == AIR


def test_sand_stacks(full_env):
    import random
    random.seed(42)
    reg, table = full_env
    grid = CellGrid(8, 8, reg, table)
    sand_id = reg.get_by_name("sand").type_id
    wall_id = reg.get_by_name("wall").type_id
    # Build a 1-wide column with walls so sand can only stack vertically
    for y in range(8):
        grid.set_cell(2, y, wall_id)
        grid.set_cell(4, y, wall_id)
    for x in range(8):
        grid.set_cell(x, 7, wall_id)
    grid.set_cell(3, 0, sand_id)
    grid.set_cell(3, 1, sand_id)
    for _ in range(20):
        grid.update()
    assert grid.get_type_id(3, 6) == sand_id
    assert grid.get_type_id(3, 5) == sand_id


def test_oil_floats_on_water(full_env):
    import random
    random.seed(42)
    reg, table = full_env
    # Use a 3-wide grid: walls at x=0 and x=2, bottom wall at y=4
    # This creates a single-cell-wide column at x=1
    grid = CellGrid(3, 5, reg, table)
    water_id = reg.get_by_name("water").type_id
    oil_id = reg.get_by_name("oil").type_id
    wall_id = reg.get_by_name("wall").type_id
    for y in range(5):
        grid.set_cell(0, y, wall_id)
        grid.set_cell(2, y, wall_id)
    grid.set_cell(1, 4, wall_id)
    # Place water below oil - oil should float above water after settling
    grid.set_cell(1, 1, water_id)
    grid.set_cell(1, 0, oil_id)
    for _ in range(20):
        grid.update()
    col = [grid.get_type_id(1, y) for y in range(5)]
    oil_y = next(y for y in range(5) if col[y] == oil_id)
    water_y = next(y for y in range(5) if col[y] == water_id)
    assert oil_y < water_y


def test_lifetime_expires(full_env):
    import random
    random.seed(42)
    reg, table = full_env
    grid = CellGrid(4, 4, reg, table)
    fire_id = reg.get_by_name("fire").type_id
    grid.set_cell(2, 2, fire_id)
    for _ in range(10):
        grid.update()
    has_fire = any(
        grid.get_type_id(x, y) == fire_id
        for x in range(4) for y in range(4)
    )
    assert not has_fire


def test_lava_water_reaction(full_env):
    import random
    random.seed(42)
    reg, table = full_env
    grid = CellGrid(8, 8, reg, table)
    lava_id = reg.get_by_name("lava").type_id
    water_id = reg.get_by_name("water").type_id
    rock_id = reg.get_by_name("rock").type_id
    steam_id = reg.get_by_name("steam").type_id
    wall_id = reg.get_by_name("wall").type_id
    for x in range(8):
        grid.set_cell(x, 7, wall_id)
    grid.set_cell(3, 6, lava_id)
    grid.set_cell(4, 6, water_id)
    for _ in range(30):
        grid.update()
    all_types = set()
    for x in range(8):
        for y in range(8):
            all_types.add(grid.get_type_id(x, y))
    assert rock_id in all_types or steam_id in all_types


def test_integration_with_real_toml():
    """Use the actual materials.toml to verify everything wires up."""
    import random
    from pathlib import Path
    random.seed(123)
    toml_path = str(Path(__file__).parent.parent / "data" / "materials.toml")
    reg = MaterialRegistry(toml_path)
    table = ReactionTable(toml_path, reg)
    grid = CellGrid(16, 16, reg, table)

    sand_id = reg.get_by_name("sand").type_id
    water_id = reg.get_by_name("water").type_id
    wall_id = reg.get_by_name("wall").type_id

    for x in range(16):
        grid.set_cell(x, 15, wall_id)

    for y in range(5):
        grid.set_cell(8, y, sand_id)

    for x in range(4, 12):
        grid.set_cell(x, 14, water_id)

    for _ in range(100):
        grid.update()

    assert grid.get_type_id(8, 0) == AIR
