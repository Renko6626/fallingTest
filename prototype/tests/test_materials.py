import pytest
from core.material import MaterialDef, MaterialRegistry


@pytest.fixture
def registry(tmp_path):
    toml_content = """
[meta]
version = 1

[materials.wall]
cell_type = "solid"
density = 100
color = [128, 128, 128]
tags = ["solid"]

[materials.sand]
cell_type = "powder"
density = 60
color = [194, 178, 128]
color_variance = 15
tags = ["powder"]

[materials.water]
cell_type = "liquid"
density = 10.0
color = [48, 96, 255]
tags = ["liquid", "water"]
dispersion = 5
"""
# 注：water 故意保留 float 写法——验证加载器 int(round()) 兼容（D1）
    f = tmp_path / "test_materials.toml"
    f.write_text(toml_content)
    return MaterialRegistry(str(f))


def test_air_is_type_id_zero(registry):
    air = registry.get_by_id(0)
    assert air.name == "air"
    assert air.cell_type == "solid"
    assert air.density == 0
    assert isinstance(air.density, int)


def test_density_is_int_even_from_float_toml(registry):
    """D1：加载器 int(round()) 兼容旧 float 写法。"""
    water = registry.get_by_name("water")
    assert water.density == 10
    assert isinstance(water.density, int)


def test_materials_loaded(registry):
    assert registry.get_by_name("wall") is not None
    assert registry.get_by_name("sand") is not None
    assert registry.get_by_name("water") is not None


def test_type_ids_unique(registry):
    ids = [registry.get_by_name(n).type_id for n in ["wall", "sand", "water"]]
    assert len(set(ids)) == 3
    assert all(tid > 0 for tid in ids)


def test_tag_index(registry):
    water_ids = registry.get_ids_by_tag("water")
    water = registry.get_by_name("water")
    assert water.type_id in water_ids
    liquid_ids = registry.get_ids_by_tag("liquid")
    assert water.type_id in liquid_ids


def test_tag_index_empty(registry):
    assert registry.get_ids_by_tag("nonexistent") == set()


def test_material_def_fields(registry):
    sand = registry.get_by_name("sand")
    assert sand.cell_type == "powder"
    assert sand.density == 60
    assert sand.color == (194, 178, 128)
    assert sand.color_variance == 15
    assert "powder" in sand.tags


def test_default_color_variance(registry):
    wall = registry.get_by_name("wall")
    assert wall.color_variance == 0


def test_default_lifetime(registry):
    wall = registry.get_by_name("wall")
    assert wall.lifetime == 0


def test_all_materials(registry):
    all_mats = registry.all()
    names = {m.name for m in all_mats}
    assert names == {"air", "wall", "sand", "water"}


def test_dispersion_loaded(registry):
    water = registry.get_by_name("water")
    assert water.dispersion == 5
    assert isinstance(water.dispersion, int)


def test_default_dispersion(registry):
    """未声明 dispersion 的材质缺省 1（= 现行为）。"""
    wall = registry.get_by_name("wall")
    assert wall.dispersion == 1
