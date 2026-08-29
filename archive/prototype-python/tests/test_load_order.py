"""R1 / D3 契约：加载是数据内容的纯函数，与 toml 声明/容器枚举顺序无关。
（A2 反应表排序经核对为非 live bug 已砍，本文件只锁 type_id + hash 契约。）"""
from pathlib import Path

from core.material import MaterialRegistry
from core.reaction import ReactionTable
from core.grid import CellGrid

TOML = str(Path(__file__).parent.parent / "data" / "materials.toml")


def test_type_ids_follow_sorted_name_order():
    """真实 materials.toml：type_id 序列严格等于按 name 排序的顺序。
    materials.toml 声明序非字母序，故去掉 sorted 必红。"""
    reg = MaterialRegistry(TOML)
    names = sorted(m.name for m in reg.all() if m.name != "air")
    for expected_id, name in enumerate(names, start=1):
        assert reg.get_by_name(name).type_id == expected_id, (
            f"{name} 应得 type_id {expected_id}，实际 {reg.get_by_name(name).type_id}"
        )


def test_double_load_state_hash_identical():
    """同一文件两次独立加载 → 跑同一场景 N 帧 → state_hash 序列逐帧相等。
    （加载是纯函数的端到端验证。）"""
    def run(frames=30):
        reg = MaterialRegistry(TOML)
        table = ReactionTable(TOML, reg)
        grid = CellGrid(48, 48, reg, table, seed=7)
        wall = reg.get_by_name("wall").type_id
        sand = reg.get_by_name("sand").type_id
        water = reg.get_by_name("water").type_id
        for x in range(48):
            grid.set_cell(x, 47, wall)
        for x in range(10, 38):
            grid.set_cell(x, 5, sand)
            grid.set_cell(x, 10, water)
        hashes = []
        for _ in range(frames):
            grid.update()
            hashes.append(grid.state_hash())
        return hashes

    assert run() == run()
