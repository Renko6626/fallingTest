from __future__ import annotations

from typing import Optional

from core.cell import AIR, VELOCITY
from core.grid import CellGrid
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
        return _move_liquid(grid, x, y, mat.density)
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


def _move_powder(grid: CellGrid, x: int, y: int, density: int) -> Optional[tuple[int, int]]:
    if _can_move_to(grid, x, y + 1, density, heavier_sinks=True):
        return (x, y + 1)

    diags = ((x - 1, y + 1), (x + 1, y + 1))
    for i in order2(grid._fseed, grid._pass_id, x, y, SALT_DIAG):
        dx, dy = diags[i]
        if _can_move_to(grid, dx, dy, density, heavier_sinks=True):
            return (dx, dy)

    return None


def _move_liquid(grid: CellGrid, x: int, y: int, density: int) -> Optional[tuple[int, int]]:
    if _can_move_to(grid, x, y + 1, density, heavier_sinks=True):
        return (x, y + 1)

    diags = ((x - 1, y + 1), (x + 1, y + 1))
    for i in order2(grid._fseed, grid._pass_id, x, y, SALT_DIAG):
        dx, dy = diags[i]
        if _can_move_to(grid, dx, dy, density, heavier_sinks=True):
            return (dx, dy)

    base = grid._base(x, y)
    vel = grid.cells[base + VELOCITY]
    sides = [(x + vel, y), (x - vel, y)]
    for sx, sy in sides:
        if _can_move_to(grid, sx, sy, density, heavier_sinks=True):
            return (sx, sy)

    grid.cells[base + VELOCITY] = -vel
    return None


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
    sides = [(x + vel, y), (x - vel, y)]
    for sx, sy in sides:
        if _can_move_to(grid, sx, sy, density, heavier_sinks=False):
            return (sx, sy)

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
