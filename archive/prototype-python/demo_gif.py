#!/usr/bin/env python3
"""Generate a GIF demo: water + lava falling into a container."""
from __future__ import annotations

import os
import random
from pathlib import Path

import numpy as np
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
TOTAL_FRAMES = 300
CAPTURE_EVERY = 2


def setup_scene(grid: CellGrid, reg: MaterialRegistry) -> None:
    wall_id = reg.get_by_name("wall").type_id
    water_id = reg.get_by_name("water").type_id
    lava_id = reg.get_by_name("lava").type_id
    sand_id = reg.get_by_name("sand").type_id
    wood_id = reg.get_by_name("wood").type_id
    oil_id = reg.get_by_name("oil").type_id

    # Floor
    for x in range(WIDTH):
        grid.set_cell(x, HEIGHT - 1, wall_id)

    # Left wall
    for y in range(HEIGHT // 3, HEIGHT):
        grid.set_cell(0, y, wall_id)

    # Right wall
    for y in range(HEIGHT // 3, HEIGHT):
        grid.set_cell(WIDTH - 1, y, wall_id)

    # Center divider (partial, with gap at bottom)
    for y in range(HEIGHT // 3, HEIGHT - 15):
        grid.set_cell(WIDTH // 2, y, wall_id)

    # Wood platform on the left
    for x in range(15, 45):
        grid.set_cell(x, HEIGHT - 20, wood_id)

    # Oil pool on the wood platform
    for x in range(18, 42):
        for y in range(HEIGHT - 22, HEIGHT - 20):
            grid.set_cell(x, y, oil_id)

    # Water blob (left side, falling)
    for y in range(5, 20):
        for x in range(15, 40):
            grid.set_cell(x, y, water_id)

    # Lava blob (right side, falling)
    for y in range(5, 18):
        for x in range(75, 100):
            grid.set_cell(x, y, lava_id)

    # Sand pile (right side, above lava)
    for y in range(0, 8):
        for x in range(85, 110):
            grid.set_cell(x, y, sand_id)


def main() -> None:
    random.seed(42)
    np.random.seed(42)

    reg = MaterialRegistry(TOML_PATH)
    table = ReactionTable(TOML_PATH, reg)
    grid = CellGrid(WIDTH, HEIGHT, reg, table)
    color_lut = build_color_lut(reg)
    variance = build_variance_matrix(WIDTH, HEIGHT, reg, seed=42)

    setup_scene(grid, reg)

    frames: list[Image.Image] = []

    for frame_idx in range(TOTAL_FRAMES):
        if frame_idx % CAPTURE_EVERY == 0:
            buf = render_to_array(grid, color_lut, variance)
            # buf is (width, height, 3) — transpose to (height, width, 3) for PIL
            img_array = buf.transpose(1, 0, 2)
            img = Image.fromarray(img_array, "RGB")
            img = img.resize((WIDTH * SCALE, HEIGHT * SCALE), Image.NEAREST)
            frames.append(img)

        grid.update()

        if frame_idx % 50 == 0:
            print(f"Frame {frame_idx}/{TOTAL_FRAMES}")

    out_path = str(Path(__file__).parent.parent / "demo.gif")
    frames[0].save(
        out_path,
        save_all=True,
        append_images=frames[1:],
        duration=33,  # ~30 fps
        loop=0,
    )
    print(f"Saved {len(frames)} frames to {out_path}")


if __name__ == "__main__":
    main()
