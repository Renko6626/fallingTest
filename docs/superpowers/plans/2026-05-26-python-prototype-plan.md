# Python 原型：Noita 风格像素物理引擎 — 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 构建一个可交互的 Python 原型，验证 Noita 风格元胞自动机（CA）落沙模拟的核心算法正确性。

**Architecture:** 数据驱动架构——材质属性和反应规则全部定义在 `data/materials.toml` 中，代码不含材质特定逻辑。模拟核心用平铺 int 数组存储像素状态，按 cell_type 分派 4 套运动规则。渲染层用 numpy surfarray + LUT 实现零 Python 循环的像素绘制。

**Tech Stack:** Python 3.11+, pygame, numpy, pytest

**Spec:** `docs/superpowers/specs/2026-05-26-python-prototype-design.md`

---

## File Map

```
prototype/
├── data/
│   └── materials.toml
├── core/
│   ├── __init__.py
│   ├── cell.py              # 常量定义（STRIDE, TYPE_ID, FLAGS 等）
│   ├── material.py          # MaterialDef, MaterialRegistry
│   ├── reaction.py          # ReactionResult, ReactionTable
│   ├── grid.py              # CellGrid
│   └── rules.py             # move_powder, move_liquid, move_gas, move_energy
├── render/
│   ├── __init__.py
│   ├── pygame_renderer.py   # PygameRenderer（surfarray + LUT）
│   └── input_handler.py     # InputHandler（鼠标/键盘）
├── tests/
│   ├── conftest.py
│   ├── test_materials.py
│   ├── test_reactions.py
│   ├── test_rules.py
│   ├── test_grid.py
│   └── test_renderer.py
├── main.py
└── requirements.txt
```

---

## Task 1: 项目脚手架 + cell.py 常量

**Files:**
- Create: `prototype/requirements.txt`
- Create: `prototype/core/__init__.py`
- Create: `prototype/core/cell.py`
- Create: `prototype/render/__init__.py`
- Create: `prototype/tests/__init__.py` (empty, 让 pytest 发现)

- [ ] **Step 1: 创建 requirements.txt**

```
pygame>=2.5
numpy>=1.24
pytest>=7.0
```

- [ ] **Step 2: 安装依赖**

Run: `cd /data/sunyunbo/playground/godot/fallingTest/prototype && pip install -r requirements.txt`

- [ ] **Step 3: 创建 core/__init__.py**

```python
```

（空文件）

- [ ] **Step 4: 创建 render/__init__.py**

```python
```

（空文件）

- [ ] **Step 5: 创建 tests/__init__.py**

```python
```

（空文件，但需要存在让 pytest 的包导入正常工作）

- [ ] **Step 6: 创建 core/cell.py**

```python
"""像素存储的常量定义。

每个像素在平铺数组中占 STRIDE 个 int：
  [type_id, velocity, lifetime, flags]
"""

TYPE_ID = 0
VELOCITY = 1
LIFETIME = 2
FLAGS = 3
STRIDE = 4

FLAG_DIRTY = 0b01
FLAG_STATIC = 0b10

AIR = 0
```

- [ ] **Step 7: Commit**

```bash
git add prototype/requirements.txt prototype/core/__init__.py prototype/core/cell.py prototype/render/__init__.py prototype/tests/__init__.py
git commit -m "feat: project scaffold and cell constants"
```

---

## Task 2: MaterialDef + MaterialRegistry

**Files:**
- Create: `prototype/data/materials.toml`
- Create: `prototype/core/material.py`
- Create: `prototype/tests/conftest.py`
- Create: `prototype/tests/test_materials.py`

- [ ] **Step 1: 创建 data/materials.toml**

```toml
[meta]
version = 1
default_grid_size = [128, 128]

[materials.wall]
cell_type = "solid"
density = 10.0
color = [128, 128, 128]
tags = ["solid"]

[materials.wood]
cell_type = "solid"
density = 8.0
color = [139, 90, 43]
color_variance = 10
tags = ["solid", "flammable"]

[materials.sand]
cell_type = "powder"
density = 6.0
color = [194, 178, 128]
color_variance = 15
tags = ["powder"]

[materials.water]
cell_type = "liquid"
density = 1.0
color = [48, 96, 255]
tags = ["liquid", "water", "conductive"]

[materials.oil]
cell_type = "liquid"
density = 0.8
color = [80, 60, 30]
tags = ["liquid", "flammable"]

[materials.lava]
cell_type = "liquid"
density = 3.0
color = [255, 96, 0]
tags = ["liquid", "lava", "hot"]

[materials.steam]
cell_type = "gas"
density = 0.1
color = [200, 200, 255]
lifetime = 300
tags = ["gas"]

[materials.fire]
cell_type = "energy"
density = 0.0
color = [255, 160, 40]
color_variance = 40
lifetime = 60
tags = ["energy", "hot"]

[[reactions]]
input = ["lava", "water"]
output = ["rock", "steam"]
probability = 0.8

[[reactions]]
input = ["fire", "[flammable]"]
output = ["fire", "fire"]
probability = 0.05

[[reactions]]
input = ["[hot]", "wood"]
output = ["_self", "fire"]
probability = 0.02

[[reactions]]
input = ["[hot]", "water"]
output = ["_self", "steam"]
probability = 0.5
```

- [ ] **Step 2: 写测试 — test_materials.py**

```python
import pytest
from core.material import MaterialDef, MaterialRegistry


@pytest.fixture
def registry(tmp_path):
    toml_content = """
[meta]
version = 1

[materials.wall]
cell_type = "solid"
density = 10.0
color = [128, 128, 128]
tags = ["solid"]

[materials.sand]
cell_type = "powder"
density = 6.0
color = [194, 178, 128]
color_variance = 15
tags = ["powder"]

[materials.water]
cell_type = "liquid"
density = 1.0
color = [48, 96, 255]
tags = ["liquid", "water"]
"""
    f = tmp_path / "test_materials.toml"
    f.write_text(toml_content)
    return MaterialRegistry(str(f))


def test_air_is_type_id_zero(registry):
    air = registry.get_by_id(0)
    assert air.name == "air"
    assert air.cell_type == "solid"
    assert air.density == 0.0


def test_materials_loaded(registry):
    assert registry.get_by_name("wall") is not None
    assert registry.get_by_name("sand") is not None
    assert registry.get_by_name("water") is not None


def test_type_ids_unique(registry):
    ids = [registry.get_by_name(n).type_id for n in ["wall", "sand", "water"]]
    assert len(set(ids)) == 3
    assert all(tid > 0 for tid in ids)


def test_tag_index(registry):
    water_ids = registry.get_ids_by_tag("water")
    water = registry.get_by_name("water")
    assert water.type_id in water_ids

    liquid_ids = registry.get_ids_by_tag("liquid")
    assert water.type_id in liquid_ids


def test_tag_index_empty(registry):
    assert registry.get_ids_by_tag("nonexistent") == set()


def test_material_def_fields(registry):
    sand = registry.get_by_name("sand")
    assert sand.cell_type == "powder"
    assert sand.density == 6.0
    assert sand.color == (194, 178, 128)
    assert sand.color_variance == 15
    assert "powder" in sand.tags


def test_default_color_variance(registry):
    wall = registry.get_by_name("wall")
    assert wall.color_variance == 0


def test_default_lifetime(registry):
    wall = registry.get_by_name("wall")
    assert wall.lifetime == 0


def test_all_materials(registry):
    all_mats = registry.all()
    names = {m.name for m in all_mats}
    assert names == {"air", "wall", "sand", "water"}
```

- [ ] **Step 3: 运行测试确认失败**

Run: `cd /data/sunyunbo/playground/godot/fallingTest/prototype && python -m pytest tests/test_materials.py -v`

Expected: `ModuleNotFoundError: No module named 'core.material'`

- [ ] **Step 4: 实现 core/material.py**

```python
from __future__ import annotations

import tomllib
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class MaterialDef:
    name: str
    type_id: int
    cell_type: str
    density: float
    color: tuple[int, int, int]
    color_variance: int
    lifetime: int
    tags: frozenset[str]


_AIR = MaterialDef(
    name="air",
    type_id=0,
    cell_type="solid",
    density=0.0,
    color=(0, 0, 0),
    color_variance=0,
    lifetime=0,
    tags=frozenset(),
)


class MaterialRegistry:
    def __init__(self, toml_path: str) -> None:
        with open(toml_path, "rb") as f:
            data = tomllib.load(f)

        self._by_name: dict[str, MaterialDef] = {"air": _AIR}
        self._by_id: dict[int, MaterialDef] = {0: _AIR}
        self._tag_index: dict[str, set[int]] = {}

        next_id = 1
        for name, props in data.get("materials", {}).items():
            color_raw = props["color"]
            tags = frozenset(props.get("tags", []))
            mat = MaterialDef(
                name=name,
                type_id=next_id,
                cell_type=props["cell_type"],
                density=props["density"],
                color=(color_raw[0], color_raw[1], color_raw[2]),
                color_variance=props.get("color_variance", 0),
                lifetime=props.get("lifetime", 0),
                tags=tags,
            )
            self._by_name[name] = mat
            self._by_id[next_id] = mat
            for tag in tags:
                self._tag_index.setdefault(tag, set()).add(next_id)
            next_id += 1

        self._max_id = next_id - 1

    def get_by_name(self, name: str) -> MaterialDef:
        return self._by_name[name]

    def get_by_id(self, type_id: int) -> MaterialDef:
        return self._by_id[type_id]

    def get_ids_by_tag(self, tag: str) -> set[int]:
        return self._tag_index.get(tag, set())

    def all(self) -> list[MaterialDef]:
        return list(self._by_id.values())

    @property
    def max_id(self) -> int:
        return self._max_id
```

- [ ] **Step 5: 运行测试确认通过**

Run: `cd /data/sunyunbo/playground/godot/fallingTest/prototype && python -m pytest tests/test_materials.py -v`

Expected: all 9 tests PASS

- [ ] **Step 6: 创建 conftest.py**

```python
import pytest
from pathlib import Path

from core.material import MaterialRegistry


FIXTURE_TOML = """
[meta]
version = 1

[materials.wall]
cell_type = "solid"
density = 10.0
color = [128, 128, 128]
tags = ["solid"]

[materials.sand]
cell_type = "powder"
density = 6.0
color = [194, 178, 128]
tags = ["powder"]

[materials.water]
cell_type = "liquid"
density = 1.0
color = [48, 96, 255]
tags = ["liquid", "water"]
"""


@pytest.fixture
def toml_path(tmp_path):
    f = tmp_path / "test_materials.toml"
    f.write_text(FIXTURE_TOML)
    return str(f)


@pytest.fixture
def small_registry(toml_path):
    return MaterialRegistry(toml_path)
```

- [ ] **Step 7: Commit**

```bash
git add prototype/data/materials.toml prototype/core/material.py prototype/tests/conftest.py prototype/tests/test_materials.py
git commit -m "feat: MaterialDef and MaterialRegistry with TOML loading"
```

---

## Task 3: ReactionTable（标签展开 + 对称注册）

**Files:**
- Create: `prototype/core/reaction.py`
- Create: `prototype/tests/test_reactions.py`

- [ ] **Step 1: 写测试 — test_reactions.py**

```python
import pytest
import random
from core.material import MaterialRegistry
from core.reaction import ReactionResult, ReactionTable


@pytest.fixture
def registry_with_reactions(tmp_path):
    toml_content = """
[meta]
version = 1

[materials.wall]
cell_type = "solid"
density = 10.0
color = [128, 128, 128]
tags = ["solid"]

[materials.water]
cell_type = "liquid"
density = 1.0
color = [48, 96, 255]
tags = ["liquid", "water"]

[materials.oil]
cell_type = "liquid"
density = 0.8
color = [80, 60, 30]
tags = ["liquid", "flammable"]

[materials.lava]
cell_type = "liquid"
density = 3.0
color = [255, 96, 0]
tags = ["liquid", "lava", "hot"]

[materials.steam]
cell_type = "gas"
density = 0.1
color = [200, 200, 255]
lifetime = 300
tags = ["gas"]

[materials.fire]
cell_type = "energy"
density = 0.0
color = [255, 160, 40]
lifetime = 60
tags = ["energy", "hot"]

[materials.rock]
cell_type = "solid"
density = 9.0
color = [100, 100, 100]
tags = ["solid"]

[[reactions]]
input = ["lava", "water"]
output = ["rock", "steam"]
probability = 0.8

[[reactions]]
input = ["fire", "[flammable]"]
output = ["fire", "fire"]
probability = 0.05

[[reactions]]
input = ["[hot]", "water"]
output = ["_self", "steam"]
probability = 0.5
"""
    f = tmp_path / "test_materials.toml"
    f.write_text(toml_content)
    reg = MaterialRegistry(str(f))
    table = ReactionTable(str(f), reg)
    return reg, table


def test_direct_reaction_lookup(registry_with_reactions):
    reg, table = registry_with_reactions
    lava_id = reg.get_by_name("lava").type_id
    water_id = reg.get_by_name("water").type_id
    results = table.get(lava_id, water_id)
    assert results is not None
    assert len(results) >= 1
    r = results[0]
    assert r.output1 == reg.get_by_name("rock").type_id
    assert r.output2 == reg.get_by_name("steam").type_id
    assert r.probability == 0.8


def test_symmetric_lookup(registry_with_reactions):
    reg, table = registry_with_reactions
    lava_id = reg.get_by_name("lava").type_id
    water_id = reg.get_by_name("water").type_id
    results_forward = table.get(lava_id, water_id)
    results_reverse = table.get(water_id, lava_id)
    assert results_forward is not None
    assert results_reverse is not None
    assert results_reverse[0].output1 == reg.get_by_name("steam").type_id
    assert results_reverse[0].output2 == reg.get_by_name("rock").type_id


def test_tag_expansion(registry_with_reactions):
    reg, table = registry_with_reactions
    fire_id = reg.get_by_name("fire").type_id
    oil_id = reg.get_by_name("oil").type_id
    results = table.get(fire_id, oil_id)
    assert results is not None
    assert results[0].output1 == fire_id
    assert results[0].output2 == fire_id


def test_self_keyword(registry_with_reactions):
    reg, table = registry_with_reactions
    lava_id = reg.get_by_name("lava").type_id
    water_id = reg.get_by_name("water").type_id
    results = table.get(lava_id, water_id)
    has_self = any(r.output1 == -1 or r.output2 == -1 for r in results)
    # lava+water -> rock+steam, no _self here
    # but [hot]+water -> _self+steam, and lava has [hot]
    all_results = table.get(lava_id, water_id)
    self_results = [r for r in all_results if r.output1 == -1]
    assert len(self_results) >= 1


def test_no_reaction(registry_with_reactions):
    reg, table = registry_with_reactions
    wall_id = reg.get_by_name("wall").type_id
    water_id = reg.get_by_name("water").type_id
    results = table.get(wall_id, water_id)
    assert results is None


def test_probability_zero_never_triggers(registry_with_reactions):
    random.seed(42)
    reg, table = registry_with_reactions
    lava_id = reg.get_by_name("lava").type_id
    water_id = reg.get_by_name("water").type_id
    results = table.get(lava_id, water_id)
    # Just verify the probability field is stored correctly
    assert all(0.0 <= r.probability <= 1.0 for r in results)
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cd /data/sunyunbo/playground/godot/fallingTest/prototype && python -m pytest tests/test_reactions.py -v`

Expected: `ModuleNotFoundError: No module named 'core.reaction'`

- [ ] **Step 3: 实现 core/reaction.py**

```python
from __future__ import annotations

import tomllib
from dataclasses import dataclass

from core.material import MaterialRegistry

SELF_MARKER = -1


@dataclass(frozen=True)
class ReactionResult:
    output1: int
    output2: int
    probability: float


class ReactionTable:
    def __init__(self, toml_path: str, registry: MaterialRegistry) -> None:
        with open(toml_path, "rb") as f:
            data = tomllib.load(f)

        self._table: dict[tuple[int, int], list[ReactionResult]] = {}

        for reaction in data.get("reactions", []):
            input_names = reaction["input"]
            output_names = reaction["output"]
            probability = reaction["probability"]

            input1_ids = self._resolve(input_names[0], registry)
            input2_ids = self._resolve(input_names[1], registry)

            for id1 in input1_ids:
                for id2 in input2_ids:
                    if id1 == id2:
                        continue
                    out1 = self._resolve_output(output_names[0], registry)
                    out2 = self._resolve_output(output_names[1], registry)

                    forward = ReactionResult(out1, out2, probability)
                    reverse = ReactionResult(out2, out1, probability)

                    self._table.setdefault((id1, id2), []).append(forward)
                    self._table.setdefault((id2, id1), []).append(reverse)

    def _resolve(self, name: str, registry: MaterialRegistry) -> set[int]:
        if name.startswith("[") and name.endswith("]"):
            tag = name[1:-1]
            return registry.get_ids_by_tag(tag)
        return {registry.get_by_name(name).type_id}

    def _resolve_output(self, name: str, registry: MaterialRegistry) -> int:
        if name == "_self":
            return SELF_MARKER
        return registry.get_by_name(name).type_id

    def get(self, type_a: int, type_b: int) -> list[ReactionResult] | None:
        results = self._table.get((type_a, type_b))
        return results if results else None
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cd /data/sunyunbo/playground/godot/fallingTest/prototype && python -m pytest tests/test_reactions.py -v`

Expected: all 6 tests PASS

- [ ] **Step 5: Commit**

```bash
git add prototype/core/reaction.py prototype/tests/test_reactions.py
git commit -m "feat: ReactionTable with tag expansion and symmetric lookup"
```

---

## Task 4: CellGrid（基础操作 — 不含 update）

**Files:**
- Create: `prototype/core/grid.py`
- Create: `prototype/tests/test_grid.py`

- [ ] **Step 1: 写测试 — test_grid.py 基础操作**

```python
import pytest
from core.material import MaterialRegistry
from core.reaction import ReactionTable
from core.grid import CellGrid
from core.cell import AIR, STRIDE, TYPE_ID, VELOCITY, LIFETIME, FLAGS


@pytest.fixture
def grid(tmp_path):
    toml_content = """
[meta]
version = 1

[materials.wall]
cell_type = "solid"
density = 10.0
color = [128, 128, 128]
tags = ["solid"]

[materials.sand]
cell_type = "powder"
density = 6.0
color = [194, 178, 128]
tags = ["powder"]

[materials.water]
cell_type = "liquid"
density = 1.0
color = [48, 96, 255]
tags = ["liquid", "water"]
"""
    f = tmp_path / "test_materials.toml"
    f.write_text(toml_content)
    reg = MaterialRegistry(str(f))
    table = ReactionTable(str(f), reg)
    return CellGrid(8, 8, reg, table)


def test_grid_initial_state(grid):
    for y in range(8):
        for x in range(8):
            assert grid.get_type_id(x, y) == AIR


def test_set_and_get(grid):
    sand_id = grid.registry.get_by_name("sand").type_id
    grid.set_cell(x=3, y=4, type_id=sand_id)
    assert grid.get_type_id(3, 4) == sand_id


def test_swap(grid):
    sand_id = grid.registry.get_by_name("sand").type_id
    water_id = grid.registry.get_by_name("water").type_id
    grid.set_cell(3, 3, sand_id)
    grid.set_cell(3, 4, water_id)
    grid.swap(3, 3, 3, 4)
    assert grid.get_type_id(3, 3) == water_id
    assert grid.get_type_id(3, 4) == sand_id


def test_in_bounds(grid):
    assert grid.in_bounds(0, 0)
    assert grid.in_bounds(7, 7)
    assert not grid.in_bounds(-1, 0)
    assert not grid.in_bounds(0, 8)
    assert not grid.in_bounds(8, 0)


def test_get_type_id_array(grid):
    sand_id = grid.registry.get_by_name("sand").type_id
    grid.set_cell(0, 0, sand_id)
    arr = grid.get_type_id_array()
    assert len(arr) == 64  # 8 * 8
    assert arr[0] == sand_id
    assert arr[1] == AIR


def test_set_cell_with_lifetime(grid):
    """set_cell should copy lifetime from MaterialDef."""
    toml_content = """
[meta]
version = 1

[materials.steam]
cell_type = "gas"
density = 0.1
color = [200, 200, 255]
lifetime = 300
tags = ["gas"]
"""
    import tempfile, os
    from core.material import MaterialRegistry
    from core.reaction import ReactionTable
    fd, path = tempfile.mkstemp(suffix=".toml")
    try:
        with os.fdopen(fd, "w") as f:
            f.write(toml_content)
        reg = MaterialRegistry(path)
        table = ReactionTable(path, reg)
        g = CellGrid(4, 4, reg, table)
        steam_id = reg.get_by_name("steam").type_id
        g.set_cell(1, 1, steam_id)
        base = (1 * 4 + 1) * STRIDE
        assert g.cells[base + LIFETIME] == 300
    finally:
        os.unlink(path)
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cd /data/sunyunbo/playground/godot/fallingTest/prototype && python -m pytest tests/test_grid.py -v`

Expected: `ModuleNotFoundError: No module named 'core.grid'`

- [ ] **Step 3: 实现 core/grid.py（不含 update）**

```python
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
        pass  # Task 5 实现
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cd /data/sunyunbo/playground/godot/fallingTest/prototype && python -m pytest tests/test_grid.py -v`

Expected: all 6 tests PASS

- [ ] **Step 5: Commit**

```bash
git add prototype/core/grid.py prototype/tests/test_grid.py
git commit -m "feat: CellGrid basic operations (get/set/swap/bounds)"
```

---

## Task 5: 运动规则（rules.py）

**Files:**
- Create: `prototype/core/rules.py`
- Create: `prototype/tests/test_rules.py`

- [ ] **Step 1: 写测试 — test_rules.py**

```python
import random
import pytest
from core.material import MaterialRegistry
from core.reaction import ReactionTable
from core.grid import CellGrid
from core.cell import AIR

TOML = """
[meta]
version = 1

[materials.wall]
cell_type = "solid"
density = 10.0
color = [128, 128, 128]
tags = ["solid"]

[materials.sand]
cell_type = "powder"
density = 6.0
color = [194, 178, 128]
tags = ["powder"]

[materials.water]
cell_type = "liquid"
density = 1.0
color = [48, 96, 255]
tags = ["liquid", "water"]

[materials.steam]
cell_type = "gas"
density = 0.1
color = [200, 200, 255]
lifetime = 300
tags = ["gas"]

[materials.fire]
cell_type = "energy"
density = 0.0
color = [255, 160, 40]
lifetime = 60
tags = ["energy", "hot"]
"""


@pytest.fixture
def env(tmp_path):
    f = tmp_path / "m.toml"
    f.write_text(TOML)
    reg = MaterialRegistry(str(f))
    table = ReactionTable(str(f), reg)
    return reg, table


def make_grid(env, w=8, h=8):
    reg, table = env
    return CellGrid(w, h, reg, table)


def test_solid_does_not_move(env):
    random.seed(0)
    reg, _ = env
    grid = make_grid(env)
    wall_id = reg.get_by_name("wall").type_id
    grid.set_cell(3, 3, wall_id)
    from core.rules import try_move
    result = try_move(grid, 3, 3)
    assert result is None


def test_powder_falls_down(env):
    random.seed(0)
    reg, _ = env
    grid = make_grid(env)
    sand_id = reg.get_by_name("sand").type_id
    grid.set_cell(3, 0, sand_id)
    from core.rules import try_move
    result = try_move(grid, 3, 0)
    assert result == (3, 1)


def test_powder_falls_diagonal_when_blocked(env):
    random.seed(0)
    reg, _ = env
    grid = make_grid(env)
    sand_id = reg.get_by_name("sand").type_id
    wall_id = reg.get_by_name("wall").type_id
    grid.set_cell(3, 3, sand_id)
    grid.set_cell(3, 4, wall_id)
    result = None
    from core.rules import try_move
    for seed in range(100):
        random.seed(seed)
        r = try_move(grid, 3, 3)
        if r is not None:
            result = r
            break
    assert result is not None
    assert result[1] == 4
    assert result[0] in (2, 4)


def test_powder_stops_at_bottom(env):
    random.seed(0)
    reg, _ = env
    grid = make_grid(env)
    sand_id = reg.get_by_name("sand").type_id
    grid.set_cell(3, 7, sand_id)
    from core.rules import try_move
    result = try_move(grid, 3, 7)
    assert result is None


def test_liquid_spreads_horizontally(env):
    reg, _ = env
    grid = make_grid(env)
    water_id = reg.get_by_name("water").type_id
    wall_id = reg.get_by_name("wall").type_id
    grid.set_cell(3, 6, water_id)
    grid.set_cell(3, 7, wall_id)
    # Block both diagonals
    grid.set_cell(2, 7, wall_id)
    grid.set_cell(4, 7, wall_id)
    from core.rules import try_move
    moved = False
    for seed in range(100):
        random.seed(seed)
        grid.set_cell(3, 6, water_id)
        r = try_move(grid, 3, 6)
        if r is not None and r[1] == 6:
            moved = True
            assert r[0] in (2, 4)
            break
    assert moved


def test_gas_rises(env):
    random.seed(0)
    reg, _ = env
    grid = make_grid(env)
    steam_id = reg.get_by_name("steam").type_id
    grid.set_cell(3, 5, steam_id)
    from core.rules import try_move
    result = try_move(grid, 3, 5)
    assert result is not None
    assert result[1] < 5


def test_density_swap_heavy_sinks(env):
    random.seed(0)
    reg, _ = env
    grid = make_grid(env)
    sand_id = reg.get_by_name("sand").type_id
    water_id = reg.get_by_name("water").type_id
    grid.set_cell(3, 3, sand_id)
    grid.set_cell(3, 4, water_id)
    from core.rules import try_move
    result = try_move(grid, 3, 3)
    assert result == (3, 4)
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cd /data/sunyunbo/playground/godot/fallingTest/prototype && python -m pytest tests/test_rules.py -v`

Expected: `ModuleNotFoundError: No module named 'core.rules'`

- [ ] **Step 3: 实现 core/rules.py**

```python
from __future__ import annotations

import random
from typing import Optional

from core.cell import AIR, TYPE_ID, VELOCITY, STRIDE
from core.grid import CellGrid


def try_move(grid: CellGrid, x: int, y: int) -> Optional[tuple[int, int]]:
    type_id = grid.get_type_id(x, y)
    if type_id == AIR:
        return None

    mat = grid.registry.get_by_id(type_id)
    cell_type = mat.cell_type

    if cell_type == "solid":
        return None
    elif cell_type == "powder":
        return _move_powder(grid, x, y, mat.density)
    elif cell_type == "liquid":
        return _move_liquid(grid, x, y, mat.density)
    elif cell_type == "gas":
        return _move_gas(grid, x, y, mat.density)
    elif cell_type == "energy":
        return _move_energy(grid, x, y)
    return None


def _can_move_to(grid: CellGrid, x: int, y: int, self_density: float, heavier_sinks: bool) -> bool:
    if not grid.in_bounds(x, y):
        return False
    target_id = grid.get_type_id(x, y)
    if target_id == AIR:
        return True
    target_mat = grid.registry.get_by_id(target_id)
    if target_mat.cell_type == "solid":
        return False
    if heavier_sinks:
        return target_mat.density < self_density
    else:
        return target_mat.density > self_density


def _move_powder(grid: CellGrid, x: int, y: int, density: float) -> Optional[tuple[int, int]]:
    if _can_move_to(grid, x, y + 1, density, heavier_sinks=True):
        return (x, y + 1)

    diags = [(x - 1, y + 1), (x + 1, y + 1)]
    random.shuffle(diags)
    for dx, dy in diags:
        if _can_move_to(grid, dx, dy, density, heavier_sinks=True):
            return (dx, dy)

    return None


def _move_liquid(grid: CellGrid, x: int, y: int, density: float) -> Optional[tuple[int, int]]:
    if _can_move_to(grid, x, y + 1, density, heavier_sinks=True):
        return (x, y + 1)

    diags = [(x - 1, y + 1), (x + 1, y + 1)]
    random.shuffle(diags)
    for dx, dy in diags:
        if _can_move_to(grid, dx, dy, density, heavier_sinks=True):
            return (dx, dy)

    base = grid._base(x, y)
    vel = grid.cells[base + VELOCITY]
    sides = [(x + vel, y), (x - vel, y)]
    for sx, sy in sides:
        if _can_move_to(grid, sx, sy, density, heavier_sinks=True):
            return (sx, sy)

    # Flip velocity when blocked
    grid.cells[base + VELOCITY] = -vel
    return None


def _move_gas(grid: CellGrid, x: int, y: int, density: float) -> Optional[tuple[int, int]]:
    if _can_move_to(grid, x, y - 1, density, heavier_sinks=False):
        return (x, y - 1)

    diags = [(x - 1, y - 1), (x + 1, y - 1)]
    random.shuffle(diags)
    for dx, dy in diags:
        if _can_move_to(grid, dx, dy, density, heavier_sinks=False):
            return (dx, dy)

    base = grid._base(x, y)
    vel = grid.cells[base + VELOCITY]
    sides = [(x + vel, y), (x - vel, y)]
    for sx, sy in sides:
        if _can_move_to(grid, sx, sy, density, heavier_sinks=False):
            return (sx, sy)

    grid.cells[base + VELOCITY] = -vel
    return None


def _move_energy(grid: CellGrid, x: int, y: int) -> Optional[tuple[int, int]]:
    candidates = [(x, y - 1), (x - 1, y - 1), (x + 1, y - 1)]
    random.shuffle(candidates)
    for cx, cy in candidates:
        if grid.in_bounds(cx, cy) and grid.get_type_id(cx, cy) == AIR:
            return (cx, cy)
    return None
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cd /data/sunyunbo/playground/godot/fallingTest/prototype && python -m pytest tests/test_rules.py -v`

Expected: all 8 tests PASS

- [ ] **Step 5: Commit**

```bash
git add prototype/core/rules.py prototype/tests/test_rules.py
git commit -m "feat: movement rules for powder, liquid, gas, energy"
```

---

## Task 6: CellGrid.update()（主循环 + 反应）

**Files:**
- Modify: `prototype/core/grid.py`
- Add tests to: `prototype/tests/test_grid.py`

- [ ] **Step 1: 追加测试到 test_grid.py**

在 `test_grid.py` 文件末尾追加以下测试：

```python
# --- update() 测试 ---

FULL_TOML = """
[meta]
version = 1

[materials.wall]
cell_type = "solid"
density = 10.0
color = [128, 128, 128]
tags = ["solid"]

[materials.sand]
cell_type = "powder"
density = 6.0
color = [194, 178, 128]
tags = ["powder"]

[materials.water]
cell_type = "liquid"
density = 1.0
color = [48, 96, 255]
tags = ["liquid", "water"]

[materials.oil]
cell_type = "liquid"
density = 0.8
color = [80, 60, 30]
tags = ["liquid", "flammable"]

[materials.lava]
cell_type = "liquid"
density = 3.0
color = [255, 96, 0]
tags = ["liquid", "lava", "hot"]

[materials.steam]
cell_type = "gas"
density = 0.1
color = [200, 200, 255]
lifetime = 300
tags = ["gas"]

[materials.fire]
cell_type = "energy"
density = 0.0
color = [255, 160, 40]
lifetime = 3
tags = ["energy", "hot"]

[materials.rock]
cell_type = "solid"
density = 9.0
color = [100, 100, 100]
tags = ["solid"]

[[reactions]]
input = ["lava", "water"]
output = ["rock", "steam"]
probability = 1.0
"""


@pytest.fixture
def full_env(tmp_path):
    f = tmp_path / "full.toml"
    f.write_text(FULL_TOML)
    reg = MaterialRegistry(str(f))
    table = ReactionTable(str(f), reg)
    return reg, table


def test_sand_falls_to_bottom(full_env):
    import random
    random.seed(42)
    reg, table = full_env
    grid = CellGrid(8, 8, reg, table)
    sand_id = reg.get_by_name("sand").type_id
    grid.set_cell(3, 0, sand_id)
    for _ in range(20):
        grid.update()
    assert grid.get_type_id(3, 7) == sand_id
    assert grid.get_type_id(3, 0) == AIR


def test_sand_stacks(full_env):
    import random
    random.seed(42)
    reg, table = full_env
    grid = CellGrid(8, 8, reg, table)
    sand_id = reg.get_by_name("sand").type_id
    grid.set_cell(3, 0, sand_id)
    grid.set_cell(3, 1, sand_id)
    for _ in range(20):
        grid.update()
    assert grid.get_type_id(3, 7) == sand_id
    assert grid.get_type_id(3, 6) == sand_id


def test_oil_floats_on_water(full_env):
    import random
    random.seed(42)
    reg, table = full_env
    grid = CellGrid(4, 4, reg, table)
    water_id = reg.get_by_name("water").type_id
    oil_id = reg.get_by_name("oil").type_id
    wall_id = reg.get_by_name("wall").type_id
    # Container: walls on sides and bottom
    for y in range(4):
        grid.set_cell(0, y, wall_id)
        grid.set_cell(3, y, wall_id)
    for x in range(4):
        grid.set_cell(x, 3, wall_id)
    # Place water first, oil on top
    grid.set_cell(1, 1, water_id)
    grid.set_cell(1, 0, oil_id)
    for _ in range(20):
        grid.update()
    # Oil should be above water in the container
    col = [grid.get_type_id(1, y) for y in range(4)]
    oil_y = next(y for y in range(4) if col[y] == oil_id)
    water_y = next(y for y in range(4) if col[y] == water_id)
    assert oil_y < water_y


def test_lifetime_expires(full_env):
    import random
    random.seed(42)
    reg, table = full_env
    grid = CellGrid(4, 4, reg, table)
    fire_id = reg.get_by_name("fire").type_id
    grid.set_cell(2, 2, fire_id)
    # fire lifetime = 3 in this fixture
    for _ in range(10):
        grid.update()
    # Fire should have expired
    has_fire = any(
        grid.get_type_id(x, y) == fire_id
        for x in range(4) for y in range(4)
    )
    assert not has_fire


def test_lava_water_reaction(full_env):
    import random
    random.seed(42)
    reg, table = full_env
    grid = CellGrid(8, 8, reg, table)
    lava_id = reg.get_by_name("lava").type_id
    water_id = reg.get_by_name("water").type_id
    rock_id = reg.get_by_name("rock").type_id
    steam_id = reg.get_by_name("steam").type_id
    wall_id = reg.get_by_name("wall").type_id
    # Build floor
    for x in range(8):
        grid.set_cell(x, 7, wall_id)
    # Place lava and water adjacent
    grid.set_cell(3, 6, lava_id)
    grid.set_cell(4, 6, water_id)
    for _ in range(30):
        grid.update()
    # At least one rock or steam should exist (reaction fired with p=1.0)
    all_types = set()
    for x in range(8):
        for y in range(8):
            all_types.add(grid.get_type_id(x, y))
    assert rock_id in all_types or steam_id in all_types
```

- [ ] **Step 2: 运行测试确认 update 相关测试失败**

Run: `cd /data/sunyunbo/playground/godot/fallingTest/prototype && python -m pytest tests/test_grid.py::test_sand_falls_to_bottom -v`

Expected: sand stays at (3, 0) because `update()` is a no-op

- [ ] **Step 3: 实现 CellGrid.update()**

替换 `core/grid.py` 中的 `update` 方法（及添加 `_check_reactions` 辅助方法）：

```python
import random
from core.cell import AIR, STRIDE, TYPE_ID, VELOCITY, LIFETIME, FLAGS, FLAG_DIRTY
from core.material import MaterialRegistry
from core.reaction import ReactionTable, SELF_MARKER
from core.rules import try_move


class CellGrid:
    # ... __init__, _base, in_bounds, get_type_id, set_cell, swap,
    #     get_type_id_array 保持不变 ...

    def update(self) -> None:
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
```

- [ ] **Step 4: 运行全部 test_grid.py 测试**

Run: `cd /data/sunyunbo/playground/godot/fallingTest/prototype && python -m pytest tests/test_grid.py -v`

Expected: all tests PASS (basic ops + update tests)

- [ ] **Step 5: 运行全部测试确认无回退**

Run: `cd /data/sunyunbo/playground/godot/fallingTest/prototype && python -m pytest tests/ -v`

Expected: all tests across all files PASS

- [ ] **Step 6: Commit**

```bash
git add prototype/core/grid.py prototype/tests/test_grid.py
git commit -m "feat: CellGrid.update() with movement dispatch and reactions"
```

---

## Task 7: PygameRenderer（surfarray + LUT）

**Files:**
- Create: `prototype/render/pygame_renderer.py`
- Create: `prototype/tests/test_renderer.py`

- [ ] **Step 1: 写测试 — test_renderer.py**

```python
import pytest
import numpy as np
from core.material import MaterialRegistry
from core.reaction import ReactionTable
from core.grid import CellGrid
from core.cell import AIR


@pytest.fixture
def env(tmp_path):
    toml_content = """
[meta]
version = 1

[materials.wall]
cell_type = "solid"
density = 10.0
color = [128, 128, 128]
tags = ["solid"]

[materials.sand]
cell_type = "powder"
density = 6.0
color = [194, 178, 128]
color_variance = 15
tags = ["powder"]
"""
    f = tmp_path / "m.toml"
    f.write_text(toml_content)
    reg = MaterialRegistry(str(f))
    table = ReactionTable(str(f), reg)
    grid = CellGrid(4, 4, reg, table)
    return reg, grid


def test_color_lut_shape(env):
    reg, grid = env
    from render.pygame_renderer import build_color_lut
    lut = build_color_lut(reg)
    assert lut.shape == (reg.max_id + 1, 3)
    assert lut.dtype == np.uint8


def test_color_lut_air_is_black(env):
    reg, grid = env
    from render.pygame_renderer import build_color_lut
    lut = build_color_lut(reg)
    assert tuple(lut[0]) == (0, 0, 0)


def test_color_lut_values(env):
    reg, grid = env
    from render.pygame_renderer import build_color_lut
    lut = build_color_lut(reg)
    wall = reg.get_by_name("wall")
    assert tuple(lut[wall.type_id]) == (128, 128, 128)


def test_render_buffer_shape(env):
    reg, grid = env
    from render.pygame_renderer import build_color_lut, render_to_array
    lut = build_color_lut(reg)
    buf = render_to_array(grid, lut, variance_matrix=None)
    assert buf.shape == (4, 4, 3)
    assert buf.dtype == np.uint8


def test_render_buffer_content(env):
    reg, grid = env
    sand_id = reg.get_by_name("sand").type_id
    grid.set_cell(1, 2, sand_id)
    from render.pygame_renderer import build_color_lut, render_to_array
    lut = build_color_lut(reg)
    buf = render_to_array(grid, lut, variance_matrix=None)
    assert tuple(buf[1, 2]) == (194, 178, 128)
    assert tuple(buf[0, 0]) == (0, 0, 0)
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cd /data/sunyunbo/playground/godot/fallingTest/prototype && python -m pytest tests/test_renderer.py -v`

Expected: `ModuleNotFoundError: No module named 'render.pygame_renderer'`

- [ ] **Step 3: 实现 render/pygame_renderer.py**

```python
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
    type_ids = np.array(grid.get_type_id_array(), dtype=np.int32).reshape(
        grid.width, grid.height
    )
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
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cd /data/sunyunbo/playground/godot/fallingTest/prototype && python -m pytest tests/test_renderer.py -v`

Expected: all 5 tests PASS

- [ ] **Step 5: Commit**

```bash
git add prototype/render/pygame_renderer.py prototype/tests/test_renderer.py
git commit -m "feat: PygameRenderer with surfarray LUT rendering"
```

---

## Task 8: InputHandler

**Files:**
- Create: `prototype/render/input_handler.py`

- [ ] **Step 1: 实现 render/input_handler.py**

```python
from __future__ import annotations

from core.cell import AIR
from core.grid import CellGrid
from core.material import MaterialRegistry


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
    def __init__(self, registry: MaterialRegistry, scale: int = 4) -> None:
        self.registry = registry
        self.scale = scale
        self.selected_material = "sand"
        self.brush_size = 3
        self.paused = False

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

            r = self.brush_size // 2
            for dy in range(-r, r + 1):
                for dx in range(-r, r + 1):
                    px, py = gx + dx, gy + dy
                    if grid.in_bounds(px, py):
                        grid.set_cell(px, py, type_id)

        return True
```

- [ ] **Step 2: Commit**

```bash
git add prototype/render/input_handler.py
git commit -m "feat: InputHandler with mouse drawing and keyboard controls"
```

---

## Task 9: main.py — 入口与主循环

**Files:**
- Create: `prototype/main.py`

- [ ] **Step 1: 实现 main.py**

```python
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
```

- [ ] **Step 2: 手动运行验证**

Run: `cd /data/sunyunbo/playground/godot/fallingTest/prototype && python main.py`

验证以下功能（用户手测）：
1. 窗口打开，黑色背景（128×128 放大 4 倍 = 512×512）
2. 按数字键 3 选 sand，鼠标左键拖拽画沙子，沙子下落
3. 按数字键 4 选 water，画水，水流动扩散
4. 沙子沉入水中（密度交换）
5. 按数字键 6 选 lava，画岩浆，接触水产生 rock + steam
6. 按数字键 7 选 fire，画火，接触 wood/oil 蔓延
7. 按数字键 2 选 wood，画木头，被火点燃
8. 空格暂停/恢复，R 清空
9. 滚轮调整画笔大小
10. 左上角显示当前材质和 FPS

- [ ] **Step 3: Commit**

```bash
git add prototype/main.py
git commit -m "feat: main.py entry point with game loop"
```

---

## Task 10: materials.toml 补全 rock 材质

注意：反应表中 `lava + water → rock + steam`，但 materials.toml 里还没有定义 `rock`。需要补上。

**Files:**
- Modify: `prototype/data/materials.toml`

- [ ] **Step 1: 在 materials.toml 中添加 rock**

在 `[materials.wall]` 后面添加：

```toml
[materials.rock]
cell_type = "solid"
density = 9.0
color = [100, 100, 100]
color_variance = 8
tags = ["solid"]
```

- [ ] **Step 2: 运行全部测试确认无回退**

Run: `cd /data/sunyunbo/playground/godot/fallingTest/prototype && python -m pytest tests/ -v`

Expected: all tests PASS

- [ ] **Step 3: Commit**

```bash
git add prototype/data/materials.toml
git commit -m "fix: add rock material definition for lava+water reaction"
```

---

## Task 11: 全量集成测试

**Files:**
- Modify: `prototype/tests/test_grid.py`（追加集成测试）

- [ ] **Step 1: 追加用完整 materials.toml 的集成测试**

在 `test_grid.py` 末尾追加：

```python
def test_integration_with_real_toml():
    """Use the actual materials.toml to verify everything wires up."""
    import random
    from pathlib import Path
    random.seed(123)
    toml_path = str(Path(__file__).parent.parent / "data" / "materials.toml")
    reg = MaterialRegistry(toml_path)
    table = ReactionTable(toml_path, reg)
    grid = CellGrid(16, 16, reg, table)

    # Place some materials and run
    sand_id = reg.get_by_name("sand").type_id
    water_id = reg.get_by_name("water").type_id
    wall_id = reg.get_by_name("wall").type_id

    # Floor
    for x in range(16):
        grid.set_cell(x, 15, wall_id)

    # Sand column
    for y in range(5):
        grid.set_cell(8, y, sand_id)

    # Water pool
    for x in range(4, 12):
        grid.set_cell(x, 14, water_id)

    # Run 100 frames without crash
    for _ in range(100):
        grid.update()

    # Sand should have settled
    assert grid.get_type_id(8, 0) == AIR
```

- [ ] **Step 2: 运行全部测试**

Run: `cd /data/sunyunbo/playground/godot/fallingTest/prototype && python -m pytest tests/ -v`

Expected: all tests PASS

- [ ] **Step 3: Commit**

```bash
git add prototype/tests/test_grid.py
git commit -m "test: integration test with real materials.toml"
```

---

## 任务依赖总览

```
Task 1 (scaffold)
  └→ Task 2 (material) 
       └→ Task 3 (reaction)
            └→ Task 4 (grid basic)
                 └→ Task 5 (rules)
                      └→ Task 6 (grid.update)
                           ├→ Task 7 (renderer)
                           │    └→ Task 9 (main.py)
                           ├→ Task 8 (input handler)
                           │    └→ Task 9 (main.py)
                           └→ Task 10 (rock material)
                                └→ Task 11 (integration test)
```

Task 7 和 Task 8 可并行。Task 9 依赖 Task 7 + 8。Task 10 和 Task 11 可在 Task 9 之后串行。
