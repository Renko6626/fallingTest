"""M0.5 语义验收：缝隙守恒 / 跨缝运动 / 产物盖戳 / 确定性（192×128 = 3×2 chunk）。

128×128 每 pass 只有 1 个活跃 chunk，跑不到多 chunk 路径（评审 m4）——
本套件用 192×128：垂直缝 x=64/128，水平缝 y=64。
"""
import random
from pathlib import Path

from core.cell import STRIDE, TYPE_ID
from core.chunks import Rect
from core.grid import CellGrid
from core.material import MaterialRegistry
from core.reaction import ReactionTable

REAL_TOML = str(Path(__file__).parent.parent / "data" / "materials.toml")
W, H = 192, 128


def real_env():
    reg = MaterialRegistry(REAL_TOML)
    return reg, ReactionTable(REAL_TOML, reg)


def count_by_type(grid: CellGrid) -> dict[int, int]:
    counts: dict[int, int] = {}
    for i in range(grid.width * grid.height):
        t = grid.cells[i * STRIDE + TYPE_ID]
        counts[t] = counts.get(t, 0) + 1
    return counts


def build_mixed(seed: int = 7) -> CellGrid:
    reg, table = real_env()
    grid = CellGrid(W, H, reg, table, seed=seed)
    wall = reg.get_by_name("wall").type_id
    sand = reg.get_by_name("sand").type_id
    water = reg.get_by_name("water").type_id
    for x in range(W):
        grid.set_cell(x, H - 1, wall)
    for x in range(40, 152):          # 横跨 x=64 与 x=128 两条垂直缝
        for y in range(20, 50):       # 下落路径穿过 y=64 水平缝
            grid.set_cell(x, y, sand)
    for x in range(40, 152):
        for y in range(100, 115):
            grid.set_cell(x, y, water)
    return grid


def test_conservation_across_seams():
    """缝隙源/汇 bug 的最强探测器：逐材质计数 N 帧不变（场景无 lifetime 材质）。"""
    grid = build_mixed()
    before = count_by_type(grid)
    for _ in range(80):
        grid.update()
    assert count_by_type(grid) == before


def test_sand_column_crosses_horizontal_seam():
    reg, table = real_env()
    grid = CellGrid(W, H, reg, table)
    wall = reg.get_by_name("wall").type_id
    sand = reg.get_by_name("sand").type_id
    for x in range(W):
        grid.set_cell(x, H - 1, wall)
    for y in range(40, 60):           # 柱体在 y=64 缝上方，x=96 处
        grid.set_cell(96, y, sand)
    n0 = count_by_type(grid).get(sand, 0)
    for _ in range(120):
        grid.update()
    counts = count_by_type(grid)
    assert counts.get(sand, 0) == n0   # 跨缝不丢沙
    settled = sum(
        1
        for x in range(W)
        for y in range(100, H - 1)
        if grid.get_type_id(x, y) == sand
    )
    assert settled == n0               # 全部穿过缝落到底部区


def test_water_flows_across_vertical_seam():
    reg, table = real_env()
    grid = CellGrid(W, H, reg, table)
    wall = reg.get_by_name("wall").type_id
    water = reg.get_by_name("water").type_id
    for x in range(W):
        grid.set_cell(x, H - 1, wall)
    for x in range(56, 64):           # 水贴 x=64 垂直缝左侧
        for y in range(115, 126):
            grid.set_cell(x, y, water)
    for _ in range(160):
        grid.update()
    right = sum(
        1
        for x in range(64, W)
        for y in range(100, H - 1)
        if grid.get_type_id(x, y) == water
    )
    assert right > 0                   # 水穿过垂直缝向右摊开


def test_same_seed_and_pollution_at_multichunk():
    a_grid = build_mixed(seed=11)
    b_grid = build_mixed(seed=11)
    a, b = [], []
    for f in range(40):
        a_grid.update()
        a.append(a_grid.state_hash())
        random.seed(f * 31 + 5)        # 污染全局 random 流
        random.random()
        b_grid.update()
        b.append(b_grid.state_hash())
    assert a == b


def test_reaction_product_does_not_act_same_frame(tmp_path):
    """决策② 红绿验证：产物盖戳 → 本帧不动，下帧才动。"""
    f = tmp_path / "m.toml"
    f.write_text(
        """
[meta]
version = 1

[materials.wall]
cell_type = "solid"
density = 100
color = [128, 128, 128]
tags = ["solid"]

[materials.lava]
cell_type = "liquid"
density = 30
color = [255, 96, 0]
tags = ["liquid"]

[materials.water]
cell_type = "liquid"
density = 10
color = [48, 96, 255]
tags = ["liquid"]

[materials.rock]
cell_type = "solid"
density = 90
color = [100, 100, 100]
tags = ["solid"]

[materials.steam]
cell_type = "gas"
density = 1
color = [200, 200, 255]
tags = ["gas"]

[[reactions]]
input = ["lava", "water"]
output = ["rock", "steam"]
probability = 1.0
"""
    )
    reg = MaterialRegistry(str(f))
    table = ReactionTable(str(f), reg)
    grid = CellGrid(8, 8, reg, table)
    wall = reg.get_by_name("wall").type_id
    lava = reg.get_by_name("lava").type_id
    water = reg.get_by_name("water").type_id
    steam = reg.get_by_name("steam").type_id

    # lava(3,4) 四面封死；water(3,3) 在其上方。自底向上扫描先到 y=4 行：
    # lava 触发反应（p=1.0）→ rock@(3,4) + steam@(3,3)。
    # (3,3) 行尚未扫描；(3,2) 是空气——若产物不盖戳，steam 将在同帧上浮。
    for wx, wy in ((2, 4), (4, 4), (2, 5), (3, 5), (4, 5), (2, 3), (4, 3)):
        grid.set_cell(wx, wy, wall)
    grid.set_cell(3, 4, lava)
    grid.set_cell(3, 3, water)

    grid.update()
    assert grid.get_type_id(3, 3) == steam      # 产物本帧没动（盖戳生效）
    grid.update()
    assert grid.get_type_id(3, 2) == steam      # 下帧才上浮


def test_write_rect_clamps_movement():
    """域契约逻辑直测：±1 物理触不到域边，手工缩小写域验证拒绝。"""
    from core.rules import _can_move_to

    reg, table = real_env()
    grid = CellGrid(64, 64, reg, table)
    grid._write_rect = Rect(0, 0, 10, 10)
    sand_density = reg.get_by_name("sand").density
    assert _can_move_to(grid, 5, 5, sand_density, heavier_sinks=True)        # 域内空格
    assert not _can_move_to(grid, 10, 5, sand_density, heavier_sinks=True)   # 域外（仍在世界内）
    assert not _can_move_to(grid, 5, 10, sand_density, heavier_sinks=True)
