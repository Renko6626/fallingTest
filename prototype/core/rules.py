from __future__ import annotations

import random
from typing import Optional

from core.cell import AIR, TYPE_ID, VELOCITY, STRIDE
from core.grid import CellGrid


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


def _can_move_to(grid: CellGrid, x: int, y: int, self_density: float, heavier_sinks: bool) -> bool:
    if not grid.in_bounds(x, y):
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


def _move_powder(grid: CellGrid, x: int, y: int, density: float) -> Optional[tuple[int, int]]:
    if _can_move_to(grid, x, y + 1, density, heavier_sinks=True):
        return (x, y + 1)

    diags = [(x - 1, y + 1), (x + 1, y + 1)]
    random.shuffle(diags)
    for dx, dy in diags:
        if _can_move_to(grid, dx, dy, density, heavier_sinks=True):
            return (dx, dy)

    return None


def _move_liquid(grid: CellGrid, x: int, y: int, density: float) -> Optional[tuple[int, int]]:
    if _can_move_to(grid, x, y + 1, density, heavier_sinks=True):
        return (x, y + 1)

    diags = [(x - 1, y + 1), (x + 1, y + 1)]
    random.shuffle(diags)
    for dx, dy in diags:
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


def _move_gas(grid: CellGrid, x: int, y: int, density: float) -> Optional[tuple[int, int]]:
    if _can_move_to(grid, x, y - 1, density, heavier_sinks=False):
        return (x, y - 1)

    diags = [(x - 1, y - 1), (x + 1, y - 1)]
    random.shuffle(diags)
    for dx, dy in diags:
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
    if random.random() < 0.4:
        return None
    candidates = [(x, y - 1), (x - 1, y - 1), (x + 1, y - 1)]
    random.shuffle(candidates)
    for cx, cy in candidates:
        if grid.in_bounds(cx, cy) and grid.get_type_id(cx, cy) == AIR:
            return (cx, cy)
    return None
