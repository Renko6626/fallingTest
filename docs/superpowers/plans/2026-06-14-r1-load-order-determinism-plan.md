# R1 加载顺序确定性 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 消除 `material.py` 中 type_id 按 toml 声明顺序分配的跨平台确定性隐患——改为按材质 name 排序分配，并用红绿测试锁死契约。

**Architecture:** 仅改加载期一处（`MaterialRegistry.__init__` 的材质遍历），sim 热路径零改动。type_id 改为 `sorted(material names)` 顺序分配，与 toml 解析/C# Dictionary 枚举顺序解耦。新增 `test_load_order.py` 把"加载顺序无关"变成红绿契约。

**Tech Stack:** Python 3 + pytest（venv：`venv/bin/python`，测试从 `prototype/` 目录跑）。

## Global Constraints

- 来源 proposal：`docs/proposals/2026-06-14-determinism-hardening-r1-r3.md` Part 1（A2 已砍，只做 A1+A3）。
- 改动改变 type_id 分配 → **既往 state_hash 序列作废**（语义等价变更，与历史同口径；录放/同 seed 等价测试不锚死具体 type_id 值，不受影响）。
- 测试从 `prototype/` 目录跑：`cd prototype && ../venv/bin/python -m pytest ...`。
- 禁止在 subagent 内调 Godot/godot CLI；只做静态写 + pytest + commit。

---

### Task 1: type_id 按 name 排序分配

**Files:**
- Modify: `prototype/core/material.py:42-43`
- Test: `prototype/tests/test_materials.py`

**Interfaces:**
- Consumes: 无（独立）。
- Produces: `MaterialRegistry` 的 type_id 分配顺序契约——type_id 1,2,3… 按 `sorted(material names)` 升序，供 Task 2 的 capstone 测试依赖。

- [ ] **Step 1: 写失败测试**

在 `prototype/tests/test_materials.py` 末尾追加（自带一个非字母序声明的 fixture，确保红绿）：

```python
def test_type_id_assigned_by_sorted_name(tmp_path):
    """type_id 按材质 name 排序分配，与 toml 声明顺序无关（R1 / D3）。
    fixture 故意非字母序声明：zebra 在前、alpha 在后。"""
    toml_content = """
[meta]
version = 1

[materials.zebra]
cell_type = "solid"
density = 50
color = [1, 1, 1]

[materials.alpha]
cell_type = "solid"
density = 50
color = [2, 2, 2]
"""
    f = tmp_path / "order.toml"
    f.write_text(toml_content)
    reg = MaterialRegistry(str(f))
    # 按 name 排序：alpha 先得 1，zebra 得 2（与声明顺序相反）
    assert reg.get_by_name("alpha").type_id == 1
    assert reg.get_by_name("zebra").type_id == 2
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd prototype && ../venv/bin/python -m pytest tests/test_materials.py::test_type_id_assigned_by_sorted_name -q`
Expected: FAIL —— 现按声明序分配，zebra 得 1、alpha 得 2，`assert reg.get_by_name("alpha").type_id == 1` 失败（实际为 2）。

- [ ] **Step 3: 最小实现**

`prototype/core/material.py`，把 `next_id` 循环改为按排序后的 name 遍历。原代码（42-43 行附近）：

```python
        next_id = 1
        for name, props in data.get("materials", {}).items():
            color_raw = props["color"]
```

改为：

```python
        next_id = 1
        materials = data.get("materials", {})
        for name in sorted(materials.keys()):  # R1/D3：按 name 排序，解耦 toml/Dict 枚举序
            props = materials[name]
            color_raw = props["color"]
```

循环体其余部分（`tags = ...`、`mat = MaterialDef(...)`、`self._by_name[name] = mat` 等）保持不变。

- [ ] **Step 4: 跑测试确认通过 + 全量回归**

Run: `cd prototype && ../venv/bin/python -m pytest tests/ -q`
Expected: 全部 PASS（含新测试）。注意：本改动改变 type_id → 若有任何**绝对 state_hash 金值**断言会红——预期没有（`test_determinism.py` 用动态 `get_by_name(...).type_id` 且只比对同 run 内 hash 序列；`test_rng.py` 金值是 squirrel5 输出、与 type_id 无关）。若意外有金值红，停下报告，按"语义变更作废 hash"口径重钉。

- [ ] **Step 5: Commit**

```bash
git add prototype/core/material.py prototype/tests/test_materials.py
git commit -m "fix(determinism): assign type_id by sorted material name (R1)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task 2: D3 capstone 回归测试（真实 toml）

**Files:**
- Create: `prototype/tests/test_load_order.py`

**Interfaces:**
- Consumes: Task 1 的契约——`MaterialRegistry` type_id 按 sorted name 分配。
- Produces: 无（终端测试）。

- [ ] **Step 1: 写测试（Task 1 落地后应直接通过——这是 capstone 契约锁，非红绿）**

新建 `prototype/tests/test_load_order.py`：

```python
"""R1 / D3 契约：加载是数据内容的纯函数，与 toml 声明/容器枚举顺序无关。
（A2 反应表排序经核对为非 live bug 已砍，本文件只锁 type_id + hash 契约。）"""
from pathlib import Path

from core.material import MaterialRegistry
from core.reaction import ReactionTable
from core.grid import CellGrid

TOML = str(Path(__file__).parent.parent / "data" / "materials.toml")


def test_type_ids_follow_sorted_name_order():
    """真实 materials.toml：type_id 序列严格等于按 name 排序的顺序。
    materials.toml 声明序非字母序，故去掉 sorted 必红。"""
    reg = MaterialRegistry(TOML)
    names = sorted(m.name for m in reg.all() if m.name != "air")
    for expected_id, name in enumerate(names, start=1):
        assert reg.get_by_name(name).type_id == expected_id, (
            f"{name} 应得 type_id {expected_id}，实际 {reg.get_by_name(name).type_id}"
        )


def test_double_load_state_hash_identical():
    """同一文件两次独立加载 → 跑同一场景 N 帧 → state_hash 序列逐帧相等。
    （加载是纯函数的端到端验证。）"""
    def run(frames=30):
        reg = MaterialRegistry(TOML)
        table = ReactionTable(TOML, reg)
        grid = CellGrid(48, 48, reg, table, seed=7)
        wall = reg.get_by_name("wall").type_id
        sand = reg.get_by_name("sand").type_id
        water = reg.get_by_name("water").type_id
        for x in range(48):
            grid.set_cell(x, 47, wall)
        for x in range(10, 38):
            grid.set_cell(x, 5, sand)
            grid.set_cell(x, 10, water)
        hashes = []
        for _ in range(frames):
            grid.update()
            hashes.append(grid.state_hash())
        return hashes

    assert run() == run()
```

- [ ] **Step 2: 跑测试**

Run: `cd prototype && ../venv/bin/python -m pytest tests/test_load_order.py -q`
Expected: 2 PASS。
（若 `test_type_ids_follow_sorted_name_order` 红 → Task 1 未生效，回查。若 `test_double_load...` 红 → 加载非纯函数，停下报告——不应发生。）

- [ ] **Step 3: 全量回归**

Run: `cd prototype && ../venv/bin/python -m pytest tests/ -q`
Expected: 全部 PASS。

- [ ] **Step 4: Commit**

```bash
git add prototype/tests/test_load_order.py
git commit -m "test(determinism): D3 load-order capstone (sorted type_id + double-load hash) (R1)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review 记录

- **Spec 覆盖**：proposal Part 1 §1.2 方案 1（material sorted）→ Task 1；方案 2（单元红绿 + D3 capstone）→ Task 1 测试 + Task 2。A2 已按裁决砍掉，不入计划。R3（Part 2）是前瞻文档决策，不入计划。无缺口。
- **占位符扫描**：无 TBD/TODO；所有代码块完整。
- **类型一致**：Task 1 产出"type_id 按 sorted name"契约，Task 2 `test_type_ids_follow_sorted_name_order` 正是消费该契约；`MaterialRegistry`/`ReactionTable`/`CellGrid` 签名与现有代码一致（`CellGrid(w,h,reg,table,seed=)` 见 test_rules.py 用法）。
- **hash 作废提示**：Task 1 Step 4 已显式提示并给出"预期无金值红"的依据 + 异常处置。
