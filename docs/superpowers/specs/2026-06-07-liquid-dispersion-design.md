> 文档路径：`docs/superpowers/specs/2026-06-07-liquid-dispersion-design.md`
> 运行时版本：Python 3.x（Phase 1 原型）
> 最近更新：2026-06-07 (UTC+8)
> **Status**: Approved（用户批准，待实施）

# 液体/气体 dispersion rate 设计

## 0. 背景与目标

朴素 CA 液体横移 1 格/帧，观感"慢渗"而非"流动"。Noita 用 dispersion rate
让液体一帧沿表面横移多格（deep-dive §3.3：水 ≈5、油更低、岩浆 1–2），
是 Phase 1 玩法队列收益/成本比最高的一项（deep-dive §6 P0，估半天）。

地基依赖：2026-06-07 已修复方向承诺 bug（`rules.py` 侧移走 `-vel` 不翻转
方向记忆 → 像素打乒乓、液面冻结）。dispersion 的方向记忆复用该修复。

**用户决策（2026-06-07）**：
1. 液体 + 气体共用同一材质字段（`_move_liquid`/`_move_gas` 镜像结构，一次接两处）。
2. 探测只穿 AIR；首格为更轻液体时退回现有 ±1 密度置换（油水分层行为不变）。

## 1. 数据层

- `prototype/data/materials.toml`：材质新增整数字段 `dispersion`，缺省 **1**（= 现行为）。
  初始参数：water **5**、oil **2**、lava **1**、steam **3**；solid/powder/energy 不读该字段。
- `prototype/core/material.py`：`MaterialDef` 增加 `dispersion: int`，
  registry 加载时 `props.get("dispersion", 1)`。

## 2. 算法（`prototype/core/rules.py`）

横移段（下、斜下优先级链不动）替换为最远空格探测；液体如下，气体镜像
（方向为上、`heavier_sinks=False`）：

```
for dir in (vel, -vel):
    furthest = None
    for i in 1..mat.dispersion:
        (tx, ty) = (x + dir*i, y)
        if not grid._write_rect.contains(tx, ty): break    # 写域契约夹断
        if grid[tx, ty] == AIR:
            furthest = (tx, ty); continue
        if i == 1 and _can_move_to(tx, ty, ...):           # 更轻液体：旧 ±1 置换
            return (tx, ty)
        break                                              # 非空气：截停
    if furthest is not None:
        if dir == -vel:
            velocity = -vel                                # 方向承诺（沿用今日修复）
        return furthest
velocity = -vel                                            # 两侧全堵：翻转重试
return None
```

要点：
- 探测**纯确定**，无 RNG，结果是 `(网格状态, 坐标, vel)` 的纯函数。
- 落点 = 最远连续 AIR；中途任何非 AIR 截停（不穿液体、不穿粉末）。
- ±1 密度置换仅在 `i == 1` 保留——分层液体（水推油）回归今日修复后的行为。
- `dispersion = 1` 时与现行为逐位等价（含两侧全堵时的翻转路径）。

## 3. 不变量（确定性契约对照）

| 契约 | 论证 |
|---|---|
| 守恒 | 仍是每帧每像素至多一次 swap，无源汇 |
| D2 确定性 | 探测无随机；方向记忆是格子状态的一部分 |
| 写域契约（提案 §2.2 条件①） | 探测循环在 `write_rect` 边界 break；N ≤ 5 < margin 32，仅世界边缘裁剪生效 |
| 世代戳语义（M0.5） | swap+盖戳路径零改动（`grid.py` 不动） |
| hash 序列 | **作废**（语义变更，与 M0.5、方向承诺修复同口径），录放等价/同 seed 等价测试不受影响 |

## 4. 测试计划（`prototype/tests/test_rules.py` 为主）

1. 落最远空格：水 dispersion=5，右侧 5 格空 → 一帧落到 x+5。
2. 探测截停：第 3 格是墙 → 落 x+2（墙前最远空格）。
3. ±1 置换回归：首格是油（更轻）→ swap 油，不穿透。
4. 写域夹断：贴世界边缘探测被裁剪，不越界。
5. 气体镜像：steam dispersion=3 同款断言。
6. 摊平加速：现 800 帧收敛的盆中水柱场景 ≤ 200 帧收敛（实施时钉实测值）。
7. 既有套件全绿（守恒、replay、同 seed 等价自动覆盖语义变更）。

## 5. 验收

- 全部测试绿（fresh run）。
- 摊平帧数对比入档（CHANGELOG）。
- benchmark 双尺寸对比，回退预算 **10%**（探测循环只在下落失败的表面像素上跑）。
- demo gif 重新生成，目测水"流"起来、岩浆明显更稠。
