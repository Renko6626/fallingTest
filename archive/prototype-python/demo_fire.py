#!/usr/bin/env python3
"""Generate a GIF demo: fire ignites a wood structure, flames spread."""
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
TOTAL_FRAMES = 600
CAPTURE_EVERY = 2


def setup_scene(grid: CellGrid, reg: MaterialRegistry) -> None:
    wall_id = reg.get_by_name("wall").type_id
    wood_id = reg.get_by_name("wood").type_id
    oil_id = reg.get_by_name("oil").type_id
    fire_id = reg.get_by_name("fire").type_id

    # Floor
    for x in range(WIDTH):
        grid.set_cell(x, HEIGHT - 1, wall_id)

    # Walls on sides
    for y in range(20, HEIGHT):
        grid.set_cell(0, y, wall_id)
        grid.set_cell(WIDTH - 1, y, wall_id)

    # === Wood house structure ===

    # Foundation platform
    for x in range(20, 108):
        grid.set_cell(x, HEIGHT - 2, wood_id)

    # Left pillar
    for y in range(HEIGHT - 30, HEIGHT - 2):
        for x in range(22, 26):
            grid.set_cell(x, y, wood_id)

    # Right pillar
    for y in range(HEIGHT - 30, HEIGHT - 2):
        for x in range(100, 104):
            grid.set_cell(x, y, wood_id)

    # Center pillar
    for y in range(HEIGHT - 25, HEIGHT - 2):
        for x in range(60, 64):
            grid.set_cell(x, y, wood_id)

    # Roof beam
    for x in range(20, 108):
        grid.set_cell(x, HEIGHT - 30, wood_id)
        grid.set_cell(x, HEIGHT - 31, wood_id)

    # Second floor
    for x in range(25, 103):
        grid.set_cell(x, HEIGHT - 15, wood_id)

    # Shelves on the left
    for x in range(28, 55):
        grid.set_cell(x, HEIGHT - 10, wood_id)

    # Shelves on the right
    for x in range(70, 98):
        grid.set_cell(x, HEIGHT - 10, wood_id)

    # Triangular roof
    roof_base_y = HEIGHT - 31
    for layer in range(15):
        left = 20 + layer * 3
        right = 108 - layer * 3
        if left >= right:
            break
        for x in range(left, right):
            grid.set_cell(x, roof_base_y - layer, wood_id)

    # === Oil barrels (accelerant) ===

    # Oil puddle on second floor left
    for x in range(30, 50):
        for y in range(HEIGHT - 17, HEIGHT - 15):
            grid.set_cell(x, y, oil_id)

    # Oil puddle on second floor right
    for x in range(75, 95):
        for y in range(HEIGHT - 17, HEIGHT - 15):
            grid.set_cell(x, y, oil_id)

    # Oil dripping down from shelf
    for x in range(35, 45):
        for y in range(HEIGHT - 12, HEIGHT - 10):
            grid.set_cell(x, y, oil_id)

    # === Ignition: small fire at the bottom left ===
    for x in range(28, 34):
        for y in range(HEIGHT - 4, HEIGHT - 2):
            grid.set_cell(x, y, fire_id)


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
            img_array = buf.transpose(1, 0, 2)
            img = Image.fromarray(img_array, "RGB")
            img = img.resize((WIDTH * SCALE, HEIGHT * SCALE), Image.NEAREST)
            frames.append(img)

        grid.update()

        if frame_idx % 100 == 0:
            print(f"Frame {frame_idx}/{TOTAL_FRAMES}")

    out_path = str(Path(__file__).parent.parent / "demo_fire.gif")
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
