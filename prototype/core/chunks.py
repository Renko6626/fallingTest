"""Chunk 划分与 4-pass 棋盘调度的纯几何（确定性提案 §2.2 条件①③）。

- 模拟 chunk = 64×64（Noita 同款，已核验）。
- 写域 = 正方形 [chunk−32, chunk+96)²（评审 B2：十字域在所有权语义下角落死锁，
  正方形同 pass 两两不相交且恰好密铺）。
- pass 序固定 (0,0)→(1,0)→(0,1)→(1,1)；pass 内行优先——交换律保证顺序无关，仍钉死（D3）。
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Iterator

CHUNK = 64
MARGIN = 32
PASS_PARITY = ((0, 0), (1, 0), (0, 1), (1, 1))


@dataclass(frozen=True)
class Rect:
    """半开区间 [x0, x1) × [y0, y1)。"""

    x0: int
    y0: int
    x1: int
    y1: int

    def contains(self, x: int, y: int) -> bool:
        return self.x0 <= x < self.x1 and self.y0 <= y < self.y1


class ChunkLayout:
    def __init__(self, width: int, height: int) -> None:
        self.width = width
        self.height = height
        self.cw = (width + CHUNK - 1) // CHUNK
        self.ch = (height + CHUNK - 1) // CHUNK

    def chunk_rect(self, cx: int, cy: int) -> Rect:
        return Rect(
            cx * CHUNK,
            cy * CHUNK,
            min((cx + 1) * CHUNK, self.width),
            min((cy + 1) * CHUNK, self.height),
        )

    def write_rect(self, cx: int, cy: int) -> Rect:
        """正方形写域 [chunk−32, chunk+96)²，裁剪到世界边界。"""
        return Rect(
            max(cx * CHUNK - MARGIN, 0),
            max(cy * CHUNK - MARGIN, 0),
            min((cx + 1) * CHUNK + MARGIN, self.width),
            min((cy + 1) * CHUNK + MARGIN, self.height),
        )

    def chunks_for_pass(self, pass_id: int) -> Iterator[tuple[int, int]]:
        px, py = PASS_PARITY[pass_id]
        for cy in range(py, self.ch, 2):
            for cx in range(px, self.cw, 2):
                yield (cx, cy)
