> 文档路径：`docs/superpowers/specs/2026-05-26-fire-system-design.md`
> 运行时版本：Python 3.11+
> 最近更新：2026-06-06 (UTC+8)

> **2026-06-06 裁决（用户拍板）**：调研证实 Noita 无温度场（`docs/reference/noita-deep-dive.md` §2.3、§5.3）。本 spec 按 **"Noita 式优先"** 方向修订执行：
> - 保留：`fire_hp` 消耗 / `requires_oxygen` 表面燃烧 / `burn_to` 燃尽转化（已与 Noita 一致），建议补 `fire_hp=-1` 永燃与烟参数化（`on_fire_smoke_material` + `generates_smoke`）。
> - 改造：点燃判定改为 火源 `temperature_of_fire` ≥ 邻居 `autoignition_temp` 的**静态比较** + 随机方向采样 + 概率（随机数走确定性契约 D2 的 counter RNG）。
> - 降级：§3 的 TEMPERATURE 字段与 §5 温度场+热传导整章 → 可选实验分支（需先设计休眠条件并过 benchmark）。
> - 时序：排在 M0 确定性地基之后实施（`docs/proposals/2026-06-06-deterministic-parallel-and-netcode.md` §5）。旧反应表火焰调参已留档于 commit `b99b2ec`。

# 火焰系统重设计：Noita 风格 HP 消耗 + 热传导 + 表面燃烧

## 1. 动机

当前火焰实现是"反应表直接替换材质"——fire 接触 wood 时通过概率反应把 wood 变成 fire。问题：

- 火焰生成后立刻上浮远离燃料，无法持续蔓延
- 没有从外向内的燃烧效果，大块木头瞬间消失
- 概率参数难以调平衡——太高则爆燃，太低则灭火
- 不支持不同材质不同燃烧速度（wood 和 oil 行为相同）

Noita 的做法：材质自身有 `fire_hp`（耐火血量），被火逐帧削减；燃烧通过温度传播驱动而非反应表；`requires_oxygen` 实现表面燃烧。

## 2. 目标与范围

| 做 | 不做 |
|---|---|
| 每像素温度场 + 完整热传导 | 烟雾粒子系统（复用现有 steam） |
| fire_hp 消耗 + 材质转化（wood→ash） | 爆炸力学（冲击波、抛射） |
| 表面燃烧（requires_oxygen） | 对流传热（热气流上升加速热传播） |
| 燃烧颜色混合 + 生成火焰像素 | 温度影响材质物理属性（如熔化） |
| 水蒸发复用温度系统 | 冰冻/凝固机制 |

---

## 3. 像素属性扩展

### 3.1 cell.py

STRIDE 从 4 扩展到 6：

```python
TYPE_ID     = 0
VELOCITY    = 1
LIFETIME    = 2
FLAGS       = 3
TEMPERATURE = 4   # 新增：当前温度（int，0-1000）
FIRE_HP     = 5   # 新增：剩余耐火血量
STRIDE      = 6

FLAG_DIRTY    = 0b001
FLAG_STATIC   = 0b010
FLAG_BURNING  = 0b100  # 新增：正在燃烧
```

### 3.2 MaterialDef 新增字段

```python
@dataclass(frozen=True)
class MaterialDef:
    # 现有字段
    name: str
    type_id: int
    cell_type: str
    density: float
    color: tuple[int, int, int]
    color_variance: int
    lifetime: int
    tags: frozenset[str]
    # 新增字段
    fire_hp: int                  # 耐火血量，0=不可燃
    autoignition_temp: int        # 自燃温度阈值
    burn_rate: int                # 每帧 fire_hp 减少量
    burn_to: str                  # 燃尽后变成的材质名
    temperature_of_fire: int      # 燃烧时辐射的温度值
    thermal_conductivity: float   # 热传导系数（0.0-1.0）
    generates_fire: float         # 燃烧时在邻居 air 生成 fire 像素的概率
    requires_oxygen: bool         # 是否需要接触空气才能燃烧
```

默认值：

| 字段 | 默认值 |
|------|--------|
| `fire_hp` | 0 |
| `autoignition_temp` | 1000 |
| `burn_rate` | 1 |
| `burn_to` | `"air"` |
| `temperature_of_fire` | 0 |
| `thermal_conductivity` | 0.1 |
| `generates_fire` | 0.0 |
| `requires_oxygen` | false |

---

## 4. materials.toml 完整定义

```toml
[meta]
version = 2
default_grid_size = [128, 128]

[materials.wall]
cell_type = "solid"
density = 10.0
color = [128, 128, 128]
tags = ["solid"]
thermal_conductivity = 0.02

[materials.rock]
cell_type = "solid"
density = 9.0
color = [100, 100, 100]
color_variance = 8
tags = ["solid"]
thermal_conductivity = 0.05

[materials.wood]
cell_type = "solid"
density = 8.0
color = [139, 90, 43]
color_variance = 10
tags = ["solid", "flammable"]
fire_hp = 200
autoignition_temp = 150
burn_rate = 1
burn_to = "ash"
temperature_of_fire = 80
thermal_conductivity = 0.05
generates_fire = 0.15
requires_oxygen = true

[materials.ash]
cell_type = "powder"
density = 2.0
color = [60, 60, 60]
color_variance = 5
tags = ["powder"]
thermal_conductivity = 0.05

[materials.sand]
cell_type = "powder"
density = 6.0
color = [194, 178, 128]
color_variance = 15
tags = ["powder"]
thermal_conductivity = 0.08

[materials.water]
cell_type = "liquid"
density = 1.0
color = [48, 96, 255]
tags = ["liquid", "water", "conductive"]
fire_hp = 1
autoignition_temp = 100
burn_to = "steam"
thermal_conductivity = 0.4
requires_oxygen = false

[materials.oil]
cell_type = "liquid"
density = 0.8
color = [80, 60, 30]
tags = ["liquid", "flammable"]
fire_hp = 60
autoignition_temp = 100
burn_rate = 2
burn_to = "air"
temperature_of_fire = 120
thermal_conductivity = 0.2
generates_fire = 0.3
requires_oxygen = false

[materials.lava]
cell_type = "liquid"
density = 3.0
color = [255, 96, 0]
tags = ["liquid", "lava", "hot"]
temperature_of_fire = 300
thermal_conductivity = 0.3

[materials.steam]
cell_type = "gas"
density = 0.1
color = [200, 200, 255]
lifetime = 300
tags = ["gas"]
thermal_conductivity = 0.6

[materials.fire]
cell_type = "energy"
density = 0.0
color = [255, 160, 40]
color_variance = 40
lifetime = 120
tags = ["energy", "hot"]
temperature_of_fire = 200
thermal_conductivity = 0.8

# --- 反应表（大幅精简） ---

# 保留：纯化学反应
[[reactions]]
input = ["lava", "water"]
output = ["rock", "steam"]
probability = 0.8

# 删除旧火焰反应——全部由温度系统取代：
# - fire+[flammable] → fire+fire（删除）
# - [hot]+wood → _self+fire（删除）
# - [hot]+water → _self+steam（删除，水蒸发改为温度驱动）
```

---

## 5. 热传导系统

### 5.1 算法

在 `update()` 中作为独立 pass 执行。使用**增量缓冲**避免遍历顺序影响结果。

```python
DIFFUSION_RATE = 0.1       # 全局扩散速率
NATURAL_COOLING = 1        # 每帧自然冷却量
TEMP_MIN = 0
TEMP_MAX = 1000

def _thermal_pass(self):
    n = self.width * self.height
    temp_delta = [0.0] * n

    for y in range(self.height):
        for x in range(self.width):
            idx = y * self.width + x
            base = idx * STRIDE
            type_id = self.cells[base + TYPE_ID]
            
            if type_id == AIR:
                self.cells[base + TEMPERATURE] = 0
                continue
            
            t_self = self.cells[base + TEMPERATURE]
            mat = self.registry.get_by_id(type_id)
            k_self = mat.thermal_conductivity
            
            for nx, ny in [(x,y-1), (x,y+1), (x-1,y), (x+1,y)]:
                if not self.in_bounds(nx, ny):
                    continue
                n_idx = ny * self.width + nx
                n_base = n_idx * STRIDE
                n_type = self.cells[n_base + TYPE_ID]
                
                if n_type == AIR:
                    t_neighbor = 0
                    k_neighbor = 1.0  # air 导热极好（散热器）
                else:
                    t_neighbor = self.cells[n_base + TEMPERATURE]
                    k_neighbor = self.registry.get_by_id(n_type).thermal_conductivity
                
                k_avg = (k_self + k_neighbor) / 2
                delta = (t_neighbor - t_self) * k_avg * DIFFUSION_RATE
                temp_delta[idx] += delta

    # 一次性应用增量
    for i in range(n):
        base = i * STRIDE
        type_id = self.cells[base + TYPE_ID]
        if type_id == AIR:
            continue
        
        # 热源：燃烧中或天然热源（lava/fire），温度钉在 temperature_of_fire
        mat = self.registry.get_by_id(type_id)
        if self.cells[base + FLAGS] & FLAG_BURNING or mat.temperature_of_fire > 0:
            self.cells[base + TEMPERATURE] = mat.temperature_of_fire
        else:
            new_temp = self.cells[base + TEMPERATURE] + int(temp_delta[i])
            new_temp -= NATURAL_COOLING
            self.cells[base + TEMPERATURE] = max(TEMP_MIN, min(TEMP_MAX, new_temp))
```

### 5.2 温度来源

| 来源 | 行为 |
|------|------|
| 燃烧中的像素（FLAG_BURNING） | 温度钉在 `temperature_of_fire` |
| lava（`temperature_of_fire=300`） | 始终辐射 300°，永不衰减 |
| fire 像素（`temperature_of_fire=200`） | 始终辐射 200°，直到 lifetime 耗尽 |
| 其它材质 | 通过传导接收温度，自然冷却每帧 -1 |

### 5.3 swap 同步

像素移动时 swap 操作交换 STRIDE 内所有字段，temperature 和 fire_hp 自动跟随，无需额外处理。

---

## 6. 燃烧系统

### 6.1 状态机

```
[正常] ──温度≥autoignition_temp──► [燃烧中] ──fire_hp=0──► [燃尽]
   ▲      且 fire_hp > 0              │                      │
   │      且 (无需O₂ 或 邻接air)       │                      ▼
   │                                   │               变为 burn_to 材质
   │      被包围（requires_oxygen）     │
   └───────────── 熄灭 ◄──────────────┘
```

### 6.2 每帧逻辑

```python
def _burn_pass(self):
    fire_type_id = self.registry.get_by_name("fire").type_id
    
    for y in range(self.height):
        for x in range(self.width):
            base = self._base(x, y)
            type_id = self.cells[base + TYPE_ID]
            if type_id == AIR:
                continue
            
            mat = self.registry.get_by_id(type_id)
            flags = self.cells[base + FLAGS]
            fire_hp = self.cells[base + FIRE_HP]
            temp = self.cells[base + TEMPERATURE]
            
            is_burning = flags & FLAG_BURNING
            
            # 1. 点燃检查
            if not is_burning and fire_hp > 0 and temp >= mat.autoignition_temp:
                if not mat.requires_oxygen or self._has_air_neighbor(x, y):
                    self.cells[base + FLAGS] |= FLAG_BURNING
                    is_burning = True
            
            # 2. 燃烧中处理
            if is_burning:
                # 2a. 熄灭检查：缺氧或温度不足
                if mat.requires_oxygen and not self._has_air_neighbor(x, y):
                    self.cells[base + FLAGS] &= ~FLAG_BURNING
                    continue
                if temp < mat.autoignition_temp:
                    self.cells[base + FLAGS] &= ~FLAG_BURNING
                    continue
                
                # 2b. 消耗 fire_hp
                fire_hp -= mat.burn_rate
                self.cells[base + FIRE_HP] = fire_hp
                
                # 2c. 生成火焰像素
                if mat.generates_fire > 0:
                    neighbors = [(x,y-1),(x,y+1),(x-1,y),(x+1,y)]
                    random.shuffle(neighbors)
                    for nx, ny in neighbors:
                        if (self.in_bounds(nx, ny) 
                            and self.get_type_id(nx, ny) == AIR
                            and random.random() < mat.generates_fire):
                            self.set_cell(nx, ny, fire_type_id)
                            break  # 每帧最多生成 1 个
                
                # 2d. 燃尽转化
                if fire_hp <= 0:
                    convert_id = self.registry.get_by_name(mat.burn_to).type_id
                    self.set_cell(x, y, convert_id)

def _has_air_neighbor(self, x: int, y: int) -> bool:
    for nx, ny in [(x,y-1),(x,y+1),(x-1,y),(x+1,y)]:
        if self.in_bounds(nx, ny) and self.get_type_id(nx, ny) == AIR:
            return True
    return False
```

### 6.3 灭火机制

| 触发条件 | 行为 |
|---------|------|
| requires_oxygen 且被包围 | 清除 FLAG_BURNING，停止消耗 fire_hp（但已消耗的不恢复） |
| water 接触 | 通过热传导：水的 thermal_conductivity=0.4 快速吸热降温，温度降到 autoignition_temp 以下后火熄灭；水自身温度升高后蒸发为 steam |
| 降温灭火 | 温度降到 autoignition_temp 以下时清除 FLAG_BURNING，停止消耗 fire_hp（但已消耗的不恢复）。需要外部持续降温（如大量水）才能实现，因为燃烧中的像素自身是热源 |

---

## 7. 渲染变更

### 7.1 燃烧颜色混合

```python
FIRE_COLOR = np.array([255, 100, 20], dtype=np.int16)

def render_to_array(grid, color_lut, variance_matrix, burn_data):
    # 现有渲染逻辑...得到 color_buffer
    
    if burn_data is not None:
        flags_arr, fire_hp_arr, max_fire_hp_arr = burn_data
        burning_mask = (flags_arr & FLAG_BURNING) != 0
        
        if np.any(burning_mask):
            burn_ratio = np.where(
                max_fire_hp_arr > 0,
                1.0 - fire_hp_arr / max_fire_hp_arr,
                0.0
            )
            blend = 0.3 + 0.7 * burn_ratio  # 0.3~1.0
            
            for c in range(3):
                color_buffer[:, :, c] = np.where(
                    burning_mask,
                    (color_buffer[:, :, c] * (1 - blend) + FIRE_COLOR[c] * blend),
                    color_buffer[:, :, c]
                )
    
    return color_buffer
```

### 7.2 CellGrid 新增方法

```python
def get_burn_state_array(self) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """返回 (flags, fire_hp, max_fire_hp) 三个数组，用于渲染。"""
    ...
```

---

## 8. update() 完整 pass 顺序

```
1. 清除 dirty flags
2. 移动 pass（现有逻辑）
3. 反应 pass（现有逻辑，反应表精简后只剩 lava+water）
4. 热传导 pass（新增 — §5）
5. 燃烧 pass（新增 — §6）
6. lifetime 衰减（现有逻辑）
7. frame_count++
```

---

## 9. 改动文件总览

| 文件 | 改动类型 | 内容 |
|------|---------|------|
| `core/cell.py` | 修改 | STRIDE 4→6, 新增 TEMPERATURE/FIRE_HP/FLAG_BURNING |
| `core/material.py` | 修改 | MaterialDef 加 8 个字段, 解析 TOML 新属性 |
| `core/grid.py` | 修改 | set_cell 初始化新字段, 新增 _thermal_pass/_burn_pass/_has_air_neighbor/get_burn_state_array, update() 加两个 pass |
| `core/rules.py` | 不变 | — |
| `core/reaction.py` | 不变 | — |
| `data/materials.toml` | 修改 | 所有材质加热属性, 新增 ash, 删除 3 条旧火焰反应 |
| `render/pygame_renderer.py` | 修改 | render_to_array 追加燃烧颜色混合 |
| `tests/test_thermal.py` | 新增 | 热传导 + 燃烧 + 集成测试 |
| `tests/test_grid.py` | 修改 | 适配 STRIDE=6 |
| `tests/test_materials.py` | 修改 | 适配新字段 |
| `tests/test_renderer.py` | 修改 | 追加燃烧渲染测试 |

---

## 10. 测试策略

### test_thermal.py（新增）

**热传导基础**：
- 高温像素向低温邻居扩散（确认温度变化方向正确）
- air 温度始终为 0
- `thermal_conductivity=0` 的材质不传热
- 温度 clamp 在 0-1000
- 自然冷却每帧 -1

**燃烧机制**：
- 温度达到 `autoignition_temp` → FLAG_BURNING 置位
- `requires_oxygen=true` 且无 air 邻居 → 不点燃
- `requires_oxygen=true` 被包围 → 熄灭（清 FLAG_BURNING）
- 燃烧中 fire_hp 每帧减少 `burn_rate`
- `fire_hp=0` → 变为 `burn_to` 材质
- 燃烧中温度钉在 `temperature_of_fire`
- `generates_fire` 在邻居 air 格子概率生成 fire 像素

**集成场景**：
- fire 接触 wood → wood 逐渐升温 → 自燃 → 从外向内烧 → 变 ash
- oil 被火点燃 → 快速蔓延（低 autoignition, 高 generates_fire）
- 水被加热到 100 → 蒸发为 steam
- 大块 wood 中间像素不燃烧（requires_oxygen 效果验证）

### 现有测试适配

- `test_grid.py`：所有 set_cell/swap 测试适配 STRIDE=6
- `test_materials.py`：验证新字段加载和默认值
- `test_renderer.py`：追加燃烧像素颜色混合验证

测试在 8×8 网格上跑，固定随机种子确保确定性。
