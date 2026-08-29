#!/usr/bin/env python3
from __future__ import annotations

import argparse
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
    ap = argparse.ArgumentParser()
    ap.add_argument("--seed", type=int, default=0, help="world seed（确定性契约 D2）")
    ap.add_argument("--record", default=None, help="录制 demo 到 JSONL（D7）")
    args = ap.parse_args()

    pygame.init()

    registry = MaterialRegistry(TOML_PATH)
    reaction_table = ReactionTable(TOML_PATH, registry)
    grid = CellGrid(GRID_WIDTH, GRID_HEIGHT, registry, reaction_table, seed=args.seed)
    renderer = PygameRenderer(grid, registry, scale=SCALE)
    recorder = None
    if args.record:
        from replay import Recorder

        recorder = Recorder(args.record, TOML_PATH, GRID_WIDTH, GRID_HEIGHT, args.seed)
    input_handler = InputHandler(registry, scale=SCALE, recorder=recorder)

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

    if recorder is not None:
        recorder.close()
    pygame.quit()
    sys.exit(0)


if __name__ == "__main__":
    main()
