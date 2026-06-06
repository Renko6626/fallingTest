from __future__ import annotations

import tomllib
from dataclasses import dataclass
from pathlib import Path


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


_AIR = MaterialDef(
    name="air",
    type_id=0,
    cell_type="solid",
    density=0,
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
                density=int(round(float(props["density"]))),  # D1：兼容旧 float 写法
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
