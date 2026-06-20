> 文档路径：`docs/proposals/2026-06-14-determinism-hardening-r1-r3.md`
> 运行时版本：Python 3.x（Phase 1）→ Godot 4.5 + C#（Phase 2）
> 最近更新：2026-06-14 (UTC+8)
> **Status**: Proposed

# 确定性加固：R1 迭代顺序 + R3 刚体桥接

> 来源：deep research 复核（`docs/reference/2026-06-14-tech-route-critical-review.md`）揪出的两个跨平台确定性硬约束。
> R1 Phase 1 立即做（代码）；R3 前瞻架构决策（Phase 2 实现，现在只定边界）。
> 用户已拍板：R3 取 **R3-A**（Teardown 式混合）；R1 取**加载期显式排序 + 防回归测试**。
>
> ⚠️ **命名约定**：本提案 R3 的刚体方案记作 **R3-A / R3-B / R3-C**，**刻意区别于**联机架构的"路线 A/B/C"（提案 §4.1：那里 A=实体进 lockstep、B=地形 lockstep+实体状态同步、C=chunk diff 流）。两套字母不同语境，勿混。R3-A/B/C 对应调研 `…deterministic-physics-netcode-survey.md` §4 的"路线 A/B/C"。

---

## 0. 背景

两者都是 **M1（Phase 2 C#）才真正爆发**的跨平台确定性问题，但性质不同：
- **R1** 在 Python 下被"dict 插入有序"掩盖，迁 C# 即翻车——**现在能低成本预防**（加载期排序 + 测试钉死契约）。
- **R3** 涉及尚未实现的刚体桥接，**现在只能定架构边界**，避免 Phase 2 误设计。

---

## Part 1 — R1：迭代顺序确定性（Phase 1 立即做）

### 1.1 问题边界（审计已确认）

sim **热路径**（`grid.update` / `try_move` / `_check_reactions`）全是 flat array + `range()` + keyed 单点查询（`reaction.get((a,b))` 不迭代）——**零顺序依赖，无需改动**。

R1 的暴露面审计：

| 位置 | 顺序依赖 | 是否 live bug |
|---|---|---|
| `core/material.py:43` `for name, props in data["materials"].items()` | type_id 按 toml dict 插入序分配 | ✅ **真 bug**：type_id 进 `cells` → 进 `state_hash`；C# `Dictionary` 无序 → type_id 全变 → 跨平台 hash 不一致 |
| `core/reaction.py:35` `for id1 in input1_ids`（input1_ids 是 **tag set**） | set 枚举序影响反应表 dict **插入顺序** | ❌ **非 live bug**（2026-06-14 核对降级，见下） |

> **A2 降级说明（核对发现）**：`for id1 in input1_ids` 生成的 `(id1,id2)` 对**互不相同**，每个 `setdefault(key,[]).append()` 落在**不同 dict key**；而 ①反应表只经 `.get((a,b))` 单点查询（`grid.py:_check_reactions`），从不迭代；②`state_hash` 只哈希 `cells`、不哈希反应表；③每 key 的 list 顺序只取决于 `[[reactions]]` 数组序（有序稳定）。故 set 枚举序在当前设计下**无任何可观测后果**，原"results append 顺序依赖 set 枚举序 → desync"高估了（append 落不同 key、非同一 list）。**裁决（用户，2026-06-14）：A2 砍掉（YAGNI），不现在加 `sorted()`**——留待 C# 迁移时作为 D3 契约一部分自然处理（若届时反应表需迭代/序列化）。

### 1.2 方案（只修 A1 真 bug，全在加载期）

1. **`material.py`**：type_id 按 **`sorted(material names)`** 分配——material name 是稳定 key，与 toml 解析顺序无关。
   ```python
   for name in sorted(data.get("materials", {}).keys()):
       props = data["materials"][name]
       ...
   ```
2. **防回归测试**（新 `tests/test_load_order.py`）：
   - **单元红绿**：fixture toml 以非字母序声明材质，断言 type_id 按 name 排序分配（去掉 sorted 必红）。
   - **D3 capstone**：用真实 `materials.toml`，断言 ①type_id 序列 == 按 name 排序的顺序；②同一文件双载、`state_hash` 序列逐帧相等。
   - 这把"加载顺序无关"从口径变成**红绿可验证契约**。

### 1.3 注意

- type_id 改为按 name 排序后，**当前 type_id 分配会变**（toml 序 `wall,rock,...` → 字母序 `fire,lava,...`），**既往 state_hash 序列作废**（语义等价变更，与历史同口径；录放/同 seed 等价测试不受影响，因它们不锚死具体 type_id 值）。
- 成本：约 1 小时，2 处 `sorted` + 1 个测试。

### 1.4 D3 契约升格

proposal §3 **D3 已补强**（2026-06-14）：sim 遍历禁裸 `Dictionary`/`HashSet`，强制稳定排序。本方案是 D3 在加载层的**测试化兑现**。M1 C# 迁移时，chunk/entity/contact 遍历必须沿用同款（`SortedDictionary` 或显式 sort），这是迁移期头号风险点。

---

## Part 2 — R3：刚体桥接确定性（前瞻，Phase 2 实现）

### 2.1 问题

Phase 2 刚体桥接（Marching Squares → Douglas-Peucker → 三角化 → Box2D）涉及几何/物理浮点。跨平台浮点不位级一致（FMA / 超越函数 / 编译器自由）+ Box2D 求解器对接触顺序敏感（Catto 亲证）→ 刚体若进地形 lockstep 域会 desync。

### 2.2 决策：R3-A — Teardown 式混合

证据见 `docs/reference/2026-06-14-deterministic-physics-netcode-survey.md`：Teardown（2026-03 上线多人）与我们几乎同构，作者明确"否定对整个引擎做确定性"，采用混合——**破坏走确定性命令流，刚体运动走状态同步**。这正是提案 §4.3 双层架构对刚体的明确表态：

```
地形 CA tick（确定性内核）  ── 整数，进 lockstep，命令流同步（各机一致）
刚体层（实体层的一部分）     ── 浮点，不要求跨机确定，状态同步 + 客户端预测
```

**核心设计点**：
1. **刚体归实体层，不进地形 tick**。各机 Box2D 各跑各的（浮点随便用），host 权威，刚体 transform/速度走**状态同步 + 客户端预测 + 插值**（≠ host 单机粗暴广播，手感接近本地）。
2. **破坏/结构变更走确定性命令流**：像素被破坏 → 刚体重算，这个"地形怎么变"由命令流确定（地形层已有机制，Teardown 同款）；"刚体怎么动"才走状态同步。
3. **量化边界 = 确定性边界**（提案铁律）：刚体写地形（压碎像素）必须走量化命令；刚体读地形（碰撞查询）用本地 lockstep 地形（各机一致）。

### 2.3 R3-B/R3-C 可选升级路径（不现在做，记录待 M2 评估）

| 方案 | 触发条件 | 做法 | 代价 |
|---|---|---|---|
| **R3-B：Box2D 3.1 默认确定** | M2 发现刚体确定性意外便宜 | 刚体也进 lockstep（Box2D 3.1 无需定点即跨平台确定，关 FMA + 全局确定接触顺序） | 编译 flag 锁死 + 全局确定接触顺序 + **放弃 rollback netcode** |
| **R3-C：全栈定点（Quantum 式）** | R3-B 仍不够 | 整个物理栈改 Q48.16 定点 | 极高，全栈改造 |

升降逻辑沿用提案 §4.4：基线 R3-A，证据支持就升。

### 2.4 Phase 1 现在要做的预防

**几乎没有**——刚体桥接 Phase 2 才写。唯一现在能做的是**文档边界**：在 `architecture.md` / 本提案写明"**刚体属实体层，不进地形 tick；刚体读地形用本地 lockstep 副本，写地形走量化命令**"，避免 Phase 2 有人误把刚体塞进确定性内核。这是决策落档，非代码。

### 2.5 诚实风险标注

- Teardown 作者自注"对全确定性的看法已松动（a view I have since reevaluated）"——hybrid 未必是终态。故 R3-A 定为**基线 + 可升级**，非锁死。
- 刚体可能成为"地形 / 弹幕 / 刚体"**三层**同步（地形 lockstep、弹幕 lockstep、刚体状态同步），而非纯双层——记入提案开放问题（已落）。

---

## 3. 行动项汇总

| # | 行动 | 落点 | 时机 |
|---|---|---|---|
| A1 | `material.py` type_id 按 name 排序分配 | 代码 | **现在**（Phase 1） |
| ~~A2~~ | ~~`reaction.py` tag 展开按 id 排序~~ → **核对降级为非 live bug，砍掉（YAGNI）** | — | C# 迁移期 D3 处理 |
| A3 | 加载顺序防回归测试（单元红绿 + D3 capstone：type_id 排序序 + 双载 hash 一致） | 测试 | **现在** |
| A4 | R3 架构边界落档：刚体属实体层、不进地形 tick | architecture.md（已基本表述，补刚体明确句） | **现在**（文档） |
| A5 | 刚体桥接按 R3-A 实现（状态同步 + 预测） | 代码 | Phase 2 / M1 |
| A6 | M2 spike 评估是否升 R3-B（刚体进 lockstep） | spike | M2 |
| A7 | C# 迁移遍历沿用 D3（SortedDictionary / 显式 sort） | 代码 | M1 |

## 4. 验收（A1–A3 即时部分）

- 全套测试绿（fresh run），含新增加载顺序防回归测试（去掉 sorted 必红）。
- 既往 state_hash 序列作废已预期（type_id 重排），录放/同 seed 等价测试仍绿。
- benchmark 无回退（加载期改动，热路径零影响）。
