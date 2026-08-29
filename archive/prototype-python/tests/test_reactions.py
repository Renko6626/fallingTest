import pytest
from core.material import MaterialRegistry
from core.reaction import ReactionResult, ReactionTable
from core.rng import threshold_u32


@pytest.fixture
def registry_with_reactions(tmp_path):
    toml_content = """
[meta]
version = 1

[materials.wall]
cell_type = "solid"
density = 100
color = [128, 128, 128]
tags = ["solid"]

[materials.water]
cell_type = "liquid"
density = 10
color = [48, 96, 255]
tags = ["liquid", "water"]

[materials.oil]
cell_type = "liquid"
density = 8
color = [80, 60, 30]
tags = ["liquid", "flammable"]

[materials.lava]
cell_type = "liquid"
density = 30
color = [255, 96, 0]
tags = ["liquid", "lava", "hot"]

[materials.steam]
cell_type = "gas"
density = 1
color = [200, 200, 255]
lifetime = 300
tags = ["gas"]

[materials.fire]
cell_type = "energy"
density = 0
color = [255, 160, 40]
lifetime = 60
tags = ["energy", "hot"]

[materials.rock]
cell_type = "solid"
density = 90
color = [100, 100, 100]
tags = ["solid"]

[[reactions]]
input = ["lava", "water"]
output = ["rock", "steam"]
probability = 0.8

[[reactions]]
input = ["fire", "[flammable]"]
output = ["fire", "fire"]
probability = 0.05

[[reactions]]
input = ["[hot]", "water"]
output = ["_self", "steam"]
probability = 0.5
"""
    f = tmp_path / "test_materials.toml"
    f.write_text(toml_content)
    reg = MaterialRegistry(str(f))
    table = ReactionTable(str(f), reg)
    return reg, table


def test_direct_reaction_lookup(registry_with_reactions):
    reg, table = registry_with_reactions
    lava_id = reg.get_by_name("lava").type_id
    water_id = reg.get_by_name("water").type_id
    results = table.get(lava_id, water_id)
    assert results is not None
    assert len(results) >= 1
    r = results[0]
    assert r.output1 == reg.get_by_name("rock").type_id
    assert r.output2 == reg.get_by_name("steam").type_id
    assert r.threshold == threshold_u32(0.8)


def test_symmetric_lookup(registry_with_reactions):
    reg, table = registry_with_reactions
    lava_id = reg.get_by_name("lava").type_id
    water_id = reg.get_by_name("water").type_id
    results_forward = table.get(lava_id, water_id)
    results_reverse = table.get(water_id, lava_id)
    assert results_forward is not None
    assert results_reverse is not None
    assert results_reverse[0].output1 == reg.get_by_name("steam").type_id
    assert results_reverse[0].output2 == reg.get_by_name("rock").type_id


def test_tag_expansion(registry_with_reactions):
    reg, table = registry_with_reactions
    fire_id = reg.get_by_name("fire").type_id
    oil_id = reg.get_by_name("oil").type_id
    results = table.get(fire_id, oil_id)
    assert results is not None
    assert results[0].output1 == fire_id
    assert results[0].output2 == fire_id


def test_self_keyword(registry_with_reactions):
    reg, table = registry_with_reactions
    lava_id = reg.get_by_name("lava").type_id
    water_id = reg.get_by_name("water").type_id
    all_results = table.get(lava_id, water_id)
    self_results = [r for r in all_results if r.output1 == -1]
    assert len(self_results) >= 1


def test_no_reaction(registry_with_reactions):
    reg, table = registry_with_reactions
    wall_id = reg.get_by_name("wall").type_id
    water_id = reg.get_by_name("water").type_id
    results = table.get(wall_id, water_id)
    assert results is None


def test_threshold_stored(registry_with_reactions):
    reg, table = registry_with_reactions
    lava_id = reg.get_by_name("lava").type_id
    water_id = reg.get_by_name("water").type_id
    results = table.get(lava_id, water_id)
    assert all(0 <= r.threshold <= 0xFFFFFFFF for r in results)
