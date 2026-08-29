from __future__ import annotations

import tomllib
from dataclasses import dataclass

from core.material import MaterialRegistry
from core.rng import threshold_u32

SELF_MARKER = -1


@dataclass(frozen=True)
class ReactionResult:
    output1: int
    output2: int
    threshold: int  # u32 阈值（确定性契约 D1）：rng_u32 < threshold 即触发


class ReactionTable:
    def __init__(self, toml_path: str, registry: MaterialRegistry) -> None:
        with open(toml_path, "rb") as f:
            data = tomllib.load(f)

        self._table: dict[tuple[int, int], list[ReactionResult]] = {}

        for reaction in data.get("reactions", []):
            input_names = reaction["input"]
            output_names = reaction["output"]
            probability = reaction["probability"]

            input1_ids = self._resolve(input_names[0], registry)
            input2_ids = self._resolve(input_names[1], registry)
            threshold = threshold_u32(probability)

            for id1 in input1_ids:
                for id2 in input2_ids:
                    if id1 == id2:
                        continue
                    out1 = self._resolve_output(output_names[0], registry)
                    out2 = self._resolve_output(output_names[1], registry)

                    forward = ReactionResult(out1, out2, threshold)
                    reverse = ReactionResult(out2, out1, threshold)

                    self._table.setdefault((id1, id2), []).append(forward)
                    self._table.setdefault((id2, id1), []).append(reverse)

    def _resolve(self, name: str, registry: MaterialRegistry) -> set[int]:
        if name.startswith("[") and name.endswith("]"):
            tag = name[1:-1]
            return registry.get_ids_by_tag(tag)
        return {registry.get_by_name(name).type_id}

    def _resolve_output(self, name: str, registry: MaterialRegistry) -> int:
        if name == "_self":
            return SELF_MARKER
        return registry.get_by_name(name).type_id

    def get(self, type_a: int, type_b: int) -> list[ReactionResult] | None:
        results = self._table.get((type_a, type_b))
        return results if results else None
