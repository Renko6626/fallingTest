#!/usr/bin/env python3
"""Demo 录制/回放（确定性契约 D7）。

JSONL 格式：首行 header，其后 paint 事件行（按帧升序）。
回放语义与 main.py 主循环一致：先应用本帧事件，再 grid.update()。
header 的 toml_sha256 / 尺寸 / seed 不匹配即拒绝（评审 m7：type_id 由 TOML
文件顺序隐式分配，材质表变动会让旧 demo 静默错放材质）。

本模块必须保持 headless——禁止 import render/pygame。
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
    """录制器：main.py --record 时由 InputHandler 调用 log_paint。"""

    def __init__(self, out_path: str, toml_path: str, width: int, height: int, seed: int) -> None:
        self._f = open(out_path, "w", encoding="utf-8")
        header = {
            "v": FORMAT_VERSION,
            "toml_sha256": toml_sha256(toml_path),
            "w": width,
            "h": height,
            "seed": seed,
        }
        self._f.write(json.dumps(header) + "\n")

    def log_paint(self, frame: int, gx: int, gy: int, type_id: int, brush_size: int) -> None:
        self._f.write(
            json.dumps({"f": frame, "op": "paint", "x": gx, "y": gy, "id": type_id, "r": brush_size})
            + "\n"
        )

    def close(self) -> None:
        self._f.close()


def replay_file(
    demo_path: str,
    toml_path: str = DEFAULT_TOML,
    extra_frames: int = 0,
    hash_every: int = 0,
) -> list[tuple[int, int]]:
    """回放并返回 [(frame, state_hash)]，末项恒为最终帧。校验失败抛 ValueError。"""
    lines = Path(demo_path).read_text(encoding="utf-8").splitlines()
    header = json.loads(lines[0])
    if header["v"] != FORMAT_VERSION:
        raise ValueError(f"unsupported demo version: {header['v']}")
    if header["toml_sha256"] != toml_sha256(toml_path):
        raise ValueError("materials.toml 与录制时不一致，拒绝回放（评审 m7）")
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
    ap = argparse.ArgumentParser(description="headless demo replayer (D7)")
    ap.add_argument("demo")
    ap.add_argument("--toml", default=DEFAULT_TOML)
    ap.add_argument("--extra-frames", type=int, default=0)
    ap.add_argument("--hash-every", type=int, default=0)
    args = ap.parse_args()
    for frame, h in replay_file(args.demo, args.toml, args.extra_frames, args.hash_every):
        print(f"frame {frame}: {h:08x}")


if __name__ == "__main__":
    main()
