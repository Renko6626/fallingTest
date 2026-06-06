from __future__ import annotations

from core.cell import AIR
from core.grid import CellGrid
from core.material import MaterialRegistry
from core.ops import apply_brush


MATERIAL_KEYS = [
    ("wall", 49),       # Key 1
    ("wood", 50),       # Key 2
    ("sand", 51),       # Key 3
    ("water", 52),      # Key 4
    ("oil", 53),        # Key 5
    ("lava", 54),       # Key 6
    ("fire", 55),       # Key 7
    ("steam", 56),      # Key 8
]


class InputHandler:
    def __init__(
        self, registry: MaterialRegistry, scale: int = 4, recorder=None
    ) -> None:
        self.registry = registry
        self.scale = scale
        self.selected_material = "sand"
        self.brush_size = 3
        self.paused = False
        self.recorder = recorder  # replay.Recorder | None（D7 录制钩子）

    def handle_events(self, grid: CellGrid) -> bool:
        import pygame

        for event in pygame.event.get():
            if event.type == pygame.QUIT:
                return False

            if event.type == pygame.KEYDOWN:
                if event.key == pygame.K_SPACE:
                    self.paused = not self.paused
                elif event.key == pygame.K_r:
                    for y in range(grid.height):
                        for x in range(grid.width):
                            grid.set_cell(x, y, AIR)

                for mat_name, key_code in MATERIAL_KEYS:
                    if event.key == key_code:
                        try:
                            self.registry.get_by_name(mat_name)
                            self.selected_material = mat_name
                        except KeyError:
                            pass

            if event.type == pygame.MOUSEWHEEL:
                self.brush_size = max(1, min(10, self.brush_size + event.y))

        mouse_buttons = pygame.mouse.get_pressed()
        if mouse_buttons[0] or mouse_buttons[2]:
            mx, my = pygame.mouse.get_pos()
            gx = mx // self.scale
            gy = my // self.scale

            if mouse_buttons[0]:
                type_id = self.registry.get_by_name(self.selected_material).type_id
            else:
                type_id = AIR

            apply_brush(grid, gx, gy, type_id, self.brush_size)
            if self.recorder is not None:
                self.recorder.log_paint(grid.frame_count, gx, gy, type_id, self.brush_size)

        return True
