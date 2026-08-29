from __future__ import annotations

import zlib
from array import array

from core.cell import AIR, STRIDE, TYPE_ID, VELOCITY, LIFETIME, FLAGS, UPDATED_AT
from core.chunks import ChunkLayout, Rect
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
        self.layout = ChunkLayout(width, height)
        self._pass_id = 0
        # 预置全世界写域：update 之外的直接 try_move（测试）也有合法域
        self._write_rect = Rect(0, 0, width, height)
        self.cells: list[int] = [0] * (width * height * STRIDE)

    def _base(self, x: int, y: int) -> int:
        return (y * self.width + x) * STRIDE

    def in_bounds(self, x: int, y: int) -> bool:
        return 0 <= x < self.width and 0 <= y < self.height

    def get_type_id(self, x: int, y: int) -> int:
        return self.cells[self._base(x, y) + TYPE_ID]

    def set_cell(self, x: int, y: int, type_id: int, stamp: bool = False) -> None:
        """写格子。stamp=True：盖当前帧世代戳——update 期间的一切调用方
        （反应产物、未来火系统转化/生成）必须传 True，产物下帧才动（评审 m2）；
        场景搭建/笔刷不盖（M0.5 spec §3，决策②）。"""
        base = self._base(x, y)
        mat = self.registry.get_by_id(type_id)
        self.cells[base + TYPE_ID] = type_id
        self.cells[base + VELOCITY] = 1
        self.cells[base + LIFETIME] = mat.lifetime
        self.cells[base + FLAGS] = 0
        self.cells[base + UPDATED_AT] = self.frame_count if stamp else -1

    def _stamp(self, x: int, y: int) -> None:
        self.cells[self._base(x, y) + UPDATED_AT] = self.frame_count

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
        frame = self.frame_count
        left_to_right = frame % 2 == 0

        # 1. 4-pass 棋盘格 chunk 调度（提案 §2.2 条件①③，单线程锁语义）
        #    所有权制：每 chunk 只扫描自己 64×64 内的像素；
        #    写域 = 正方形 [chunk−32, chunk+96)²；世代戳防同帧二动。
        for pass_id in range(4):
            self._pass_id = pass_id
            for cx, cy in self.layout.chunks_for_pass(pass_id):
                self._write_rect = self.layout.write_rect(cx, cy)
                rect = self.layout.chunk_rect(cx, cy)
                for y in range(rect.y1 - 1, rect.y0 - 1, -1):
                    x_range = (
                        range(rect.x0, rect.x1)
                        if left_to_right
                        else range(rect.x1 - 1, rect.x0 - 1, -1)
                    )
                    for x in x_range:
                        base = self._base(x, y)
                        if self.cells[base + TYPE_ID] == AIR:
                            continue
                        if self.cells[base + UPDATED_AT] == frame:
                            continue  # 本帧已行动（含跨缝移入者）

                        target = try_move(self, x, y)
                        if target is not None:
                            tx, ty = target
                            self.swap(x, y, tx, ty)
                            self._stamp(x, y)
                            self._stamp(tx, ty)
                            self._check_reactions(tx, ty)
                        else:
                            self._check_reactions(x, y)

        # 2. Lifetime decay（各格独立，顺序无关）
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
                    self.cells[base + UPDATED_AT] = 0  # 清残值，免死格历史进 hash

        # 3. Advance frame；写域复位为全世界（update 之外的 try_move 用）
        self._write_rect = Rect(0, 0, self.width, self.height)
        self.frame_count += 1

    def _check_reactions(self, x: int, y: int) -> None:
        type_a = self.get_type_id(x, y)
        if type_a == AIR:
            return
        neighbors = [(x, y - 1), (x, y + 1), (x - 1, y), (x + 1, y)]
        draw = 0  # attempt 局部计数：同格同帧多次掷骰必须去相关（D2）
        for nx, ny in neighbors:
            if not self._write_rect.contains(nx, ny):
                continue  # 读域夹断：越出写域的交互推迟到缝隙归属 chunk 的 pass（条件①）
            type_b = self.get_type_id(nx, ny)
            if type_b == AIR:
                continue
            results = self.reaction_table.get(type_a, type_b)
            if results is None:
                continue
            for result in results:
                hit = (
                    rng_u32(self._fseed, self._pass_id, x, y, SALT_REACTION, attempt=draw)
                    < result.threshold
                )
                draw += 1
                if hit:
                    out1 = type_a if result.output1 == SELF_MARKER else result.output1
                    out2 = type_b if result.output2 == SELF_MARKER else result.output2
                    self.set_cell(x, y, out1, stamp=True)    # 产物下帧才动（m2）
                    self.set_cell(nx, ny, out2, stamp=True)
                    return
