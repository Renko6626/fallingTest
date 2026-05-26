#!/usr/bin/env python3
from __future__ import annotations

import sys
from pathlib import Path

import pygame

from core.material import MaterialRegistry
from core.reaction import ReactionTable
from core.grid import CellGrid
from render.pygame_renderer import PygameRenderer
from render.input_handler import InputHandler

TOML_PATH = str(Path(__file__).parent / "data" / "materials.toml")
GRID_WIDTH = 128
GRID_HEIGHT = 128
SCALE = 4
TARGET_FPS = 60


def main() -> None:
    pygame.init()

    registry = MaterialRegistry(TOML_PATH)
    reaction_table = ReactionTable(TOML_PATH, registry)
    grid = CellGrid(GRID_WIDTH, GRID_HEIGHT, registry, reaction_table)
    renderer = PygameRenderer(grid, registry, scale=SCALE)
    input_handler = InputHandler(registry, scale=SCALE)

    clock = pygame.time.Clock()

    running = True
    while running:
        running = input_handler.handle_events(grid)

        if not input_handler.paused:
            grid.update()

        renderer.render()
        renderer.draw_ui(input_handler.selected_material, clock.get_fps())
        pygame.display.flip()
        clock.tick(TARGET_FPS)

    pygame.quit()
    sys.exit(0)


if __name__ == "__main__":
    main()
