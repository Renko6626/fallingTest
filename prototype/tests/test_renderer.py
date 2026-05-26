import pytest
import numpy as np
from core.material import MaterialRegistry
from core.reaction import ReactionTable
from core.grid import CellGrid
from core.cell import AIR


@pytest.fixture
def env(tmp_path):
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
color_variance = 15
tags = ["powder"]
"""
    f = tmp_path / "m.toml"
    f.write_text(toml_content)
    reg = MaterialRegistry(str(f))
    table = ReactionTable(str(f), reg)
    grid = CellGrid(4, 4, reg, table)
    return reg, grid


def test_color_lut_shape(env):
    reg, grid = env
    from render.pygame_renderer import build_color_lut
    lut = build_color_lut(reg)
    assert lut.shape == (reg.max_id + 1, 3)
    assert lut.dtype == np.uint8


def test_color_lut_air_is_black(env):
    reg, grid = env
    from render.pygame_renderer import build_color_lut
    lut = build_color_lut(reg)
    assert tuple(lut[0]) == (0, 0, 0)


def test_color_lut_values(env):
    reg, grid = env
    from render.pygame_renderer import build_color_lut
    lut = build_color_lut(reg)
    wall = reg.get_by_name("wall")
    assert tuple(lut[wall.type_id]) == (128, 128, 128)


def test_render_buffer_shape(env):
    reg, grid = env
    from render.pygame_renderer import build_color_lut, render_to_array
    lut = build_color_lut(reg)
    buf = render_to_array(grid, lut, variance_matrix=None)
    assert buf.shape == (4, 4, 3)
    assert buf.dtype == np.uint8


def test_render_buffer_content(env):
    reg, grid = env
    sand_id = reg.get_by_name("sand").type_id
    grid.set_cell(1, 2, sand_id)
    from render.pygame_renderer import build_color_lut, render_to_array
    lut = build_color_lut(reg)
    buf = render_to_array(grid, lut, variance_matrix=None)
    assert tuple(buf[1, 2]) == (194, 178, 128)
    assert tuple(buf[0, 0]) == (0, 0, 0)
