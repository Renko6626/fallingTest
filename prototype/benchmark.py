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
FRAMES = 200


def build(w: int, h: int) -> CellGrid:
    """场景按比例缩放：底墙 + 大沙块（≈21%）+ 水层（≈10%）。"""
    reg = MaterialRegistry(TOML)
    grid = CellGrid(w, h, reg, ReactionTable(TOML, reg), seed=42)
    wall = reg.get_by_name("wall").type_id
    sand = reg.get_by_name("sand").type_id
    water = reg.get_by_name("water").type_id
    for x in range(w):
        grid.set_cell(x, h - 1, wall)
    # 以 128 基准场景的精确锚点等比缩放（128×128 时与 M0 基线逐格一致）
    for x in range(8 * w // 128, 120 * w // 128):
        for y in range(10 * h // 128, 44 * h // 128):
            grid.set_cell(x, y, sand)
    for x in range(8 * w // 128, 120 * w // 128):
        for y in range(100 * h // 128, 114 * h // 128):
            grid.set_cell(x, y, water)
    return grid


def bench(w: int, h: int) -> None:
    grid = build(w, h)
    non_air = sum(
        1 for i in range(w * h) if grid.cells[i * STRIDE + TYPE_ID] != AIR
    )
    ratio = 100.0 * non_air / (w * h)
    t0 = time.perf_counter()
    for _ in range(FRAMES):
        grid.update()
    dt = time.perf_counter() - t0
    fps = FRAMES / dt
    print(f"{w}x{h}, {ratio:.0f}% active, {fps:.1f} FPS  ({dt / FRAMES * 1000:.1f} ms/frame)")


def main() -> None:
    bench(128, 128)   # 正式基准（与 M0 基线可比）
    bench(192, 192)   # 多 chunk 数据点（3×3，调度真实开销）


if __name__ == "__main__":
    main()
