# M0 确定性地基 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用 counter-based RNG / state hash / demo 录制回放 / 加载层整数化，把 Python 原型升级到确定性契约（D1/D2/D5/D7），验收 = 污染测试 + RNG 金值 + 回放等价 + benchmark 入档。

**Architecture:** 新增 `core/rng.py`（SquirrelNoise5 + 素数折叠单次哈希）与 `core/ops.py`（笔刷应用，UI/回放共用）；`CellGrid` 持 seed 并每帧预计算 `_fseed`，`state_hash()` 用 zlib.crc32；6 处 `random.*` 调用点换成 keyed 取数；`replay.py` 提供 JSONL 录制/headless 回放。

**Tech Stack:** Python 3.13 + pytest（venv 在仓库根 `venv/`，已装好）。测试命令统一 `PYTHONPATH=prototype venv/bin/python -m pytest prototype/tests -q`。

**Spec:** `docs/superpowers/specs/2026-06-07-m0-determinism-design.md`（已批准）。

---

## File Structure

| 文件 | 动作 | 职责 |
|---|---|---|
| `prototype/core/rng.py` | 新建 | squirrel5 / frame_seed / rng_u32 / threshold_u32 / order2 / perm3 / salt 常量 |
| `prototype/core/ops.py` | 新建 | `apply_brush`（InputHandler 与回放共用） |
| `prototype/replay.py` | 新建 | Recorder + `replay_file` + CLI（headless，禁 import render/pygame） |
| `prototype/benchmark.py` | 新建 | 128×128 / ~30% / 200 帧 → CLAUDE §5.3 格式输出 |
| `prototype/core/grid.py` | 修改 | seed/_fseed/state_hash；`_check_reactions` 换 keyed RNG；删 `import random` |
| `prototype/core/rules.py` | 修改 | 5 处 random → order2/perm3/rng_chance；删 `import random` |
| `prototype/core/material.py` | 修改 | density → int（`int(round())` 兼容 float 写法） |
| `prototype/core/reaction.py` | 修改 | probability → threshold(u32) |
| `prototype/data/materials.toml` | 修改 | density ×10 整数化 |
| `prototype/render/input_handler.py` | 修改 | 用 `apply_brush` + recorder 钩子 |
| `prototype/main.py` | 修改 | `--seed` / `--record` argparse |
| `prototype/tests/test_rng.py` | 新建 | 金值 + key 分量独立性 |
| `prototype/tests/test_determinism.py` | 新建 | 同 seed / 污染 / 录放等价 |
| `prototype/tests/{conftest,test_*}.py` | 修改 | 密度整数化 + 删 random.seed + threshold 断言 |
| `docs/perf/baseline.md` | 修改 | 实测数字替换 provisional |

---

### Task 1: `core/rng.py`

**Files:** Create `prototype/core/rng.py`、Create `prototype/tests/test_rng.py`

- [ ] **Step 1.1: 写失败测试（结构性，不含金值）**

```python
# prototype/tests/test_rng.py
from core.rng import (
    MASK32, frame_seed, order2, perm3, rng_chance, rng_u32, squirrel5, threshold_u32,
)


def test_u32_range():
    for pos in (0, 1, 12345, 0xFFFFFFFF):
        v = squirrel5(pos, 0)
        assert 0 <= v <= MASK32


def test_deterministic():
    assert squirrel5(42, 7) == squirrel5(42, 7)
    assert rng_u32(99, 0, 3, 5, 1, 0) == rng_u32(99, 0, 3, 5, 1, 0)


def test_key_component_independence():
    base = rng_u32(100, 0, 10, 20, 1, 0)
    assert rng_u32(101, 0, 10, 20, 1, 0) != base   # fseed
    assert rng_u32(100, 1, 10, 20, 1, 0) != base   # pass_id
    assert rng_u32(100, 0, 11, 20, 1, 0) != base   # x
    assert rng_u32(100, 0, 10, 21, 1, 0) != base   # y
    assert rng_u32(100, 0, 10, 20, 2, 0) != base   # salt
    assert rng_u32(100, 0, 10, 20, 1, 1) != base   # attempt


def test_threshold_u32():
    assert threshold_u32(0.0) == 0
    assert threshold_u32(1.0) == 0xFFFFFFFF          # 钳位
    assert threshold_u32(0.5) == 2147483648          # 0.5 × 2^32 精确
    assert rng_chance(1, 0, 0, 0, 1, threshold_u32(1.0)) in (True, False)


def test_order2_and_perm3_values():
    assert order2(5, 0, 1, 2, 1) in ((0, 1), (1, 0))
    assert perm3(5, 0, 1, 2, 3) in (
        (0, 1, 2), (0, 2, 1), (1, 0, 2), (1, 2, 0), (2, 0, 1), (2, 1, 0),
    )


# 金值锚点：Step 1.5 生成后填入，防 squirrel5 实现被无声改动
GOLDEN = {}  # {(pos, seed): expected}


def test_golden_values():
    assert GOLDEN, "Step 1.5 必须填入金值"
    for (pos, seed), expected in GOLDEN.items():
        assert squirrel5(pos, seed) == expected
```

- [ ] **Step 1.2: 跑测试确认失败**

Run: `PYTHONPATH=prototype venv/bin/python -m pytest prototype/tests/test_rng.py -q`
Expected: FAIL（ModuleNotFoundError: core.rng）

- [ ] **Step 1.3: 实现 `core/rng.py`**

```python
"""Counter-based RNG（确定性契约 D2）。

key = (world_seed, tick, pass_id, x, y, salt, attempt)：
frame_seed 每帧预计算一次，其余分量素数折叠后单次 squirrel5。
(x, y) 取决策时刻像素坐标；attempt 为该 (坐标, salt) 本 tick 本 pass 第 N 次取数。
"""
from __future__ import annotations

MASK32 = 0xFFFFFFFF

# SquirrelNoise5 常量（Squirrel Eiserloh / kevinmoran gist）
_N1 = 0xD2A80A3F
_N2 = 0xA884F197
_N3 = 0x6C736F4B
_N4 = 0xB79F3ABB
_N5 = 0x1B56C4F5

# key 折叠用互异大奇素数
P_X = 0x9E3779B1
P_Y = 0x85EBCA77
P_PASS = 0xC2B2AE3D
P_SALT = 0x27D4EB2F
P_ATTEMPT = 0x165667B1

# decision_salt 注册表（fire spec v2 预订 10–12）
SALT_DIAG = 1
SALT_ENERGY_LINGER = 2
SALT_ENERGY_DIR = 3
SALT_REACTION = 4

_PERMS3 = ((0, 1, 2), (0, 2, 1), (1, 0, 2), (1, 2, 0), (2, 0, 1), (2, 1, 0))


def squirrel5(pos: int, seed: int) -> int:
    m = (pos * _N1) & MASK32
    m = (m + seed) & MASK32
    m ^= m >> 9
    m = (m + _N2) & MASK32
    m ^= m >> 11
    m = (m * _N3) & MASK32
    m ^= m >> 13
    m = (m + _N4) & MASK32
    m ^= m >> 15
    m = (m * _N5) & MASK32
    m ^= m >> 17
    return m


def frame_seed(world_seed: int, tick: int) -> int:
    return squirrel5(tick & MASK32, world_seed & MASK32)


def rng_u32(fseed: int, pass_id: int, x: int, y: int, salt: int, attempt: int = 0) -> int:
    pos = (x * P_X + y * P_Y + pass_id * P_PASS + salt * P_SALT + attempt * P_ATTEMPT) & MASK32
    return squirrel5(pos, fseed)


def threshold_u32(p: float) -> int:
    """概率 → u32 阈值。2 的幂缩放无精度损失；p=1.0 钳位（失败率 2^-32 可忽略）。"""
    return min(round(p * 4294967296), 4294967295)


def rng_chance(fseed: int, pass_id: int, x: int, y: int, salt: int, threshold: int, attempt: int = 0) -> bool:
    return rng_u32(fseed, pass_id, x, y, salt, attempt) < threshold


def order2(fseed: int, pass_id: int, x: int, y: int, salt: int) -> tuple[int, int]:
    return (0, 1) if rng_u32(fseed, pass_id, x, y, salt) & 1 == 0 else (1, 0)


def perm3(fseed: int, pass_id: int, x: int, y: int, salt: int) -> tuple[int, int, int]:
    return _PERMS3[rng_u32(fseed, pass_id, x, y, salt) % 6]
```

- [ ] **Step 1.4: 跑测试**（金值测试仍 FAIL，其余 PASS）

Run: `PYTHONPATH=prototype venv/bin/python -m pytest prototype/tests/test_rng.py -q`
Expected: 5 passed, 1 failed (test_golden_values)

- [ ] **Step 1.5: 生成金值并填入 GOLDEN**

Run: `PYTHONPATH=prototype venv/bin/python -c "from core.rng import squirrel5; print({(p,s): squirrel5(p,s) for p,s in [(0,0),(1,0),(0,1),(12345,67890),(0xFFFFFFFF,0xDEADBEEF)]})"`
把输出原样填进 `test_rng.py` 的 `GOLDEN`。

- [ ] **Step 1.6: 全绿** → Run 同 1.4，Expected: 6 passed

- [ ] **Step 1.7: Commit** `git add prototype/core/rng.py prototype/tests/test_rng.py && git commit -m "feat(m0): counter-based RNG module (SquirrelNoise5 + 7-tuple key fold)"`

---

### Task 2: CellGrid seed / `_fseed` / `state_hash`

**Files:** Modify `prototype/core/grid.py`（`__init__:11-23`、`update():54`）、Modify `prototype/tests/test_grid.py`

- [ ] **Step 2.1: 失败测试（追加到 test_grid.py 末尾）**

```python
def test_state_hash_changes_and_repeats(grid):
    h0 = grid.state_hash()
    assert grid.state_hash() == h0          # 无变化 → 不变
    sand_id = grid.registry.get_by_name("sand").type_id
    grid.set_cell(2, 2, sand_id)
    h1 = grid.state_hash()
    assert h1 != h0                          # 有变化 → 变


def test_grid_seed_and_fseed(grid):
    assert grid.seed == 0                    # 默认 seed
    f0 = grid._fseed
    grid.update()
    assert grid._fseed != f0                 # 每帧重算
```

- [ ] **Step 2.2: 跑测试确认失败** → `... -m pytest prototype/tests/test_grid.py -q` Expected: 2 failed（AttributeError）

- [ ] **Step 2.3: 实现** —— `grid.py` 头部加：

```python
import zlib
from array import array

from core.rng import SALT_REACTION, frame_seed, rng_u32
```

`__init__` 签名与体：

```python
    def __init__(
        self,
        width: int,
        height: int,
        registry: MaterialRegistry,
        reaction_table: ReactionTable,
        seed: int = 0,
    ) -> None:
        ...
        self.seed = seed
        self.frame_count = 0
        self._fseed = frame_seed(seed, 0)
        self.cells: list[int] = [0] * (width * height * STRIDE)
```

`update()` 第一行加 `self._fseed = frame_seed(self.seed, self.frame_count)`；类尾加：

```python
    def state_hash(self) -> int:
        """世界状态 CRC32（D5）。同机确定；跨平台字节序口径 C# 期再钉。"""
        return zlib.crc32(array("i", self.cells).tobytes())
```

- [ ] **Step 2.4: 跑 test_grid.py 全绿** → Expected: all passed
- [ ] **Step 2.5: Commit** `git commit -am "feat(m0): CellGrid seed + per-frame fseed + crc32 state_hash"`

---

### Task 3: D1 加载层整数化（material / reaction / TOML / 测试重钉）

**Files:** Modify `prototype/core/material.py:13,49`、`prototype/core/reaction.py:11-44`、`prototype/data/materials.toml`、`prototype/tests/{conftest.py,test_materials.py,test_reactions.py,test_grid.py,test_rules.py,test_renderer.py}`

- [ ] **Step 3.1: 失败测试** —— `test_materials.py` 改断言：`assert sand.density == 60`（fixture 同步 ×10：wall 100 / sand 60 / water 10）、`test_air_is_type_id_zero` 改 `assert air.density == 0`；`test_reactions.py` 头部加 `from core.rng import threshold_u32`，`test_direct_reaction_lookup` 改 `assert r.threshold == threshold_u32(0.8)`，`test_probability_stored` 重命名 `test_threshold_stored` 改 `assert all(0 <= r.threshold <= 0xFFFFFFFF for r in results)`，fixture 密度 ×10（wall 100 / water 10 / oil 8 / lava 30 / steam 1 / fire 0 / rock 90）。
- [ ] **Step 3.2: 确认失败** → Expected: test_materials 2 failed、test_reactions 2 failed
- [ ] **Step 3.3: 实现**
  - `material.py`：`MaterialDef.density: int`；`_AIR` 的 `density=0`；加载处 `density=int(round(float(props["density"])))`。
  - `reaction.py`：`ReactionResult(output1, output2, threshold)`（int）；头部 `from core.rng import threshold_u32`；构造处 `threshold=threshold_u32(probability)`（forward/reverse 同值）。
  - `data/materials.toml`：wall 100 / rock 90 / wood 80 / sand 60 / water 10 / oil 8 / lava 30 / steam 1 / fire 0。
  - 其余 fixture TOML 密度 ×10：`conftest.py`（100/60/10）、`test_grid.py` 两处 fixture（含 FULL_TOML：oil 8、lava 30、steam 1、fire 0、rock 90）、`test_rules.py`、`test_renderer.py`。
- [ ] **Step 3.4: 全套跑** → `... -m pytest prototype/tests -q` Expected: all passed（此刻 random 调用点未动，行为不变）
- [ ] **Step 3.5: Commit** `git commit -am "feat(m0): integer densities + u32 reaction thresholds (D1)"`

---

### Task 4: 替换 6 处 `random.*` + 测试重钉

**Files:** Modify `prototype/core/rules.py`（全文 5 处）、`prototype/core/grid.py:_check_reactions`、`prototype/tests/test_rules.py`、`prototype/tests/test_grid.py`

- [ ] **Step 4.1: 先改测试（确定性重钉）** —— `test_rules.py`：删 `import random` 与所有 `random.seed(...)`；两个 seed-scan 循环 collapse：

```python
def test_powder_falls_diagonal_when_blocked(env):
    reg, _ = env
    grid = make_grid(env)
    sand_id = reg.get_by_name("sand").type_id
    wall_id = reg.get_by_name("wall").type_id
    grid.set_cell(3, 3, sand_id)
    grid.set_cell(3, 4, wall_id)
    from core.rules import try_move
    result = try_move(grid, 3, 3)
    assert result is not None
    assert result[1] == 4
    assert result[0] in (2, 4)


def test_liquid_spreads_horizontally(env):
    reg, _ = env
    grid = make_grid(env)
    water_id = reg.get_by_name("water").type_id
    wall_id = reg.get_by_name("wall").type_id
    grid.set_cell(3, 6, water_id)
    grid.set_cell(3, 7, wall_id)
    grid.set_cell(2, 7, wall_id)
    grid.set_cell(4, 7, wall_id)
    from core.rules import try_move
    r = try_move(grid, 3, 6)
    assert r is not None
    assert r[1] == 6
    assert r[0] in (2, 4)
```

`test_grid.py`：删全部 5 处 `import random` / `random.seed(42|123)`。
- [ ] **Step 4.2: 跑确认现状仍绿**（旧实现下这些测试本就该过；diagonal 单断言在旧 shuffle 下也成立）
- [ ] **Step 4.3: 实现 `rules.py`** —— 头部：

```python
from core.cell import AIR, VELOCITY
from core.grid import CellGrid
from core.rng import (
    SALT_DIAG, SALT_ENERGY_DIR, SALT_ENERGY_LINGER,
    order2, perm3, rng_chance, threshold_u32,
)

ENERGY_LINGER_THRESHOLD = threshold_u32(0.4)
```

`_move_powder` 对角段（liquid/gas 同模式，gas 为 y-1）：

```python
    diags = ((x - 1, y + 1), (x + 1, y + 1))
    for i in order2(grid._fseed, 0, x, y, SALT_DIAG):
        dx, dy = diags[i]
        if _can_move_to(grid, dx, dy, density, heavier_sinks=True):
            return (dx, dy)
```

`_move_energy`：

```python
def _move_energy(grid: CellGrid, x: int, y: int) -> Optional[tuple[int, int]]:
    # 40% 驻留——让火贴住燃料蔓延
    if rng_chance(grid._fseed, 0, x, y, SALT_ENERGY_LINGER, ENERGY_LINGER_THRESHOLD):
        return None
    candidates = ((x, y - 1), (x - 1, y - 1), (x + 1, y - 1))
    for i in perm3(grid._fseed, 0, x, y, SALT_ENERGY_DIR):
        cx, cy = candidates[i]
        if grid.in_bounds(cx, cy) and grid.get_type_id(cx, cy) == AIR:
            return (cx, cy)
    return None
```

删 `import random`。
- [ ] **Step 4.4: 实现 `grid.py._check_reactions`**（attempt 局部计数）：

```python
    def _check_reactions(self, x: int, y: int) -> None:
        type_a = self.get_type_id(x, y)
        if type_a == AIR:
            return
        neighbors = [(x, y - 1), (x, y + 1), (x - 1, y), (x + 1, y)]
        draw = 0
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
                hit = rng_u32(self._fseed, 0, x, y, SALT_REACTION, attempt=draw) < result.threshold
                draw += 1
                if hit:
                    out1 = type_a if result.output1 == SELF_MARKER else result.output1
                    out2 = type_b if result.output2 == SELF_MARKER else result.output2
                    self.set_cell(x, y, out1)
                    self.set_cell(nx, ny, out2)
                    return
```

删 `grid.py` 的 `import random`。
- [ ] **Step 4.5: 全套跑** → Expected: all passed
- [ ] **Step 4.6: 防回归断言（追加 test_rng.py）**：

```python
def test_no_global_random_in_sim_modules():
    import core.grid, core.rules
    import inspect
    for mod in (core.grid, core.rules):
        src = inspect.getsource(mod)
        assert "import random" not in src, f"{mod.__name__} 不得使用全局 random（D2）"
```

- [ ] **Step 4.7: Commit** `git commit -am "feat(m0): replace all 6 global random call sites with keyed counter RNG (D2)"`

---

### Task 5: `core/ops.py` + InputHandler 钩子

**Files:** Create `prototype/core/ops.py`、Modify `prototype/render/input_handler.py:21-27,54-70`

- [ ] **Step 5.1: 失败测试（新建 `prototype/tests/test_ops.py`）**

```python
from core.ops import apply_brush


def make_grid(tmp_path):
    from core.material import MaterialRegistry
    from core.reaction import ReactionTable
    from core.grid import CellGrid
    f = tmp_path / "m.toml"
    f.write_text("""
[meta]
version = 1
[materials.sand]
cell_type = "powder"
density = 60
color = [194, 178, 128]
tags = ["powder"]
""")
    reg = MaterialRegistry(str(f))
    return CellGrid(8, 8, reg, ReactionTable(str(f), reg)), reg


def test_apply_brush_writes_square(tmp_path):
    grid, reg = make_grid(tmp_path)
    sand = reg.get_by_name("sand").type_id
    written = apply_brush(grid, 4, 4, sand, brush_size=3)
    assert set(written) == {(x, y) for x in (3, 4, 5) for y in (3, 4, 5)}
    assert grid.get_type_id(3, 3) == sand


def test_apply_brush_clips_bounds(tmp_path):
    grid, reg = make_grid(tmp_path)
    sand = reg.get_by_name("sand").type_id
    written = apply_brush(grid, 0, 0, sand, brush_size=3)
    assert set(written) == {(0, 0), (0, 1), (1, 0), (1, 1)}
```

- [ ] **Step 5.2: 确认失败** → ModuleNotFoundError
- [ ] **Step 5.3: 实现 `core/ops.py`**

```python
"""世界写入操作——InputHandler 与 demo 回放共用，保证两路径逐位一致（D7）。"""
from __future__ import annotations

from core.grid import CellGrid


def apply_brush(
    grid: CellGrid, gx: int, gy: int, type_id: int, brush_size: int
) -> list[tuple[int, int]]:
    r = brush_size // 2
    written: list[tuple[int, int]] = []
    for dy in range(-r, r + 1):
        for dx in range(-r, r + 1):
            px, py = gx + dx, gy + dy
            if grid.in_bounds(px, py):
                grid.set_cell(px, py, type_id)
                written.append((px, py))
    return written
```

- [ ] **Step 5.4: InputHandler 改造** —— `__init__(self, registry, scale=4, recorder=None)` 存 `self.recorder`；鼠标段替换为：

```python
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
```

头部 `from core.ops import apply_brush`。
- [ ] **Step 5.5: 跑全套绿 + Commit** `git commit -am "feat(m0): shared apply_brush op + recorder hook in InputHandler"`

---

### Task 6: `replay.py` + `main.py` CLI

**Files:** Create `prototype/replay.py`、Modify `prototype/main.py`

- [ ] **Step 6.1: 实现 `replay.py`**（测试在 Task 7 的录放等价里覆盖；headless——不得 import render/pygame）

```python
#!/usr/bin/env python3
"""Demo 录制/回放（D7）。JSONL：首行 header，其后 paint 事件行。

回放语义与 main.py 主循环一致：先应用本帧事件，再 grid.update()。
header 的 toml_sha256/尺寸/seed 不匹配即拒绝（评审 m7）。
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

from core.grid import CellGrid
from core.material import MaterialRegistry
from core.ops import apply_brush
from core.reaction import ReactionTable

FORMAT_VERSION = 1
DEFAULT_TOML = str(Path(__file__).parent / "data" / "materials.toml")


def toml_sha256(toml_path: str) -> str:
    return hashlib.sha256(Path(toml_path).read_bytes()).hexdigest()


class Recorder:
    def __init__(self, out_path: str, toml_path: str, width: int, height: int, seed: int) -> None:
        self._f = open(out_path, "w", encoding="utf-8")
        header = {
            "v": FORMAT_VERSION, "toml_sha256": toml_sha256(toml_path),
            "w": width, "h": height, "seed": seed,
        }
        self._f.write(json.dumps(header) + "\n")

    def log_paint(self, frame: int, gx: int, gy: int, type_id: int, brush_size: int) -> None:
        self._f.write(json.dumps(
            {"f": frame, "op": "paint", "x": gx, "y": gy, "id": type_id, "r": brush_size}
        ) + "\n")

    def close(self) -> None:
        self._f.close()


def replay_file(
    demo_path: str, toml_path: str = DEFAULT_TOML,
    extra_frames: int = 0, hash_every: int = 0,
) -> list[tuple[int, int]]:
    """回放并返回 [(frame, state_hash)]，末项恒为最终帧。校验失败抛 ValueError。"""
    lines = Path(demo_path).read_text(encoding="utf-8").splitlines()
    header = json.loads(lines[0])
    if header["v"] != FORMAT_VERSION:
        raise ValueError(f"unsupported demo version {header['v']}")
    if header["toml_sha256"] != toml_sha256(toml_path):
        raise ValueError("materials.toml 与录制时不一致，拒绝回放")
    events = [json.loads(line) for line in lines[1:]]

    registry = MaterialRegistry(toml_path)
    table = ReactionTable(toml_path, registry)
    grid = CellGrid(header["w"], header["h"], registry, table, seed=header["seed"])

    last_event_frame = events[-1]["f"] if events else -1
    total_frames = last_event_frame + 1 + extra_frames
    hashes: list[tuple[int, int]] = []
    i = 0
    for frame in range(total_frames):
        while i < len(events) and events[i]["f"] == frame:
            e = events[i]
            apply_brush(grid, e["x"], e["y"], e["id"], e["r"])
            i += 1
        grid.update()
        if hash_every and frame % hash_every == 0:
            hashes.append((frame, grid.state_hash()))
    hashes.append((total_frames - 1, grid.state_hash()))
    return hashes


def main() -> None:
    ap = argparse.ArgumentParser(description="headless demo replayer")
    ap.add_argument("demo")
    ap.add_argument("--toml", default=DEFAULT_TOML)
    ap.add_argument("--extra-frames", type=int, default=0)
    ap.add_argument("--hash-every", type=int, default=0)
    args = ap.parse_args()
    for frame, h in replay_file(args.demo, args.toml, args.extra_frames, args.hash_every):
        print(f"frame {frame}: {h:08x}")


if __name__ == "__main__":
    main()
```

- [ ] **Step 6.2: `main.py` 加 argparse**（`main()` 开头）：

```python
    ap = argparse.ArgumentParser()
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--record", default=None, help="录制 demo 到 JSONL")
    args = ap.parse_args()
    ...
    grid = CellGrid(GRID_WIDTH, GRID_HEIGHT, registry, reaction_table, seed=args.seed)
    recorder = None
    if args.record:
        from replay import Recorder
        recorder = Recorder(args.record, TOML_PATH, GRID_WIDTH, GRID_HEIGHT, args.seed)
    input_handler = InputHandler(registry, scale=SCALE, recorder=recorder)
    ...
    # 主循环结束后：
    if recorder is not None:
        recorder.close()
```

（头部 `import argparse`。）
- [ ] **Step 6.3: 解析校验** → `PYTHONPATH=prototype venv/bin/python -c "import replay; print('ok')"` Expected: ok（且确认 replay.py 无 pygame import）
- [ ] **Step 6.4: Commit** `git commit -am "feat(m0): JSONL demo recorder + headless replayer with header validation"`

---

### Task 7: `test_determinism.py`（验收 ①②③）

**Files:** Create `prototype/tests/test_determinism.py`

- [ ] **Step 7.1: 写测试**

```python
"""M0 验收：①同 seed 等价 ②污染测试 ③录放等价（提案 §5 M0 行）。"""
import random
from pathlib import Path

import pytest

from core.grid import CellGrid
from core.material import MaterialRegistry
from core.ops import apply_brush
from replay import Recorder, replay_file

REAL_TOML = str(Path(__file__).parent.parent / "data" / "materials.toml")


def build_world(seed: int) -> CellGrid:
    from core.reaction import ReactionTable
    reg = MaterialRegistry(REAL_TOML)
    grid = CellGrid(64, 64, reg, ReactionTable(REAL_TOML, reg), seed=seed)
    wall = reg.get_by_name("wall").type_id
    sand = reg.get_by_name("sand").type_id
    water = reg.get_by_name("water").type_id
    lava = reg.get_by_name("lava").type_id
    for x in range(64):
        grid.set_cell(x, 63, wall)
    for x in range(8, 56):
        for y in range(5, 20):
            grid.set_cell(x, y, sand)        # 沙块（下落扰动）
    for x in range(8, 56):
        for y in range(50, 60):
            grid.set_cell(x, y, water)       # 水池
    for x in range(30, 34):
        grid.set_cell(x, 45, lava)           # 岩浆点（触发反应）
    return grid


def run_hashes(grid: CellGrid, frames: int, pollute: bool = False) -> list[int]:
    out = []
    for f in range(frames):
        if pollute:                          # 帧间扰动全局 random 流
            random.seed(f * 1337 + 1)
            random.random()
            random.shuffle([1, 2, 3])
        grid.update()
        out.append(grid.state_hash())
    return out


def test_same_seed_identical_hashes():
    a = run_hashes(build_world(seed=7), 120)
    b = run_hashes(build_world(seed=7), 120)
    assert a == b


def test_different_seed_diverges():
    a = run_hashes(build_world(seed=7), 60)
    b = run_hashes(build_world(seed=8), 60)
    assert a != b


def test_pollution_does_not_affect_sim():
    clean = run_hashes(build_world(seed=7), 120, pollute=False)
    dirty = run_hashes(build_world(seed=7), 120, pollute=True)
    assert clean == dirty                    # 核心谓词：sim 已脱离全局 random 流


def test_record_replay_roundtrip(tmp_path):
    demo = str(tmp_path / "demo.jsonl")
    reg = MaterialRegistry(REAL_TOML)
    from core.reaction import ReactionTable
    grid = CellGrid(64, 64, reg, ReactionTable(REAL_TOML, reg), seed=3)
    rec = Recorder(demo, REAL_TOML, 64, 64, seed=3)
    sand = reg.get_by_name("sand").type_id
    water = reg.get_by_name("water").type_id
    script = {0: (10, 5, sand, 5), 3: (40, 5, water, 5), 7: (20, 30, sand, 3)}
    total = 40
    for frame in range(total):
        if frame in script:
            gx, gy, tid, brush = script[frame]
            apply_brush(grid, gx, gy, tid, brush)
            rec.log_paint(frame, gx, gy, tid, brush)
        grid.update()
    rec.close()
    live_hash = grid.state_hash()
    hashes = replay_file(demo, REAL_TOML, extra_frames=total - 7 - 1)
    assert hashes[-1][1] == live_hash        # 回放终值 == 实时跑终值


def test_replay_rejects_wrong_toml(tmp_path):
    demo = str(tmp_path / "demo.jsonl")
    rec = Recorder(demo, REAL_TOML, 8, 8, seed=0)
    rec.close()
    other = tmp_path / "other.toml"
    other.write_text(Path(REAL_TOML).read_text() + "\n# changed\n")
    with pytest.raises(ValueError):
        replay_file(demo, str(other))
```

- [ ] **Step 7.2: 跑测试** → Expected: 5 passed（若 roundtrip 失败，按"实时循环与 replay_file 的帧语义是否一致"排查——两边都是先事件后 update）
- [ ] **Step 7.3: Commit** `git commit -am "test(m0): determinism acceptance (same-seed, pollution, record-replay)"`

---

### Task 8: benchmark + baseline 入档

**Files:** Create `prototype/benchmark.py`、Modify `docs/perf/baseline.md`

- [ ] **Step 8.1: 实现 `benchmark.py`**

```python
#!/usr/bin/env python3
"""性能基准（CLAUDE.md §5.3）。输出：{w}x{h}, {ratio}% active, {fps} FPS"""
from __future__ import annotations

import time
from pathlib import Path

from core.cell import AIR
from core.grid import CellGrid
from core.material import MaterialRegistry
from core.reaction import ReactionTable

TOML = str(Path(__file__).parent / "data" / "materials.toml")
W, H, FRAMES = 128, 128, 200


def build() -> CellGrid:
    reg = MaterialRegistry(TOML)
    grid = CellGrid(W, H, reg, ReactionTable(TOML, reg), seed=42)
    wall = reg.get_by_name("wall").type_id
    sand = reg.get_by_name("sand").type_id
    water = reg.get_by_name("water").type_id
    for x in range(W):
        grid.set_cell(x, H - 1, wall)
    for x in range(8, 120):
        for y in range(10, 44):
            grid.set_cell(x, y, sand)        # ~21%
    for x in range(8, 120):
        for y in range(100, 114):
            grid.set_cell(x, y, water)       # ~10%
    return grid


def main() -> None:
    grid = build()
    non_air = sum(1 for i in range(W * H) if grid.cells[i * 4] != AIR)
    ratio = 100.0 * non_air / (W * H)
    t0 = time.perf_counter()
    for _ in range(FRAMES):
        grid.update()
    dt = time.perf_counter() - t0
    fps = FRAMES / dt
    print(f"{W}x{H}, {ratio:.0f}% active, {fps:.1f} FPS  ({dt/FRAMES*1000:.1f} ms/frame)")


if __name__ == "__main__":
    main()
```

注意：`grid.cells[i * 4]` 处的 `4` 是 STRIDE——直接 `from core.cell import STRIDE, TYPE_ID` 用 `grid.cells[i * STRIDE + TYPE_ID]`。
- [ ] **Step 8.2: 跑** → `PYTHONPATH=prototype venv/bin/python prototype/benchmark.py`（约 5–10 秒）
- [ ] **Step 8.3: 把实测数字写进 `docs/perf/baseline.md`**：新增"2026-06-07 — M0 后实测"小节（M0 前数字保留为对照；若想要 M0 前精确对照，可先在 Task 4 之前的 commit 上跑一次同款脚本——可选）。回退超 20% 预算须在 CHANGELOG 说明。
- [ ] **Step 8.4: Commit** `git commit -am "perf(m0): benchmark script + measured baseline"`

---

### Task 9: 收尾

- [ ] **Step 9.1: 全套测试** → `PYTHONPATH=prototype venv/bin/python -m pytest prototype/tests -q` Expected: all passed
- [ ] **Step 9.2: 视觉冒烟（用户手测，可选）**：`cd prototype && ../venv/bin/python main.py --seed 1 --record /tmp/smoke.jsonl`——画沙倒水，退出后 `replay /tmp/smoke.jsonl` 验证 hash 输出。
- [ ] **Step 9.3: CHANGELOG + session 落账**（Added：rng/ops/replay/benchmark + M0 完成；数字入账）
- [ ] **Step 9.4: 最终 commit**

---

## Self-Review 记录

- Spec 覆盖：D2→T1/T4；D5→T2/T7；D7→T5/T6/T7；D1→T3；benchmark→T8 ✅
- 占位符：仅 GOLDEN 留待 Step 1.5 生成（有确切生成命令，非 TBD）✅
- 类型一致性：`threshold` 字段名贯穿 reaction.py / grid.py / test_reactions.py；`_fseed` 贯穿 grid/rules；`apply_brush` 签名 T5/T6/T7 一致 ✅
- 已知行为变化（接受）：旧 `random.shuffle` → `order2/perm3` 改变随机序列（语义等价、分布一致）；`probability` 字段更名 `threshold`。
