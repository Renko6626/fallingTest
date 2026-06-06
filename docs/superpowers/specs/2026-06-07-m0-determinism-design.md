> 文档路径：`docs/superpowers/specs/2026-06-07-m0-determinism-design.md`
> 运行时版本：Python 3.13（CPython）
> 最近更新：2026-06-07 (UTC+8)

# M0 确定性地基 — 实现级设计

需求与验收源：`docs/proposals/2026-06-06-deterministic-parallel-and-netcode.md` §5 M0 行（双重评审 + 用户四项裁决）。本 spec 只钉**实现级决策**；设计于 2026-06-07 经用户批准。

## 1. 新模块

### 1.1 `core/rng.py`（D2）

```python
MASK32 = 0xFFFFFFFF

def squirrel5(pos: int, seed: int) -> int:
    """SquirrelNoise5 逐位移植（kevinmoran gist 常量），每步 & MASK32。"""
    # SQ5_BIT_NOISE1..5 = 0xd2a80a3f, 0xa884f197, 0x6C736F4B, 0xB79F3ABB, 0x1b56c4f5
    # mangled = pos*N1 +seed ^>>9 +N2 ^>>11 *N3 ^>>13 +N4 ^>>15 *N5 ^>>17

def frame_seed(world_seed: int, tick: int) -> int:
    return squirrel5(tick, world_seed)          # 每帧预计算一次

# 互异大奇素数折叠（Squirrel 多维噪声同款思路）；每 draw 仅 1 次 squirrel5
P_X, P_Y, P_PASS, P_SALT, P_ATTEMPT = 0x9E3779B1, 0x85EBCA77, 0xC2B2AE3D, 0x27D4EB2F, 0x165667B1

def rng_u32(fseed, pass_id, x, y, salt, attempt=0) -> int:
    pos = (x*P_X + y*P_Y + pass_id*P_PASS + salt*P_SALT + attempt*P_ATTEMPT) & MASK32
    return squirrel5(pos, fseed)

def rng_chance(fseed, pass_id, x, y, salt, threshold_u32, attempt=0) -> bool   # rng < threshold
def order2(fseed, pass_id, x, y, salt) -> tuple[int, int]    # (0,1) 或 (1,0)，替代两对角 shuffle
def perm3(fseed, pass_id, x, y, salt) -> tuple[int, int, int]  # rng % 6 查 PERMS3 表

# salt 注册表（fire spec v2 已预订 10–12）
SALT_DIAG = 1; SALT_ENERGY_LINGER = 2; SALT_ENERGY_DIR = 3; SALT_REACTION = 4
```

- key 语义遵守 D2：(x,y) = 决策时刻像素所在坐标；attempt = 该 (坐标,salt) 本 tick 本 pass 第 N 次取数；M0 串行期 `pass_id=0`。
- 选"素数折叠 + 单次哈希"而非链式 3 连哈希：纯 Python 下 RNG 是性能主因（评审实测 ~18×），每 draw 一次调用。折叠的理论碰撞（线性组合 aliasing）只影响噪声质量不影响确定性，可接受。

### 1.2 `core/ops.py`

```python
def apply_brush(grid, gx, gy, type_id, brush_size) -> list[tuple[int,int]]:
    """矩形 brush 写格子，与 InputHandler 现行为逐位一致；返回实际写入坐标（录制用）。"""
```
InputHandler 与回放器共用，消除两份实现漂移。

### 1.3 `prototype/replay.py`（D7）

- **格式 JSONL**：首行 header `{"v":1,"toml_sha256":...,"w":...,"h":...,"seed":...}`；事件行 `{"f":frame,"op":"paint","x":...,"y":...,"id":...,"r":brush}`。
- `Recorder`：main.py `--record out.jsonl --seed N` 开启；InputHandler 每帧把实际 apply 的 op 交给 recorder。
- `python replay.py demo.jsonl [--hash-every N]`：headless（不 import render/pygame），按 header 建世界、逐帧应用事件 + `grid.update()`，打印逐帧/终值 `state_hash`。**header 的 toml_sha256 / 尺寸 / seed 不匹配即拒绝**（评审 m7）。

## 2. 既有代码改动

### 2.1 替换 6 处 `random.*`（完成后两文件删 `import random`）

| 位置 | 现状 | 替换 |
|---|---|---|
| `rules.py` `_move_powder` | `random.shuffle(diags)` | `order2(salt=SALT_DIAG)` 决定两对角顺序 |
| `rules.py` `_move_liquid` | 同上 | 同上 |
| `rules.py` `_move_gas` | 同上 | 同上 |
| `rules.py` `_move_energy` | `random.random() < 0.4` | `rng_chance(SALT_ENERGY_LINGER, THRESH_40)` |
| `rules.py` `_move_energy` | `random.shuffle(candidates)`（3 个） | `perm3(SALT_ENERGY_DIR)` |
| `grid.py` `_check_reactions` | `random.random() < result.probability` | `rng_u32(SALT_REACTION, attempt=局部计数) < result.threshold` |

### 2.2 `CellGrid`

- `__init__(..., seed: int = 0)`；`update()` 开头 `self._fseed = frame_seed(self.seed, self.frame_count)`（rules 经 grid 取用）。
- `state_hash() -> int`：`zlib.crc32(array('i', self.cells).tobytes())`——stdlib、C 速度、同机确定；跨平台字节序口径 C# 期再钉（D5 注明）。M0 仅 world hash；per-chunk 待 M0.5。

### 2.3 D1 加载层整数化

- `materials.toml`：density ×10 取整（wall 100 / rock 90 / wood 80 / sand 60 / water 10 / oil 8 / lava 30 / steam 1 / fire 0）。
- `material.py`：`MaterialDef.density: int`，加载 `int(round(float(...)))`（兼容测试 fixture 的旧 float 写法）。
- `reaction.py`：`ReactionResult.threshold: int = min(round(p × 2**32), 2**32−1)`，替换 probability 字段。

## 3. 测试计划

- `tests/test_rng.py`：squirrel5 金值（固定输入→固定输出，≥4 锚点）；key 各分量（x/y/pass/salt/attempt）独立性（改任一分量输出变化）；u32 值域。
- `tests/test_determinism.py`（提案验收四件套的 ①②③）：
  1. 同 seed 两次构建 + 跑 200 帧 → 逐帧 hash 相等（128×128 合成场景：底部墙 + 沙柱 + 水池 + lava 点，~30% 活跃）；
  2. **污染测试**：第二跑帧间执行 `random.seed(os.urandom)` + 取数 → hash 仍与第一跑逐帧相等；
  3. 录制（合成事件流）→ 回放 → 逐帧 hash 相等。
- 既有测试重钉：`test_rules.py` 的 `for seed in range(100)` 扫描 collapse 为固定 world_seed 单断言；各 `random.seed(42/0/123)` 删除；conftest 不变（loader 兼容 float density）。
- `prototype/benchmark.py`：128×128、30% 活跃、200 帧计时 → 打印 `{w}x{h}, {ratio}% active, {fps} FPS`；结果替换 `docs/perf/baseline.md` 的 provisional 数字（验收④）。

## 4. 不做（YAGNI）

per-chunk hash（M0.5）；save/load（D6 后续）；velocity 8.8 定点化（速度积分时做，M0 不动 VELOCITY 的 ±1 方向记忆语义）；跨语言 hash 一致；录像压缩。

## 5. 风险与备注

- 性能回退预算：基线 42 FPS → 预测 ~28–33（评审 m1）；benchmark 实测超预算 20% 须在 CHANGELOG 说明。
- 仓库无 venv（`.gitignore` 有条目但目录不存在）：实施首步建 `venv/` 装 `prototype/requirements.txt`。
- replay 路径不得 import pygame（headless 可用）。
