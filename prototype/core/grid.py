from __future__ import annotations

import random  # TODO(M0 Task 4): _check_reactions 换 keyed RNG 后删除
import zlib
from array import array

from core.cell import AIR, STRIDE, TYPE_ID, VELOCITY, LIFETIME, FLAGS, FLAG_DIRTY
from core.material import MaterialRegistry
from core.reaction import ReactionTable, SELF_MARKER
from core.rng import SALT_REACTION, frame_seed, rng_u32


class CellGrid:
    def __init__(
        self,
        width: int,
        height: int,
        registry: MaterialRegistry,
        reaction_table: ReactionTable,
        seed: int = 0,
    ) -> None:
        self.width = width
        self.height = height
        self.registry = registry
        self.reaction_table = reaction_table
        self.frame_count = 0
        self.seed = seed
        self._fseed = frame_seed(seed, 0)
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

    def state_hash(self) -> int:
        """世界状态 CRC32（确定性契约 D5）。同机确定；跨平台字节序口径 C# 期再钉。"""
        return zlib.crc32(array("i", self.cells).tobytes())

    def update(self) -> None:
        from core.rules import try_move  # lazy import to avoid circular dependency

        # 0. Per-frame RNG seed（确定性契约 D2）
        self._fseed = frame_seed(self.seed, self.frame_count)

        # 1. Clear dirty flags
        for i in range(self.width * self.height):
            self.cells[i * STRIDE + FLAGS] &= ~FLAG_DIRTY

        # 2. Bottom-up traversal
        left_to_right = self.frame_count % 2 == 0
        for y in range(self.height - 1, -1, -1):
            x_range = range(self.width) if left_to_right else range(self.width - 1, -1, -1)
            for x in x_range:
                base = self._base(x, y)
                type_id = self.cells[base + TYPE_ID]
                if type_id == AIR:
                    continue
                if self.cells[base + FLAGS] & FLAG_DIRTY:
                    continue

                target = try_move(self, x, y)
                if target is not None:
                    tx, ty = target
                    self.swap(x, y, tx, ty)
                    self.cells[self._base(x, y) + FLAGS] |= FLAG_DIRTY
                    self.cells[self._base(tx, ty) + FLAGS] |= FLAG_DIRTY
                    self._check_reactions(tx, ty)
                else:
                    self._check_reactions(x, y)

        # 3. Lifetime decay
        for i in range(self.width * self.height):
            base = i * STRIDE
            lt = self.cells[base + LIFETIME]
            if lt > 0:
                lt -= 1
                self.cells[base + LIFETIME] = lt
                if lt == 0:
                    self.cells[base + TYPE_ID] = AIR
                    self.cells[base + VELOCITY] = 0
                    self.cells[base + FLAGS] = 0

        # 4. Advance frame
        self.frame_count += 1

    def _check_reactions(self, x: int, y: int) -> None:
        type_a = self.get_type_id(x, y)
        if type_a == AIR:
            return
        neighbors = [(x, y - 1), (x, y + 1), (x - 1, y), (x + 1, y)]
        for nx, ny in neighbors:
            if not self.in_bounds(nx, ny):
                continue
            type_b = self.get_type_id(nx, ny)
            if type_b == AIR:
                continue
            results = self.reaction_table.get(type_a, type_b)
            if results is None:
                continue
            for result in results:
                if random.random() < result.probability:
                    out1 = type_a if result.output1 == SELF_MARKER else result.output1
                    out2 = type_b if result.output2 == SELF_MARKER else result.output2
                    self.set_cell(x, y, out1)
                    self.set_cell(nx, ny, out2)
                    return
