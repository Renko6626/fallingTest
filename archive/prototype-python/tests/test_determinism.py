"""M0 验收：①同 seed 等价 ②污染测试 ③录放等价（提案 §5 M0 行）。"""
import random
from pathlib import Path

import pytest

from core.grid import CellGrid
from core.material import MaterialRegistry
from core.ops import apply_brush
from core.reaction import ReactionTable
from replay import Recorder, replay_file

REAL_TOML = str(Path(__file__).parent.parent / "data" / "materials.toml")


def build_world(seed: int) -> CellGrid:
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
        if pollute:                          # 帧间扰动全局 random 顺序流
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
    """核心谓词（评审 M3）：sim 已彻底脱离全局 random 流。"""
    clean = run_hashes(build_world(seed=7), 120, pollute=False)
    dirty = run_hashes(build_world(seed=7), 120, pollute=True)
    assert clean == dirty


def test_record_replay_roundtrip(tmp_path):
    demo = str(tmp_path / "demo.jsonl")
    reg = MaterialRegistry(REAL_TOML)
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
    assert hashes[-1][0] == total - 1        # 回放总帧数对齐
    assert hashes[-1][1] == live_hash        # 回放终值 == 实时跑终值


def test_replay_rejects_wrong_toml(tmp_path):
    demo = str(tmp_path / "demo.jsonl")
    rec = Recorder(demo, REAL_TOML, 8, 8, seed=0)
    rec.close()
    other = tmp_path / "other.toml"
    other.write_text(Path(REAL_TOML).read_text() + "\n# changed\n")
    with pytest.raises(ValueError):
        replay_file(demo, str(other))
