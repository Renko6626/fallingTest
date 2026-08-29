from __future__ import annotations

import numpy as np

from core.cell import AIR
from core.grid import CellGrid
from core.material import MaterialRegistry


def build_color_lut(registry: MaterialRegistry) -> np.ndarray:
    lut = np.zeros((registry.max_id + 1, 3), dtype=np.uint8)
    for mat in registry.all():
        lut[mat.type_id] = mat.color
    return lut


def build_variance_matrix(
    width: int, height: int, registry: MaterialRegistry, seed: int = 0
) -> np.ndarray:
    rng = np.random.RandomState(seed)
    max_var = max((m.color_variance for m in registry.all()), default=0)
    if max_var == 0:
        return np.zeros((width, height, 3), dtype=np.int16)
    return rng.randint(-max_var, max_var + 1, (width, height, 3)).astype(np.int16)


def render_to_array(
    grid: CellGrid,
    color_lut: np.ndarray,
    variance_matrix: np.ndarray | None,
) -> np.ndarray:
    # get_type_id_array is row-major (y * width + x), so reshape as (height, width)
    # then transpose to (width, height) so that buf[x, y] indexing is correct
    type_ids = np.array(grid.get_type_id_array(), dtype=np.int32).reshape(
        grid.height, grid.width
    ).T
    buf = color_lut[type_ids].astype(np.int16)
    if variance_matrix is not None:
        air_mask = (type_ids == AIR)[:, :, np.newaxis]
        buf += variance_matrix
        buf = np.where(air_mask, 0, buf)
    np.clip(buf, 0, 255, out=buf)
    return buf.astype(np.uint8)


class PygameRenderer:
    def __init__(
        self, grid: CellGrid, registry: MaterialRegistry, scale: int = 4
    ) -> None:
        import pygame

        self.grid = grid
        self.scale = scale
        self.color_lut = build_color_lut(registry)
        self.variance_matrix = build_variance_matrix(
            grid.width, grid.height, registry
        )

        window_w = grid.width * scale
        window_h = grid.height * scale
        self.screen = pygame.display.set_mode((window_w, window_h))
        pygame.display.set_caption("fallingTest — Pixel Physics Prototype")
        self.surface = pygame.Surface((grid.width, grid.height))
        self.font = pygame.font.SysFont(None, 24)

    def render(self) -> None:
        import pygame

        buf = render_to_array(self.grid, self.color_lut, self.variance_matrix)
        pygame.surfarray.blit_array(self.surface, buf)
        scaled = pygame.transform.scale(
            self.surface,
            (self.grid.width * self.scale, self.grid.height * self.scale),
        )
        self.screen.blit(scaled, (0, 0))

    def draw_ui(self, selected_material: str, fps: float) -> None:
        text = self.font.render(
            f"Material: {selected_material}  FPS: {fps:.0f}", True, (255, 255, 255)
        )
        self.screen.blit(text, (5, 5))
