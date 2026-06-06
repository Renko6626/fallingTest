import pytest
from pathlib import Path
from core.material import MaterialRegistry

FIXTURE_TOML = """
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
tags = ["powder"]

[materials.water]
cell_type = "liquid"
density = 10
color = [48, 96, 255]
tags = ["liquid", "water"]
"""


@pytest.fixture
def toml_path(tmp_path):
    f = tmp_path / "test_materials.toml"
    f.write_text(FIXTURE_TOML)
    return str(f)


@pytest.fixture
def small_registry(toml_path):
    return MaterialRegistry(toml_path)
