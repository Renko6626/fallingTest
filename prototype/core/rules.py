from __future__ import annotations

from typing import Optional

from core.cell import AIR, VELOCITY
from core.grid import CellGrid
from core.material import MaterialDef
from core.rng import (
    SALT_DIAG,
    SALT_ENERGY_DIR,
    SALT_ENERGY_LINGER,
    order2,
    perm3,
    rng_chance,
    threshold_u32,
)

ENERGY_LINGER_THRESHOLD = threshold_u32(0.4)


def try_move(grid: CellGrid, x: int, y: int) -> Optional[tuple[int, int]]:
    type_id = grid.get_type_id(x, y)
    if type_id == AIR:
        return None

    mat = grid.registry.get_by_id(type_id)
    cell_type = mat.cell_type

    if cell_type == "solid":
        return None
    elif cell_type == "powder":
        return _move_powder(grid, x, y, mat.density)
    elif cell_type == "liquid":
        return _move_liquid(grid, x, y, mat)
    elif cell_type == "gas":
        return _move_gas(grid, x, y, mat.density)
    elif cell_type == "energy":
        return _move_energy(grid, x, y)
    return None


def _can_move_to(grid: CellGrid, x: int, y: int, self_density: int, heavier_sinks: bool) -> bool:
    # 写域契约（提案 §2.2 条件①）：目标必须在当前活跃写域内。
    # 写域已裁剪到世界边界，蕴含 in_bounds；±1 移动距域边 ≥30px，
    # 本检查在速度积分（一帧多格）落地前不会实际触发——预埋契约。
    if not grid._write_rect.contains(x, y):
        return False
    target_id = grid.get_type_id(x, y)
    if target_id == AIR:
        return True
    target_mat = grid.registry.get_by_id(target_id)
    if target_mat.cell_type == "solid":
        return False
    if heavier_sinks:
        return target_mat.density < self_density
    else:
        return target_mat.density > self_density


def _probe_side(grid: CellGrid, x: int, y: int, density: int, dispersion: int, heavier_sinks: bool) -> Optional[tuple[int, int]]:
    """沿方向记忆探测最多 dispersion 格，落最远连续 AIR；
    首格可密度置换则走旧 ±1 路径（spec §2，2026-06-07）。
    纯确定（无 RNG）；在 write_rect 边界截断（写域契约）。"""
    base = grid._base(x, y)
    vel = grid.cells[base + VELOCITY]
    assert vel in (1, -1), f"方向记忆必须为 ±1，实际 {vel}（契约：set_cell 初始化为 1）"
    for direction in (vel, -vel):
        furthest = None
        for i in range(1, dispersion + 1):
            tx = x + direction * i
            if not grid._write_rect.contains(tx, y):
                break
            target_id = grid.get_type_id(tx, y)
            if target_id == AIR:
                furthest = (tx, y)
                continue
            if i == 1 and _can_move_to(grid, tx, y, density, heavier_sinks=heavier_sinks):
                return (tx, y)  # 密度置换（仅 i==1）：旧 ±1 兜底，落点非 AIR
            break
        if furthest is not None:
            if direction == -vel:
                grid.cells[base + VELOCITY] = -vel  # 方向承诺（2026-06-07 修复同款）
            return furthest
    grid.cells[base + VELOCITY] = -vel  # 两侧全堵也翻转：下帧换方向探，防冻结
    return None


def _move_powder(grid: CellGrid, x: int, y: int, density: int) -> Optional[tuple[int, int]]:
    if _can_move_to(grid, x, y + 1, density, heavier_sinks=True):
        return (x, y + 1)

    diags = ((x - 1, y + 1), (x + 1, y + 1))
    for i in order2(grid._fseed, grid._pass_id, x, y, SALT_DIAG):
        dx, dy = diags[i]
        if _can_move_to(grid, dx, dy, density, heavier_sinks=True):
            return (dx, dy)

    return None


def _move_liquid(grid: CellGrid, x: int, y: int, mat: MaterialDef) -> Optional[tuple[int, int]]:
    density = mat.density
    if _can_move_to(grid, x, y + 1, density, heavier_sinks=True):
        return (x, y + 1)

    diags = ((x - 1, y + 1), (x + 1, y + 1))
    for i in order2(grid._fseed, grid._pass_id, x, y, SALT_DIAG):
        dx, dy = diags[i]
        if _can_move_to(grid, dx, dy, density, heavier_sinks=True):
            return (dx, dy)

    return _probe_side(grid, x, y, density, mat.dispersion, heavier_sinks=True)


def _move_gas(grid: CellGrid, x: int, y: int, density: int) -> Optional[tuple[int, int]]:
    if _can_move_to(grid, x, y - 1, density, heavier_sinks=False):
        return (x, y - 1)

    diags = ((x - 1, y - 1), (x + 1, y - 1))
    for i in order2(grid._fseed, grid._pass_id, x, y, SALT_DIAG):
        dx, dy = diags[i]
        if _can_move_to(grid, dx, dy, density, heavier_sinks=False):
            return (dx, dy)

    base = grid._base(x, y)
    vel = grid.cells[base + VELOCITY]
    if _can_move_to(grid, x + vel, y, density, heavier_sinks=False):
        return (x + vel, y)
    if _can_move_to(grid, x - vel, y, density, heavier_sinks=False):
        grid.cells[base + VELOCITY] = -vel  # 方向承诺（与液体同款修复）
        return (x - vel, y)

    grid.cells[base + VELOCITY] = -vel
    return None


def _move_energy(grid: CellGrid, x: int, y: int) -> Optional[tuple[int, int]]:
    # 40% chance to stay in place — lets fire linger near fuel and spread
    if rng_chance(grid._fseed, grid._pass_id, x, y, SALT_ENERGY_LINGER, ENERGY_LINGER_THRESHOLD):
        return None
    candidates = ((x, y - 1), (x - 1, y - 1), (x + 1, y - 1))
    for i in perm3(grid._fseed, grid._pass_id, x, y, SALT_ENERGY_DIR):
        cx, cy = candidates[i]
        if grid._write_rect.contains(cx, cy) and grid.get_type_id(cx, cy) == AIR:
            return (cx, cy)
    return None
