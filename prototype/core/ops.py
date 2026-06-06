"""世界写入操作——InputHandler 与 demo 回放共用，保证两路径逐位一致（D7）。"""
from __future__ import annotations

from core.grid import CellGrid


def apply_brush(
    grid: CellGrid, gx: int, gy: int, type_id: int, brush_size: int
) -> list[tuple[int, int]]:
    """矩形笔刷写格子（与原 InputHandler 行为逐位一致），返回实际写入坐标。"""
    r = brush_size // 2
    written: list[tuple[int, int]] = []
    for dy in range(-r, r + 1):
        for dx in range(-r, r + 1):
            px, py = gx + dx, gy + dy
            if grid.in_bounds(px, py):
                grid.set_cell(px, py, type_id)
                written.append((px, py))
    return written
