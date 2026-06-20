> 文档路径：`docs/reference/2026-06-14-deterministic-physics-netcode-survey.md`
> 运行时版本：Godot 4.5 + C#（Phase 2 议题）
> 最近更新：2026-06-14 (UTC+8)

# 跨平台确定性物理 / 刚体联机方案调研（deep research）

> 针对 R3（刚体桥接浮点确定性）的专项调研：为"host 单机权威刚体再状态同步"找更优雅且经过验证的替代。
> 5 角度 / 23 源 / 101 claim / 25 条对抗式验证（**23 confirmed / 2 killed**，几乎全一手来源）。
> 与 `2026-06-14-tech-route-critical-review.md`（路线总复核）互补——本篇专钻刚体/物理确定性这一点。

---

## 0. 一句话结论

> **工业主流是"混合架构"**——离散破坏走确定性命令流、刚体运动走状态同步——**而非"全引擎 lockstep"或"纯 host 权威"**。Teardown（2026-03 上线多人）是与我们几乎同构的直接商用先例，其分层正好沿我们 §4.3 双层切。所以"刚体走状态同步"**是合理默认而非无奈兜底**；更优雅的替代（刚体也进 lockstep）生产可行但成本转移到别处。

---

## 1. 最强证据：Teardown 混合架构（与我们同构）⭐

Teardown 2026-03 上线多人，作者 Dennis Gustafsson 一手工程复盘。体素破坏 + 刚体桥接，与 fallingTest 几乎同构。

> 原话："I dismissed the idea of full determinism for the entire engine... a hybrid approach: destruction done deterministically, while most other things use state synchronization."

| 层 | Teardown 做法 | 对应 fallingTest |
|---|---|---|
| **破坏**（改变结构/内容） | **定点整数确定性命令流**："cut hole at voxel x,y,z" / "change ownership of shape" / "reconnect joint"，可靠有序通道按相同顺序回放 → bit-identical | = 地形 CA 走 lockstep 命令流（提案路线 B 地形层）✅ |
| **刚体运动**（transform/速度/玩家位置） | **状态同步 + 本地预测 + server 修正 + 最终一致**（不可靠通道，可见 snapping） | = 刚体走实体层状态同步 |

- 证据（3-0）：voxagon 作者博客 + 80.lv 二手印证。
- **对 R3 的直接含义**：①"刚体走状态同步"经商用验证，是合理默认；②正确形态不是"host 单机跑再广播"，而是**状态同步 + 客户端预测**（手感接近本地）；③破坏/结构变更走确定性命令流，正是我们地形层计划。
- ⚠️ 时效 caveat：作者自注 "a view I have since reevaluated"——他对全确定性的看法已松动，暗示 hybrid 未必是终态。

## 2. 跨平台确定性物理的现状（2024-2026，比预期成熟）

### 2.1 Box2D 3.1：默认即跨平台位级确定（意外，无需定点）
- 证据（3-0，官方 FAQ + Catto 博客）："Box2D has cross-platform determinism as of version 3.1"。
- 手段：避免 fast-math、关 FMA（`-ffp-contract=off`）、自实现 `atan2f/sinf/cosf`——Catto 明确 "a much better result than fixed-point math which would likely be much slower"。
- 验证：x64+ARM、MSVC/GCC/Clang，Apple M2 vs AMD Ryzen bit-identical。
- **硬约束**：① 隐含编译 flag 前提（关 FMA/fast-math、SIMD 一致或 `BOX2D_DISABLE_SIMD`）；② 需同二进制/同输入/固定步长/**确定接触顺序**（呼应 R1：contact array 顺序敏感）；③ **不支持 rollback determinism**——适合 lockstep，不适合 rewind 式 rollback netcode。

### 2.2 Rapier（Rust）：跨平台确定是可选项，代价是放弃并行
- 证据（3-0）：默认仅本地确定；跨平台需开 `enhanced-determinism` feature，**与 `simd`/`parallel` 互斥**（放弃 SIMD 与多线程），且仅在严格 IEEE 754-2008、指针≥32-bit 平台有界保证。
- godot-rapier-2d 提供两个独立安装变体："Faster Parallel SIMD" vs "Slower Cross-Platform Deterministic"——确认确定性变体独立且更慢。
- ⚠️ "Rapier 产出 byte-identical 序列化状态"这一更强主张**被验证驳回（1-2）**——是有界条件保证，非通用。

### 2.3 Photon Quantum：全栈定点 lockstep 已出货
- 证据（3-0）：FP（Q48.16 定点）替换所有 float/double，**物理与导航全用 FP**；predict-rollback 网络（只传输入、模拟即权威，不必等最慢客户端）。
- 出货：Stumble Guys（**32 玩家全摆动物理**）、Motion Twin Windblown、Unity Verified Solution。
- **代价**：必须放弃 float、全系统改 Q48.16 定点。

## 3. lockstep 的奠基先例与脆弱性（佐证我们的契约严格性）

- **Age of Empires《1500 Archers on a 28.8》**（3-0）：input-sync lockstep 的奠基论文。核心动机正是反"状态同步"——传 per-unit 状态（X/Y/status/facing/damage）**带宽上限仅 ~250 单位**，只传命令可支撑数千。→ 我们整数 CA 地形可直接套此模型。注意：AoE 是 P2P 确定性，host 仅做 speed-control。
- **lockstep 极脆弱**（3-0）：任何微小分歧随时间放大成 out-of-sync 即"游戏停止"；需全量 checksum + 同步 RNG seed；"code path 必须各机一致，不依赖任何本地因素"。→ 正是浮点刚体（跨 CPU/编译器结果有别）在 lockstep 中的根本危险，也解释了为何 Box2D 要关 FMA、Quantum 要定点。这条**佐证我们 D1–D10 契约的严格性是必要的，不是过度工程**。

## 4. 三条路线对比（R3 决策依据）

| 路线 | 刚体确定性 | 代价 | 先例 | 定位 |
|---|---|---|---|---|
| **A. Teardown 式混合**（推荐基线） | 刚体不要求确定，走状态同步+预测 | **低** | Teardown 直接同构 ✅ | **工业主流**；= 我们 §4.3 双层现状 |
| **B. Box2D 3.1 默认确定** | 刚体进 lockstep | 中：编译 flag + 全局确定接触顺序 + **放弃 rollback** | Box2D 官方 | **可行但少见**；意外地无需定点 |
| **C. 全栈定点（Quantum 式）** | 全物理定点进 lockstep | **高**：整个物理栈改 Q48.16 | Stumble Guys 出货 | 可行但高代价 |

## 5. 对 fallingTest 的结论

1. **"host 权威刚体"是合理默认，不是无奈**（Teardown 商用验证）——但"host 单机跑再广播"的描述太粗，正确形态是 **Teardown 式状态同步 + 客户端预测**。
2. **真有更优雅替代（B/C），但成本都转移到别处**：B 要全局确定接触顺序 + 放弃 rollback；C 要全栈定点改造。
3. **对横版动作权衡**：刚体数量少、活跃区域 ≈ 同屏，A 的 "snapping" 几乎不可见，B/C 的额外成本未必值。**建议路线 A 为基线，B 留作"若刚体确定性意外便宜则升级"的可选项**（呼应提案 §4.4 升降路径思路）。
4. **R1 关联确认**：Box2D 的"确定接触顺序"要求 = R1 迭代顺序契约在物理层的同款体现——D3 契约补强对刚体层同样适用。

## 6. 开放问题（本轮未取得通过验证的 claim）

- Noita/Nolla Games 是否有刚体桥接确定性/联机的一手披露？**Noita 是单机游戏，很可能根本无联机确定性披露**——需直接核实（GDC 2019 本轮未产出通过验证的 claim）。
- 其他破坏性物理游戏（Rust / 7 Days to Die / Space Engineers / Besiege）的刚体同步策略未取得通过验证 claim，无法判定 hybrid 是否更普遍。
- 把我们 marching-squares→Box2D 刚体层也定点化以实现端到端 lockstep，相对 Teardown 式 hybrid 的成本/收益——文献无现成答案，需原型对比。
- falling-sand + 刚体 + 联机的学术/开源具体实现——本轮未产出。

## 附：被驳回论断（禁引用）
- "Rapier 产出 byte-identical 序列化状态"（1-2）——实为有界条件保证。
- "float 转换 100% desync"（1-2）——措辞过强。
