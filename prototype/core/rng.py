"""Counter-based RNG（确定性契约 D2）。

key = (world_seed, tick, pass_id, x, y, salt, attempt)：
`frame_seed` 每帧预计算一次，其余分量素数折叠后单次 squirrel5。
(x, y) 取决策时刻像素坐标；attempt 为该 (坐标, salt) 在本 tick 本 pass 的第 N 次取数。
任何模拟代码禁止使用全局 `random` 顺序流。
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
    pos = (
        x * P_X + y * P_Y + pass_id * P_PASS + salt * P_SALT + attempt * P_ATTEMPT
    ) & MASK32
    return squirrel5(pos, fseed)


def threshold_u32(p: float) -> int:
    """概率 → u32 阈值。2 的幂缩放无精度损失；p=1.0 钳位（失败率 2^-32 可忽略）。"""
    return min(round(p * 4294967296), 4294967295)


def rng_chance(
    fseed: int, pass_id: int, x: int, y: int, salt: int, threshold: int, attempt: int = 0
) -> bool:
    return rng_u32(fseed, pass_id, x, y, salt, attempt) < threshold


def order2(fseed: int, pass_id: int, x: int, y: int, salt: int) -> tuple[int, int]:
    return (0, 1) if rng_u32(fseed, pass_id, x, y, salt) & 1 == 0 else (1, 0)


def perm3(fseed: int, pass_id: int, x: int, y: int, salt: int) -> tuple[int, int, int]:
    return _PERMS3[rng_u32(fseed, pass_id, x, y, salt) % 6]
