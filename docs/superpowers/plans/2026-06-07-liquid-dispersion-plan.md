# 液体/气体 Dispersion Rate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 液体/气体横移一帧沿方向记忆探测最多 `dispersion` 格、落到最远连续 AIR，治"水流慢渗"观感。

**Architecture:** 材质新增整数字段 `dispersion`（缺省 1 = 现行为）；`rules.py` 横移段替换为共享的最远空格探测 helper（液体/气体镜像复用）；探测纯确定无 RNG，在 `write_rect` 边界截断；`grid.py` 调度/世代戳零改动。Spec：`docs/superpowers/specs/2026-06-07-liquid-dispersion-design.md`。

**Tech Stack:** Python 3 + pytest（venv：`venv/bin/python`，测试从 `prototype/` 目录跑）。

---

### Task 1: MaterialDef.dispersion 字段

**Files:**
- Modify: `prototype/core/material.py`
- Modify: `prototype/data/materials.toml`
- Test: `prototype/tests/test_materials.py`

- [ ] **Step 1: 写失败测试**

在 `prototype/tests/test_materials.py` 的 fixture TOML 中给 water 加 `dispersion = 5`（保持 float density 不动），文件末尾追加：

```python
def test_dispersion_loaded(registry):
    water = registry.get_by_name("water")
    assert water.dispersion == 5
    assert isinstance(water.dispersion, int)


def test_default_dispersion(registry):
    """未声明 dispersion 的材质缺省 1（= 现行为）。"""
    wall = registry.get_by_name("wall")
    assert wall.dispersion == 1
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd prototype && ../venv/bin/python -m pytest tests/test_materials.py -q`
Expected: 2 failed —— `AttributeError: 'MaterialDef' object has no attribute 'dispersion'`

- [ ] **Step 3: 最小实现**

`prototype/core/material.py` 两处：

```python
@dataclass(frozen=True)
class MaterialDef:
    name: str
    type_id: int
    cell_type: str
    density: int  # 整数密度等级（确定性契约 D1；标度 ≈ 旧 float ×10）
    color: tuple[int, int, int]
    color_variance: int
    lifetime: int
    tags: frozenset[str]
    dispersion: int = 1  # 横移一帧最多探测格数（液体/气体；spec 2026-06-07）
```

registry 加载处（`mat = MaterialDef(...)` 调用）追加一行参数：

```python
                lifetime=props.get("lifetime", 0),
                tags=tags,
                dispersion=int(props.get("dispersion", 1)),
```

`prototype/data/materials.toml` 四处材质各加一行 `dispersion = N`：water 5、oil 2、lava 1、steam 3（其余材质不写，吃缺省 1）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cd prototype && ../venv/bin/python -m pytest tests/test_materials.py -q`
Expected: 全部 PASS

- [ ] **Step 5: Commit**

```bash
git add prototype/core/material.py prototype/data/materials.toml prototype/tests/test_materials.py
git commit -m "feat(dispersion): MaterialDef.dispersion field, default 1"
```

### Task 2: 液体最远空格探测

**Files:**
- Modify: `prototype/core/rules.py`
- Test: `prototype/tests/test_rules.py`

- [ ] **Step 1: 写失败测试**

`prototype/tests/test_rules.py` 顶部 fixture TOML 中给 water 加 `dispersion = 5`、steam 加 `dispersion = 3`。在 `test_density_swap_heavy_sinks` 前插入：

```python
def test_liquid_disperses_to_furthest_air(env):
    """水 dispersion=5：右侧 5 格全空 → 一帧落到 x+5。"""
    reg, table = env
    grid = CellGrid(16, 8, reg, table)
    water_id = reg.get_by_name("water").type_id
    wall_id = reg.get_by_name("wall").type_id
    for x in range(16):
        grid.set_cell(x, 7, wall_id)  # 地板：下/斜下全堵
    grid.set_cell(3, 6, water_id)     # vel 初始 +1
    from core.rules import try_move
    assert try_move(grid, 3, 6) == (8, 6)


def test_liquid_probe_stops_at_obstacle(env):
    """探测中途遇墙截停 → 落墙前最远空格。"""
    reg, table = env
    grid = CellGrid(16, 8, reg, table)
    water_id = reg.get_by_name("water").type_id
    wall_id = reg.get_by_name("wall").type_id
    for x in range(16):
        grid.set_cell(x, 7, wall_id)
    grid.set_cell(6, 6, wall_id)      # x+3 是墙
    grid.set_cell(3, 6, water_id)
    from core.rules import try_move
    assert try_move(grid, 3, 6) == (5, 6)


def test_liquid_displacement_only_at_first_cell(env):
    """首格是更轻液体 → 走 ±1 密度置换，不穿透。"""
    reg, table = env
    grid = CellGrid(16, 8, reg, table)
    water_id = reg.get_by_name("water").type_id
    oil_id = reg.get_by_name("oil").type_id
    wall_id = reg.get_by_name("wall").type_id
    for x in range(16):
        grid.set_cell(x, 7, wall_id)
    grid.set_cell(4, 6, oil_id)       # 右首格：油（密度 8 < 10）
    grid.set_cell(3, 6, water_id)
    from core.rules import try_move
    assert try_move(grid, 3, 6) == (4, 6)  # 置换油，而非越过油落远处


def test_liquid_probe_respects_write_rect(env):
    """写域契约直测：探测在 write_rect 边界截断。"""
    from core.chunks import Rect
    reg, table = env
    grid = CellGrid(16, 8, reg, table)
    water_id = reg.get_by_name("water").type_id
    wall_id = reg.get_by_name("wall").type_id
    for x in range(16):
        grid.set_cell(x, 7, wall_id)
    grid.set_cell(3, 6, water_id)
    grid._write_rect = Rect(0, 0, 6, 8)  # 人为收窄：x ≥ 6 不可写
    from core.rules import try_move
    assert try_move(grid, 3, 6) == (5, 6)  # 截断在域内最远空格
```

注意：fixture TOML 目前没有 oil——在 fixture 的 water 块后面加：

```toml
[materials.oil]
cell_type = "liquid"
density = 8
dispersion = 2
color = [80, 60, 30]
tags = ["liquid", "flammable"]
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd prototype && ../venv/bin/python -m pytest tests/test_rules.py -q`
Expected: 新增 4 个中至少 `furthest_air`、`stops_at_obstacle`、`respects_write_rect` FAIL（现实现只横移 1 格）；`displacement_only_at_first_cell` 可能 PASS（现行为本就置换）——确认其余 3 个红即可。

- [ ] **Step 3: 实现共享探测 helper + 液体接线**

`prototype/core/rules.py`：在 `_can_move_to` 之后新增 helper；`_move_liquid` 横移段替换、签名改收 `mat`；`try_move` 调用处同步：

```python
def _probe_side(grid, x, y, density, dispersion, heavier_sinks):
    """沿方向记忆探测最多 dispersion 格，落最远连续 AIR；
    首格可密度置换则走旧 ±1 路径（spec §2，2026-06-07）。
    纯确定（无 RNG）；在 write_rect 边界截断（写域契约）。"""
    base = grid._base(x, y)
    vel = grid.cells[base + VELOCITY]
    for direction in (vel, -vel):
        furthest = None
        for i in range(1, dispersion + 1):
            tx = x + direction * i
            if not grid._write_rect.contains(tx, y):
                break
            target_id = grid.get_type_id(tx, y)
            if target_id == AIR:
                furthest = (tx, y)
                continue
            if i == 1 and _can_move_to(grid, tx, y, density, heavier_sinks=heavier_sinks):
                return (tx, y)
            break
        if furthest is not None:
            if direction == -vel:
                grid.cells[base + VELOCITY] = -vel  # 方向承诺（2026-06-07 修复同款）
            return furthest
    grid.cells[base + VELOCITY] = -vel
    return None
```

`try_move` 中：

```python
    elif cell_type == "liquid":
        return _move_liquid(grid, x, y, mat)
```

`_move_liquid` 整体替换为：

```python
def _move_liquid(grid: CellGrid, x: int, y: int, mat) -> Optional[tuple[int, int]]:
    density = mat.density
    if _can_move_to(grid, x, y + 1, density, heavier_sinks=True):
        return (x, y + 1)

    diags = ((x - 1, y + 1), (x + 1, y + 1))
    for i in order2(grid._fseed, grid._pass_id, x, y, SALT_DIAG):
        dx, dy = diags[i]
        if _can_move_to(grid, dx, dy, density, heavier_sinks=True):
            return (dx, dy)

    return _probe_side(grid, x, y, density, mat.dispersion, heavier_sinks=True)
```

（注意删除 `_move_liquid` 原有的方向承诺注释块——逻辑整体移入 `_probe_side`。）

- [ ] **Step 4: 跑测试确认通过**

Run: `cd prototype && ../venv/bin/python -m pytest tests/test_rules.py -q`
Expected: 全部 PASS（含今天的方向承诺/摊平测试——`_probe_side` 保持同语义）

- [ ] **Step 5: Commit**

```bash
git add prototype/core/rules.py prototype/tests/test_rules.py
git commit -m "feat(dispersion): liquid furthest-air side probe (shared helper)"
```

### Task 3: 气体镜像接线

**Files:**
- Modify: `prototype/core/rules.py`
- Test: `prototype/tests/test_rules.py`

- [ ] **Step 1: 写失败测试**

`prototype/tests/test_rules.py` 追加：

```python
def test_gas_disperses_to_furthest_air(env):
    """steam dispersion=3：天花板下右侧 3 格空 → 一帧落 x+3。"""
    reg, table = env
    grid = CellGrid(16, 8, reg, table)
    steam_id = reg.get_by_name("steam").type_id
    wall_id = reg.get_by_name("wall").type_id
    for x in range(16):
        grid.set_cell(x, 0, wall_id)  # 天花板：上/斜上全堵
    grid.set_cell(3, 1, steam_id)
    from core.rules import try_move
    assert try_move(grid, 3, 1) == (6, 1)
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd prototype && ../venv/bin/python -m pytest tests/test_rules.py::test_gas_disperses_to_furthest_air -q`
Expected: FAIL（气体现仍是 ±1 横移）

- [ ] **Step 3: 实现**

`try_move` 中：

```python
    elif cell_type == "gas":
        return _move_gas(grid, x, y, mat)
```

`_move_gas` 整体替换为：

```python
def _move_gas(grid: CellGrid, x: int, y: int, mat) -> Optional[tuple[int, int]]:
    density = mat.density
    if _can_move_to(grid, x, y - 1, density, heavier_sinks=False):
        return (x, y - 1)

    diags = ((x - 1, y - 1), (x + 1, y - 1))
    for i in order2(grid._fseed, grid._pass_id, x, y, SALT_DIAG):
        dx, dy = diags[i]
        if _can_move_to(grid, dx, dy, density, heavier_sinks=False):
            return (dx, dy)

    return _probe_side(grid, x, y, density, mat.dispersion, heavier_sinks=False)
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd prototype && ../venv/bin/python -m pytest tests/test_rules.py -q`
Expected: 全部 PASS

- [ ] **Step 5: Commit**

```bash
git add prototype/core/rules.py prototype/tests/test_rules.py
git commit -m "feat(dispersion): gas mirror wiring via shared probe"
```

### Task 4: 摊平加速验收 + 全量回归

**Files:**
- Modify: `prototype/tests/test_rules.py:test_liquid_levels_out`

- [ ] **Step 1: 收紧摊平测试帧数**

`test_liquid_levels_out` 中 `for _ in range(800):` 改为 `for _ in range(200):`，
断言消息同步改 `f"液面未摊平（200 帧，dispersion=5）：heights={heights}"`。

- [ ] **Step 2: 跑测试确认通过（200 帧足够收敛）**

Run: `cd prototype && ../venv/bin/python -m pytest tests/test_rules.py::test_liquid_levels_out -q`
Expected: PASS。若 FAIL：把帧数回调到最小可过值（150/200/250 试探），落账实测值。

- [ ] **Step 3: 全量回归**

Run: `cd prototype && ../venv/bin/python -m pytest tests/ -q`
Expected: 全部 PASS（replay/同 seed 等价测试自动覆盖语义变更；若有金值 hash 断言失败，按"语义变更作废"口径重钉——预期没有）

- [ ] **Step 4: Commit**

```bash
git add prototype/tests/test_rules.py
git commit -m "test(dispersion): leveling converges 4x faster (800->200 frames)"
```

### Task 5: Benchmark + demo 验收 + 落账

**Files:**
- Modify: `docs/CHANGELOG.md`、`docs/perf/baseline.md`

- [ ] **Step 1: 跑双尺寸 benchmark**

Run: `cd prototype && ../venv/bin/python benchmark.py`
Expected: 与基线 27.2/13.9 FPS 对比，回退 ≤ 10%（spec §5 预算）。超预算 → 停下分析（探测循环只应在下落失败的表面像素跑），不许静默接受。

- [ ] **Step 2: 重新生成 demo gif（用户目测验收）**

Run: `cd prototype && ../venv/bin/python demo_gif.py && ../venv/bin/python demo_density.py`
Expected: 水"流"起来（摊平肉眼变快）、岩浆（dispersion 1）明显更稠。

- [ ] **Step 3: CHANGELOG + baseline 落账**

`docs/CHANGELOG.md` 2026-06-07 块 Added 追加（实测数字替换占位）：

```markdown
- **液体/气体 dispersion rate**（spec `docs/superpowers/specs/2026-06-07-liquid-dispersion-design.md`）：
  材质字段 `dispersion`（water 5/oil 2/lava 1/steam 3，缺省 1），横移一帧落最远连续 AIR，
  首格保留 ±1 密度置换；探测纯确定、写域夹断。摊平收敛 800→200 帧；
  benchmark <实测> FPS（基线 27.2/13.9，回退 <实测>%）。hash 序列作废（语义变更）。
  `prototype/core/rules.py`（`_probe_side`）+ 9 个新测试。
```

`docs/perf/baseline.md` 里程碑表追加 dispersion 行（同实测数字）。

- [ ] **Step 4: Commit**

```bash
git add docs/CHANGELOG.md docs/perf/baseline.md
git commit -m "docs(dispersion): changelog + perf ledger"
```

---

## Self-Review 记录

- Spec 覆盖：§1→Task 1；§2→Task 2/3；§3 写域→Task 2 直测；§4 测试 1-5→Task 2/3、6→Task 4、7→Task 4 Step 3；§5→Task 5。无缺口。
- 类型一致：`_probe_side(grid, x, y, density, dispersion, heavier_sinks)` 在 Task 2 定义、Task 3 复用，签名一致；`_move_liquid/_move_gas` 均改收 `mat`。
- 占位符：CHANGELOG 模板中 `<实测>` 为有意的待填实测值（Step 1/2 产出），非 TBD。
