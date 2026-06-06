# M0.5 单线程 4-pass/chunk 调度器 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把串行全网格扫描切换为单线程 4-pass/chunk 调度（正方形写域 + 所有权制 + 世代戳），锁定 Phase 2 并行语义。

**Architecture:** 新增 `core/chunks.py`（纯几何）；STRIDE 4→5 加 `UPDATED_AT` 世代戳并删除 FLAG_DIRTY/FLAG_STATIC；`update()` 重写为 pass→chunk→域内扫描；`_can_move_to`/`_check_reactions` 接写域契约；RNG pass_id 接线。

**Tech Stack:** 同 M0（venv pytest）。测试命令 `PYTHONPATH=prototype venv/bin/python -m pytest prototype/tests -q`。

**Spec:** `docs/superpowers/specs/2026-06-07-m05-chunked-scheduler-design.md`（已批准，三决策记录在案）。

---

### Task 1: `core/chunks.py` 纯几何 + 单测

**Files:** Create `prototype/core/chunks.py`、`prototype/tests/test_chunks.py`

- [ ] **Step 1.1 失败测试**（test_chunks.py，见下方代码——含同 pass 写域两两不相交断言）
- [ ] **Step 1.2 跑 → ModuleNotFoundError**
- [ ] **Step 1.3 实现 chunks.py**：

```python
"""Chunk 划分与 4-pass 棋盘调度的纯几何（确定性提案 §2.2 条件①③）。"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Iterator

CHUNK = 64
MARGIN = 32
PASS_PARITY = ((0, 0), (1, 0), (0, 1), (1, 1))  # 固定 pass 序（D3）


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
            cx * CHUNK, cy * CHUNK,
            min((cx + 1) * CHUNK, self.width),
            min((cy + 1) * CHUNK, self.height),
        )

    def write_rect(self, cx: int, cy: int) -> Rect:
        """正方形写域 [chunk−32, chunk+96)²，裁剪到世界（评审 B2）。"""
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
```

test_chunks.py 断言集：Rect.contains 边界；128²→2×2 / 192²→3×3 / 100×70→2×2；chunk_rect(1,1)@100×70 == Rect(64,64,100,70)；write_rect(1,1)@192² == Rect(32,32,160,160)、(0,0)→(0,0,96,96)、(2,2)→(96,96,192,192)；四 pass 覆盖全部 chunk 恰一次；同 pass parity 一致；**同 pass write_rect 两两不相交（320² 穷举）**。
- [ ] **Step 1.4 绿 → commit** `feat(m0.5): chunk layout geometry (square write domains, 4-pass parity)`

### Task 2: 世代戳 + 调度器（一个语义单元，不可拆 commit）

**Files:** Modify `prototype/core/cell.py`（全文）、`prototype/core/grid.py`（init/set_cell/update/_check_reactions）、`prototype/core/rules.py`（`_can_move_to` + 5 处 pass_id）

- [ ] **Step 2.1 cell.py**：STRIDE=5、UPDATED_AT=4、删 FLAG_DIRTY/FLAG_STATIC（FLAGS 字段保留给 FLAG_BURNING）、docstring 更新。
- [ ] **Step 2.2 grid.py**：
  - import：删 FLAG_DIRTY、加 UPDATED_AT、`from core.chunks import ChunkLayout, Rect`
  - `__init__`：`self.layout = ChunkLayout(width, height)`；`self._pass_id = 0`；`self._write_rect = Rect(0, 0, width, height)`（测试可直接调 try_move）
  - `set_cell(..., stamp: bool = False)`：`cells[base+UPDATED_AT] = self.frame_count if stamp else -1`（决策②）
  - `_stamp(x, y)` helper
  - `update()` 重写（spec §4）：删 clear-dirty pass；4-pass × chunks_for_pass × chunk 内自底向上/帧奇偶 x 交替；skip 条件 `UPDATED_AT == frame_count`；移动后双方 `_stamp`；lifetime pass 清格时连同 UPDATED_AT 清 0
  - `_check_reactions`：邻居 `not self._write_rect.contains(nx, ny)` → continue（读域夹断）；`rng_u32(..., self._pass_id, ...)`；两个 set_cell 传 `stamp=True`
- [ ] **Step 2.3 rules.py**：`_can_move_to` 的 `grid.in_bounds(x, y)` → `grid._write_rect.contains(x, y)`（写域已裁剪到世界，蕴含 in_bounds）；5 处 RNG 调用 `0` → `grid._pass_id`。
- [ ] **Step 2.4 全套跑**：预期既有 56 测试全过（8×8=单 chunk pass0 行为同旧串行；run-vs-run 确定性测试自动存活）。若 fail，按"扫描序/skip 条件/stamp 语义"三处排查。
- [ ] **Step 2.5 commit** `feat(m0.5): 4-pass chunk scheduler + generation stamps (replaces FLAG_DIRTY)`

### Task 3: 语义验收测试

**Files:** Create `prototype/tests/test_chunked_semantics.py`（世界 192×128 = 3×2 chunk，控制时长）

- [ ] **Step 3.1 写测试**：
  - `test_conservation_across_seams`：real toml，底墙 + 跨 x=64/128 缝的沙块 + 水层，80 帧后逐材质计数不变（场景不含 lifetime 材质）
  - `test_sand_column_crosses_horizontal_seam`：x=96 沙柱 y40–59，120 帧后计数不变 + 全部沉到 y≥100 区
  - `test_water_flows_across_vertical_seam`：水贴 x=64 缝左侧，160 帧后右侧出现水
  - `test_same_seed_and_pollution_at_192`：两世界同 seed 同步推进 40 帧，B 侧帧间污染全局 random → hash 序列相等
  - `test_reaction_product_does_not_act_same_frame`：fixture 反应 p=1.0（lava 下 water 上、墙封侧底、上方开洞）——lava 行先扫到触发反应，steam 产物生成在**未扫描的上一行**，盖戳 → 本帧不上浮（直查格子），第 2 帧才动。**该测试在 stamp=True 缺失时必红**（决策②的红绿验证）
  - `test_write_rect_clamps_movement`：手工把 `grid._write_rect` 设为小矩形，断言 `_can_move_to` 拒绝域外目标（域契约逻辑直测）
- [ ] **Step 3.2 跑绿 → commit** `test(m0.5): seam conservation + product-stamp semantics (192x128)`

### Task 4: benchmark 双尺寸 + baseline

**Files:** Modify `prototype/benchmark.py`（提取 `bench(w, h)`，输出 128² 主行 + 192² 副行）、`docs/perf/baseline.md`

- [ ] **Step 4.1 重构 + 跑**（两行输出）
- [ ] **Step 4.2 baseline.md 追加"M0.5 后"小节**（对比 M0 后 23.0 FPS，预算 20%）
- [ ] **Step 4.3 commit** `perf(m0.5): scheduler benchmark (128 primary + 192 datapoint)`

### Task 5: 收尾

- [ ] fresh 全套 + replay CLI 冒烟（M0 工具复跑）→ CHANGELOG + session → commit

## Self-Review 记录

- Spec 覆盖：§1→T1；§2/§3/§4/§5/§6→T2；§7→T1/T3；§8→T4 ✅
- 红绿验证：决策②有专门的"缺 stamp 必红"测试（T3）✅
- 类型一致：`Rect`/`ChunkLayout`/`_write_rect`/`_pass_id`/`UPDATED_AT` 贯穿 T1–T3 ✅
- 已知行为变化（spec §10）：缝隙相位差、产物延迟 1 帧、M0 hash 序列作废——测试均为 run-vs-run，不硬编码 hash 值 ✅
