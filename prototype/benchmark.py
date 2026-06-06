#!/usr/bin/env python3
"""性能基准（CLAUDE.md §5.3）。输出：{w}x{h}, {ratio}% active, {fps} FPS

场景：底墙 + 大沙块（下落扰动）+ 水层（横流），约 30% 非空像素。
结果记录到 docs/perf/baseline.md；新增材质/规则后必须重跑对比。
"""
from __future__ import annotations

import time
from pathlib import Path

from core.cell import AIR, STRIDE, TYPE_ID
from core.grid import CellGrid
from core.material import MaterialRegistry
from core.reaction import ReactionTable

TOML = str(Path(__file__).parent / "data" / "materials.toml")
W, H, FRAMES = 128, 128, 200


def build() -> CellGrid:
    reg = MaterialRegistry(TOML)
    grid = CellGrid(W, H, reg, ReactionTable(TOML, reg), seed=42)
    wall = reg.get_by_name("wall").type_id
    sand = reg.get_by_name("sand").type_id
    water = reg.get_by_name("water").type_id
    for x in range(W):
        grid.set_cell(x, H - 1, wall)
    for x in range(8, 120):
        for y in range(10, 44):
            grid.set_cell(x, y, sand)        # ≈21%
    for x in range(8, 120):
        for y in range(100, 114):
            grid.set_cell(x, y, water)       # ≈10%
    return grid


def main() -> None:
    grid = build()
    non_air = sum(
        1 for i in range(W * H) if grid.cells[i * STRIDE + TYPE_ID] != AIR
    )
    ratio = 100.0 * non_air / (W * H)
    t0 = time.perf_counter()
    for _ in range(FRAMES):
        grid.update()
    dt = time.perf_counter() - t0
    fps = FRAMES / dt
    print(f"{W}x{H}, {ratio:.0f}% active, {fps:.1f} FPS  ({dt / FRAMES * 1000:.1f} ms/frame)")


if __name__ == "__main__":
    main()
