#!/usr/bin/env python3
"""Generate a GIF demo: density interactions — sand sinks through water,
oil floats up through water, lava sinks below oil, steam rises."""
from __future__ import annotations

import os
from pathlib import Path

from PIL import Image

os.environ["SDL_VIDEODRIVER"] = "dummy"
os.environ["SDL_AUDIODRIVER"] = "dummy"

from core.material import MaterialRegistry
from core.reaction import ReactionTable
from core.grid import CellGrid
from render.pygame_renderer import build_color_lut, render_to_array, build_variance_matrix

TOML_PATH = str(Path(__file__).parent / "data" / "materials.toml")
WIDTH, HEIGHT = 128, 128
SCALE = 4
TOTAL_FRAMES = 400
CAPTURE_EVERY = 2


def setup_scene(grid: CellGrid, reg: MaterialRegistry) -> None:
    wall_id = reg.get_by_name("wall").type_id
    water_id = reg.get_by_name("water").type_id
    sand_id = reg.get_by_name("sand").type_id
    oil_id = reg.get_by_name("oil").type_id

    # Container: floor + side walls
    for x in range(WIDTH):
        grid.set_cell(x, HEIGHT - 1, wall_id)
    for y in range(10, HEIGHT):
        grid.set_cell(0, y, wall_id)
        grid.set_cell(WIDTH - 1, y, wall_id)

    # Water pool filling the lower half (density 10)
    for y in range(HEIGHT - 50, HEIGHT - 1):
        for x in range(1, WIDTH - 1):
            grid.set_cell(x, y, water_id)

    # Oil layer at the very bottom — should float up through the water (density 8 < 10)
    for y in range(HEIGHT - 8, HEIGHT - 1):
        for x in range(1, WIDTH - 1):
            grid.set_cell(x, y, oil_id)

    # Sand block falling from above — should sink through water (density 60)
    for y in range(5, 25):
        for x in range(45, 85):
            grid.set_cell(x, y, sand_id)


def main() -> None:
    reg = MaterialRegistry(TOML_PATH)
    table = ReactionTable(TOML_PATH, reg)
    grid = CellGrid(WIDTH, HEIGHT, reg, table, seed=42)
    color_lut = build_color_lut(reg)
    variance = build_variance_matrix(WIDTH, HEIGHT, reg, seed=42)

    setup_scene(grid, reg)

    frames: list[Image.Image] = []

    for frame_idx in range(TOTAL_FRAMES):
        if frame_idx % CAPTURE_EVERY == 0:
            buf = render_to_array(grid, color_lut, variance)
            img_array = buf.transpose(1, 0, 2)
            img = Image.fromarray(img_array, "RGB")
            img = img.resize((WIDTH * SCALE, HEIGHT * SCALE), Image.NEAREST)
            frames.append(img)

        grid.update()

        if frame_idx % 100 == 0:
            print(f"Frame {frame_idx}/{TOTAL_FRAMES}")

    out_path = str(Path(__file__).parent.parent / "demo_density.gif")
    frames[0].save(
        out_path,
        save_all=True,
        append_images=frames[1:],
        duration=33,
        loop=0,
    )
    print(f"Saved {len(frames)} frames to {out_path}")


if __name__ == "__main__":
    main()
