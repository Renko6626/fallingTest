> 文档路径：`docs/superpowers/specs/2026-05-26-python-prototype-design.md`
> 运行时版本：Python 3.11+
> 最近更新：2026-05-26 (UTC+8)

# Python 原型设计：Noita 风格像素物理引擎

## 1. 目标与范围

**Phase 1 首要目标**：验证核心元胞自动机（CA）算法的正确性。

| 做 | 不做 |
|---|---|
| CA 更新循环 + 4 套运动规则 | chunk 分块、dirty rect、多线程优化 |
| 数据驱动的材质表 + 标签系统 + 反应表 | 刚体物理、角色控制、UI 系统 |
| pygame 实时渲染 + 鼠标交互 | compute shader、GPU 加速 |
| pytest 单元测试 | 性能压测（留到材质体系稳定后） |

**网格规模**：128×128 起步，窗口 512×512（4× 放大）。

**材质范围**：首批 8 种——wall, wood, sand, water, oil, lava, steam, fire，覆盖 solid/powder/liquid/gas/energy 全部 5 类 cell_type。其中 wood 是可燃固体，用于验证"固体 + 标签触发反应"的组合。数据结构为任意数量材质留好扩展位。

---

## 2. 数据层

### 2.1 materials.toml

所有材质定义和反应规则集中在 `data/materials.toml` 中，代码不含任何材质特定逻辑。

借鉴 Noita 的 `data/materials.xml`（`<CellData>` + `<Reaction>`），但用 TOML 替代 XML：

```toml
[meta]
version = 1
default_grid_size = [128, 128]

# --- 材质定义 ---
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

# --- 反应表 ---
[[reactions]]
input = ["lava", "water"]
output = ["rock", "steam"]
probability = 0.8

[[reactions]]
input = ["fire", "[flammable]"]
output = ["fire", "fire"]
probability = 0.05

# fire 相邻 wood 时，wood 也可能自发点燃（低概率蔓延）
[[reactions]]
input = ["[hot]", "wood"]
output = ["_self", "fire"]
probability = 0.02

[[reactions]]
input = ["[hot]", "water"]
output = ["_self", "steam"]
probability = 0.5
```

**设计要点**：
- `cell_type` 五选一：`solid` / `powder` / `liquid` / `gas` / `energy`
- `tags` 列表：反应表可用 `[tag]` 语法按标签匹配，一条规则覆盖多种材质
- `color_variance`：渲染时每像素固定随机偏移，视觉更自然
- `lifetime`：仅 gas/energy 类需要，到期变为 air
- `_self` 关键字：反应输出中保持原材质不变
- `probability`：0.0–1.0 浮点数，每帧每次接触按概率触发

### 2.2 MaterialRegistry (`core/material.py`)

```python
@dataclass(frozen=True)
class MaterialDef:
    name: str
    type_id: int           # 运行时分配，0 = air
    cell_type: str         # solid / powder / liquid / gas / energy
    density: float
    color: tuple[int, int, int]
    color_variance: int    # 默认 0
    lifetime: int          # 默认 0（无限）
    tags: frozenset[str]
```

MaterialRegistry 职责：
- 启动时读 TOML，为每种材质分配 `type_id`（0 保留给 air）
- 构建 `name → MaterialDef` 和 `type_id → MaterialDef` 两个索引
- 构建 `tag → set[type_id]` 倒排索引，供 ReactionTable 展开标签用
- 提供 `get_by_name(name)`, `get_by_id(type_id)` 查询方法

### 2.3 ReactionTable (`core/reaction.py`)

```python
@dataclass(frozen=True)
class ReactionResult:
    output1: int       # type_id，-1 = _self（保持不变）
    output2: int
    probability: float
```

ReactionTable 职责：
- 读 TOML 的 `[[reactions]]`
- 展开标签：如 `[flammable]` → 查 registry 倒排索引拿到 `{oil_id}`，为每个具体材质组合生成条目
- 最终构建 `dict[tuple[int, int], list[ReactionResult]]`（同一对材质可能有多条反应）
- 查找 O(1)：`table.get((type_a, type_b))` 返回 `list[ReactionResult]` 或 None
- 对称性：`(a, b)` 和 `(b, a)` 都注册（输出对应翻转）

---

## 3. 模拟核心

### 3.1 像素存储 (`core/cell.py`)

不用 Python 对象，用平铺 int 数组：

```python
# 每像素 4 个 int：[type_id, velocity, lifetime, flags]
# flags: bit 0 = is_dirty, bit 1 = is_static
TYPE_ID  = 0
VELOCITY = 1
LIFETIME = 2
FLAGS    = 3
STRIDE   = 4

FLAG_DIRTY  = 0b01
FLAG_STATIC = 0b10
```

`cells = [0] * (width * height * STRIDE)`

访问 `(x, y)`：`base = (y * width + x) * STRIDE`

### 3.2 CellGrid (`core/grid.py`)

```
CellGrid:
  属性:
    width, height: int
    cells: list[int]
    registry: MaterialRegistry
    reaction_table: ReactionTable
    frame_count: int

  方法:
    update():
      1. 清除所有 is_dirty 标记
      2. 自底向上遍历行（y 从 height-1 到 0）
         - 列方向：frame_count 奇数 → 左到右，偶数 → 右到左
         - 跳过 type_id == 0（air）
         - 跳过 is_dirty == True 的像素
         - 查 registry 拿 cell_type → 分派运动规则
         - 规则返回目标坐标 → swap → 标记双方 is_dirty
         - 检查新位置邻居是否触发反应
      3. lifetime > 0 的像素递减，到 0 → 设为 air
      4. frame_count++

    get_type_id(x, y) → int
    set_cell(x, y, type_id)
    swap(x1, y1, x2, y2)
    get_type_id_array() → list[int]   # 渲染用，返回所有 type_id 平铺
    in_bounds(x, y) → bool
```

### 3.3 运动规则 (`core/rules.py`)

4 套纯函数，签名统一：`(grid, x, y, material_def) → Optional[tuple[int, int]]`

返回目标坐标或 None（不动）。grid 负责调用后执行 swap。

| cell_type | 移动候选序列 | 密度逻辑 |
|-----------|-------------|----------|
| `solid` | 不检查，直接返回 None | — |
| `powder` | (x, y+1) → random_order[(x-1, y+1), (x+1, y+1)] | 目标密度 < 自身 → swap |
| `liquid` | (x, y+1) → random_order[(x-1, y+1), (x+1, y+1)] → random_order[(x-vel, y), (x+vel, y)] | 目标密度 < 自身 → swap |
| `gas` | (x, y-1) → random_order[(x-1, y-1), (x+1, y-1)] → random_order[(x-vel, y), (x+vel, y)] | 目标密度 > 自身 → swap（气体上浮） |
| `energy` | fire: random_choice[(x, y-1), (x-1, y-1), (x+1, y-1)] | 只覆写 air，不做密度交换 |

**方向偏差缓解**：
- 斜下/斜上的左右检查顺序每次随机
- 液体/气体的横向方向由 velocity 字段决定（-1 或 +1），碰壁后翻转
- 列遍历方向每帧交替

**反应检查时机**：像素完成移动后，检查新位置四邻（上下左右）是否存在反应对。命中则按 probability 随机决定是否触发，触发后替换双方 type_id。

---

## 4. 渲染层

### 4.1 PygameRenderer (`render/pygame_renderer.py`)

采用 numpy surfarray + LUT 方案，零 Python 循环：

```python
# 初始化时构建颜色查找表
color_lut = np.zeros((max_type_id + 1, 3), dtype=np.uint8)
for mat in registry.all():
    color_lut[mat.type_id] = mat.color

# color_variance: 为每个 (x,y) 预生成固定偏移矩阵
variance_matrix = np.random.randint(-max_var, max_var+1, (width, height, 3), dtype=np.int16)

# 每帧渲染
type_ids = np.array(grid.get_type_id_array(), dtype=np.int32).reshape(width, height)
color_buffer = color_lut[type_ids].astype(np.int16)    # 花式索引
color_buffer += variance_matrix                         # 叠加颜色偏移
np.clip(color_buffer, 0, 255, out=color_buffer)
pygame.surfarray.blit_array(surface, color_buffer.astype(np.uint8))
scaled = pygame.transform.scale(surface, (width * scale, height * scale))
screen.blit(scaled, (0, 0))
```

渲染管线：`grid type_id 数组 → LUT 花式索引 → variance 叠加 → clip → blit_array → scale → 显示`

### 4.2 InputHandler (`render/input_handler.py`)

| 操作 | 绑定 |
|------|------|
| 放置材质 | 鼠标左键按住拖拽 |
| 擦除（air） | 鼠标右键按住拖拽 |
| 切换材质 | 数字键 1-8（wall/wood/sand/water/oil/lava/fire/steam） |
| 调整画笔大小 | 滚轮 |
| 暂停/恢复 | 空格 |
| 清空网格 | R |

画笔在光标位置 brush_size 半径内填充，坐标需除以 scale 换算到网格坐标。

### 4.3 主循环 (`main.py`)

```
1. 加载 MaterialRegistry + ReactionTable（从 data/materials.toml）
2. 创建 CellGrid(128, 128)
3. 创建 PygameRenderer(scale=4) + InputHandler
4. 主循环：
   a. handle_events() → 处理输入
   b. if not paused: grid.update()
   c. renderer.render(grid)
   d. renderer.draw_ui() → 当前材质、FPS
   e. pygame.display.flip()
   f. clock.tick(60)
```

---

## 5. 测试策略

```
tests/
├── conftest.py             # 共用 fixture
│   - small_registry()      # 精简版 registry（3 种材质）
│   - small_grid(8, 8)      # 8×8 确定性测试网格
│   - deterministic seed    # 固定随机种子
├── test_materials.py
│   - TOML 加载正确性
│   - tag 倒排索引正确
│   - type_id 分配从 1 开始、0 = air
├── test_reactions.py
│   - 标签展开为具体材质对
│   - _self 关键字处理
│   - 对称注册 (a,b) 和 (b,a)
│   - probability=0 不触发, probability=1 必触发
├── test_grid.py
│   - 沙子自由下落到底部停住
│   - 水填满 U 型容器
│   - 油浮在水上（密度交换）
│   - steam 上升后 lifetime 到期消失
│   - lava+water 反应生成 rock+steam
│   - 边界不越界
├── test_rules.py
│   - powder 下/斜下优先级
│   - liquid 横向扩散
│   - gas 上升逻辑
│   - 密度交换方向正确
│   - velocity 碰壁翻转
└── test_renderer.py
    - color_lut 构建正确
    - get_type_id_array 长度 = width * height
```

测试在 8×8 网格上跑，关闭随机化（固定种子），手动放置初始状态 → 执行 N 帧 → 断言像素位置。

---

## 6. 项目结构总览

```
prototype/
├── data/
│   └── materials.toml          # 材质定义 + 反应表
├── core/
│   ├── __init__.py
│   ├── material.py             # MaterialDef, MaterialRegistry
│   ├── reaction.py             # ReactionRule, ReactionResult, ReactionTable
│   ├── cell.py                 # 常量（STRIDE, TYPE_ID, FLAGS 等）
│   ├── grid.py                 # CellGrid
│   └── rules.py                # move_powder/liquid/gas/energy
├── render/
│   ├── __init__.py
│   ├── pygame_renderer.py      # surfarray + LUT 渲染
│   └── input_handler.py        # 鼠标/键盘交互
├── tests/
│   ├── conftest.py
│   ├── test_materials.py
│   ├── test_reactions.py
│   ├── test_grid.py
│   ├── test_rules.py
│   └── test_renderer.py
├── main.py                     # 入口
└── requirements.txt            # pygame, numpy, pytest
```

**依赖**：pygame, numpy, pytest。Python 3.11+ 内置 `tomllib`，无需额外 TOML 库。

---

## 7. Noita 参考对照

| Noita 做法 | 本原型做法 | 差异原因 |
|------------|-----------|----------|
| `materials.xml` + `<CellData>` + `<Reaction>` | `materials.toml` + `[materials.*]` + `[[reactions]]` | TOML 更 Pythonic，迁移 C# 时可换 JSON |
| `<CellDataChild _parent="...">` 继承 | Phase 1 不做继承，材质独立定义 | 7 种材质无需继承，后续可加 |
| 4 种 cell_type（solid/liquid/gas/fire）+ `liquid_sand` flag | 5 种 cell_type（solid/powder/liquid/gas/energy），8 种材质 | 简化 Noita 的 liquid+liquid_sand 组合；wood 验证可燃固体 |
| `[tag]` 标签匹配反应 | 同样支持 `[tag]` 语法 | 直接对齐 |
| `wang_color` 用于世界生成 | 不需要（无程序化世界生成） | Phase 1 无此需求 |
| 64×64 chunk + dirty rect + 棋盘格多线程 | 128×128 全量遍历 | Phase 1 验证算法，不做优化 |
| 单缓冲 in-place 更新 | 同样单缓冲 | 对齐 Noita 架构 |
