> 文档路径：`docs/superpowers/specs/2026-06-07-m05-chunked-scheduler-design.md`
> 运行时版本：Python 3.13（CPython）
> 最近更新：2026-06-07 (UTC+8)

# M0.5 单线程 4-pass/chunk 调度器 — 实现级设计

语义源：`docs/proposals/2026-06-06-deterministic-parallel-and-netcode.md` §2.2 条件①③ + §2.3 顺序账本（正方形写域、所有权制、世代戳、读域夹断）。设计于 2026-06-07 经用户批准（三决策：①删 FLAG_DIRTY ②set_cell 显式 stamp 参数 ③benchmark 双尺寸）。

## 1. `core/chunks.py`（纯函数 + 不变小类，全部可单测）

```python
CHUNK = 64
MARGIN = 32
PASS_PARITY = ((0, 0), (1, 0), (0, 1), (1, 1))   # 固定 pass 序

@dataclass(frozen=True)
class Rect:            # 半开区间 [x0,x1) × [y0,y1)
    x0: int; y0: int; x1: int; y1: int
    def contains(self, x, y) -> bool

class ChunkLayout:
    def __init__(self, width, height)             # cw/ch = ceil 划分，边缘 chunk 裁剪
    def chunk_rect(self, cx, cy) -> Rect          # 本 chunk 实际像素区（裁剪到世界）
    def write_rect(self, cx, cy) -> Rect          # [cx*64−32, cx*64+96)²，裁剪到世界
    def chunks_for_pass(self, pass_id) -> Iterator[tuple[int, int]]   # parity 匹配，行优先
```

## 2. 像素属性（STRIDE 4→5）

```python
TYPE_ID=0; VELOCITY=1; LIFETIME=2; FLAGS=3; UPDATED_AT=4; STRIDE=5
```

- `UPDATED_AT`：最后一次"行动"（移动/被动交换/in-update 转化）的帧号；初始/未行动 = **-1**。
- **FLAG_DIRTY 与 FLAG_STATIC 删除**（决策①）：世代戳取代 dirty；per-pixel static 已被调研否决（deep-dive §2.1，走 per-chunk dirty rect 路线）。FLAGS 字段保留给 fire spec 的 FLAG_BURNING。
- 每帧 O(N) 清 flag pass 随之删除。
- lifetime 归零清格时同时清 UPDATED_AT（避免死格残值进 hash 的历史噪声）。

## 3. `set_cell` 盖戳语义（决策②，对提案 §2.3 row 5 字面的已批准偏离）

```python
def set_cell(self, x, y, type_id, stamp: bool = False) -> None
    # stamp=True → UPDATED_AT = frame_count（本帧不再行动）；False → -1
```

- **update 期间**的一切 set_cell（反应产物；未来火系统转化/生成队列）**必须传 stamp=True**——观察契约"产物下帧才动"（评审 m2）不变。
- 场景搭建 / 笔刷（update 之外）不盖戳——避免笔刷 1 帧延迟与开场死帧。
- 防遗忘措施：spec 此节 + `test_chunked_semantics.py` 钉"反应产物同帧不动"。

## 4. `update()` pass 结构

```python
self._fseed = frame_seed(self.seed, self.frame_count)
for pass_id in range(4):
    self._pass_id = pass_id
    for (cx, cy) in self.layout.chunks_for_pass(pass_id):
        self._write_rect = self.layout.write_rect(cx, cy)
        rect = self.layout.chunk_rect(cx, cy)
        # chunk 内：自底向上，x 方向按帧奇偶交替（沿用全局约定）
        for y in rect.y1-1 .. rect.y0:
            for x in（奇偶决定方向）rect.x0 .. rect.x1-1:
                AIR → skip；UPDATED_AT == frame_count → skip（跨缝移入者防二动）
                target = try_move(self, x, y)
                if target: swap + 双方 _stamp() + _check_reactions(target)
                else: _check_reactions(x, y)
# lifetime 全局 pass（各格独立，不变）→ frame_count += 1
```

- `__init__` 预置 `self._pass_id = 0`、`self._write_rect = 全世界 Rect`（测试可直接调 try_move）。
- `_stamp(x, y)`：`cells[base+UPDATED_AT] = frame_count`。

## 5. 域约束（运行时契约）

- `_can_move_to`：`in_bounds` 检查替换为 `grid._write_rect.contains(x, y)`（写域已裁剪到世界，蕴含 in_bounds）。
- `_check_reactions`：邻居 `not in _write_rect` → 本 pass 跳过（读域夹断/延迟语义）。
- **诚实声明**：当前物理全部 ±1 格、距写域边 ≥30px，两条检查在 M0.5 不会实际触发——它们是为速度积分预埋的契约；逻辑本身用直接单测（喂越界坐标）验证。

## 6. RNG 接线

rules/reactions 的 `pass_id` 实参从硬编码 `0` 改为 `grid._pass_id`。M0 的具体 hash 序列随之失效（提案推论 2 已声明接受）；确定性测试全部是 run-vs-run 对比，不硬编码 hash 值，自动存活。

## 7. 测试

- `tests/test_chunks.py`：划分数学（整除/非整除世界）、parity 完备不重叠、write_rect 数值与边缘裁剪、pass 覆盖全部 chunk 恰一次。
- `tests/test_chunked_semantics.py`（192×192，3×3 chunk——128×128 每 pass 单 chunk 跑不到多 chunk 路径）：
  - **材质计数守恒**：混合场景 N 帧后逐材质计数不变（缝隙源/汇 bug 的最强探测器）；
  - 沙柱跨水平缝下落不丢沙、最终落底；水流过垂直缝两侧都可达；
  - 同 seed hash 等价 + 污染测试（192² 复跑，帧数收敛到 <10s）；
  - 反应产物同帧不动（probability=1.0 fixture，update 一次后直查产物位置）；
  - 域检查单测：`_write_rect` 限制下 `_can_move_to` 拒绝越界目标（直接喂坐标）。
- 既有 56 测试预期存活（8×8 = 单 chunk pass 0，行为同旧串行；产物延迟 1 帧只影响松断言不触及的细节）。

## 8. benchmark（决策③）

正式基准保持 128×128（与 M0 基线可比）+ 追加 192×192 数据点。预期：调度循环与域检查增开销、删 clear-dirty pass 省回——实测说话，预算 20%。

## 9. 不做（YAGNI）

并行（M1）、chunk 休眠/dirty rect、velocity、缝隙延迟交互的实际触发场景。

## 10. 已知行为变化（接受并留档）

垂直相邻 chunk 的 pass 先后造成缝隙处下落 1 帧相位差（特征测试记录形态）；反应产物推迟 1 帧行动；M0 hash 序列作废。
