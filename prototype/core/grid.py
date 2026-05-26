from __future__ import annotations

from core.cell import AIR, STRIDE, TYPE_ID, VELOCITY, LIFETIME, FLAGS, FLAG_DIRTY
from core.material import MaterialRegistry
from core.reaction import ReactionTable


class CellGrid:
    def __init__(
        self,
        width: int,
        height: int,
        registry: MaterialRegistry,
        reaction_table: ReactionTable,
    ) -> None:
        self.width = width
        self.height = height
        self.registry = registry
        self.reaction_table = reaction_table
        self.frame_count = 0
        self.cells: list[int] = [0] * (width * height * STRIDE)

    def _base(self, x: int, y: int) -> int:
        return (y * self.width + x) * STRIDE

    def in_bounds(self, x: int, y: int) -> bool:
        return 0 <= x < self.width and 0 <= y < self.height

    def get_type_id(self, x: int, y: int) -> int:
        return self.cells[self._base(x, y) + TYPE_ID]

    def set_cell(self, x: int, y: int, type_id: int) -> None:
        base = self._base(x, y)
        mat = self.registry.get_by_id(type_id)
        self.cells[base + TYPE_ID] = type_id
        self.cells[base + VELOCITY] = 1
        self.cells[base + LIFETIME] = mat.lifetime
        self.cells[base + FLAGS] = 0

    def swap(self, x1: int, y1: int, x2: int, y2: int) -> None:
        b1 = self._base(x1, y1)
        b2 = self._base(x2, y2)
        for offset in range(STRIDE):
            self.cells[b1 + offset], self.cells[b2 + offset] = (
                self.cells[b2 + offset],
                self.cells[b1 + offset],
            )

    def get_type_id_array(self) -> list[int]:
        return [self.cells[i * STRIDE + TYPE_ID] for i in range(self.width * self.height)]

    def update(self) -> None:
        pass  # Task 6 will implement this
