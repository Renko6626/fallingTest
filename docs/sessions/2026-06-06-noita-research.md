> 文档路径：`docs/sessions/2026-06-06-noita-research.md`
> 最近更新：2026-06-06 (UTC+8)

# Session 2026-06-06：Noita 深度调研

## 1. 本次做了什么

- 4 个并行网络调研 subagent（效果全景 / 运动学扩展 / 材质与火系统 datamine / 刚体+多线程+流式），共 ~58 次网络操作。
- 对照 prototype 现状（`prototype/core/*.py`、`materials.toml`、fire spec、parallel 文档）。
- 产出 `docs/reference/noita-deep-dive.md`（调研报告，410 行）。
- 首次建立 `docs/CHANGELOG.md` 与 `docs/sessions/`。
- 应用户质询做可靠性抽查：对 5 组承重来源逐字核验（80.lv / macuyiko / jason.today / FSS issues / materials.xml dump），4 组全过；删除 1 条伪引语，"无温度场"结论改为结构证据支撑（报告 §7 抽查记录）。

## 2. 关键结论（详见报告 §0）

1. 核心循环骨架与 Noita 一致，差距在**运动学**：速度/重力积分（1 格/帧 → 多格+32px 上限）+ CA↔粒子双轨（打击感核心）。
2. **Noita 无温度场**——我们的 fire spec（温度场+传导）是自创，有性能风险，待裁决（报告 §5.3 建议：Noita 式优先，传导降级实验分支）。
3. 既有 `parallel-update-strategies.md` 表述基本正确，两处待精化（十字写域；Margolus 非 Noita）。

## 2b. 第二轮：联机与确定性调研（同日追加）

- 又 4 路并行调研（Noita 本体确定性 / NT+NoitaMP / Entangled Worlds+Arena / Factorio·Teardown 等先例），共 ~63 次网络操作，一手来源为主。
- 产出 `docs/reference/noita-multiplayer-and-determinism.md`（调研）+ `docs/proposals/2026-06-06-deterministic-parallel-and-netcode.md`（策略，Status: Proposed）。
- 精化 `docs/algorithms/parallel-update-strategies.md`（十字写域原话、Margolus 非 Noita、64/512 双层、确定性 caveat）。
- 核心结论：并行不必然破坏确定性（写域互斥+读域夹断+counter RNG 三条件）；Noita 本体大概率不确定（无 TAS/replay，NEW 被迫做像素状态同步）；推荐"地形 lockstep + 实体状态同步"，M0（counter RNG + state hash + demo 回放，~2 天）应最先落地。

## 2c. 四项裁决（同日，用户拍板）

- fire spec：**Noita 式优先**（温度场降级实验分支）——spec 头部已加裁决横幅。
- M0 确定性地基：**批准，排 Phase 1 队首**。
- 联机目标形态：**coop + 小规模 PvP**（M2 spike 需加对称竞技场景，lockstep 延迟掩盖权重上调）。
- 旧火焰调参：已留档 commit `b99b2ec`。提案 Status → Trial。

## 2d. 外部评审采纳（同日，用户提供的 GPT 审阅）

6/6 成立并采纳：①RNG key 升级 7 元组（修"确定但强相关"）；②插入 M0.5 单线程 4-pass 语义原型；③D1 整数化细则（density 整数、概率 u32 阈值）；④实体连续占位升级为提案 §4.3 一等规则（量化实体快照 = 地形 tick 输入）；⑤fire spec 全文重写 v2（自相矛盾消除，另补延迟点燃队列防扫描偏置）；⑥M0 队首为评审确认项。docs 第一批已入库 `10c35ee`。

## 3. 未收尾 / 下一步（评审后定型的执行队列）

1. [ ] **M0**（~2 天）：counter RNG（完整 key）替换全部 `random.*` + per-chunk/world hash + pytest 确定性回归 + demo 录制回放 + D1 加载层整数化（density 整数等级、概率 u32 阈值）。
2. [ ] **M0.5**（~1–2 天）：Python 单线程 4-pass/chunk 调度原型——只锁语义（写/读域夹断、世代计数器、固定 pass 序），不并行。
3. [ ] Phase 1 玩法队列（在 M0.5 语义上实施）：dispersion rate → velocity 积分（8.8 定点）→ fire 实施（spec v2 已就绪）→ 粉末 inertia → 粒子双轨+爆炸（打击感里程碑 demo）→ benchmark + per-chunk dirty rect。
4. [ ] 实施期按需补查：GDC 视频 9:20 多线程段落与 23–30min 粒子段落（线程池细节、粒子弹出阈值的唯一剩余来源）。
5. [ ] 本轮评审采纳修订（proposal + fire spec v2 + 账本）尚未 git 提交——待用户确认。
