from core.ops import apply_brush


def make_grid(tmp_path):
    from core.grid import CellGrid
    from core.material import MaterialRegistry
    from core.reaction import ReactionTable

    f = tmp_path / "m.toml"
    f.write_text(
        """
[meta]
version = 1

[materials.sand]
cell_type = "powder"
density = 60
color = [194, 178, 128]
tags = ["powder"]
"""
    )
    reg = MaterialRegistry(str(f))
    return CellGrid(8, 8, reg, ReactionTable(str(f), reg)), reg


def test_apply_brush_writes_square(tmp_path):
    grid, reg = make_grid(tmp_path)
    sand = reg.get_by_name("sand").type_id
    written = apply_brush(grid, 4, 4, sand, brush_size=3)
    assert set(written) == {(x, y) for x in (3, 4, 5) for y in (3, 4, 5)}
    assert grid.get_type_id(3, 3) == sand


def test_apply_brush_clips_bounds(tmp_path):
    grid, reg = make_grid(tmp_path)
    sand = reg.get_by_name("sand").type_id
    written = apply_brush(grid, 0, 0, sand, brush_size=3)
    assert set(written) == {(0, 0), (0, 1), (1, 0), (1, 1)}
