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
