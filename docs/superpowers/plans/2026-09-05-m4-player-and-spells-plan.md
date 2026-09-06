> 文档路径：`docs/superpowers/plans/2026-09-05-m4-player-and-spells-plan.md`
> 运行时版本：Rust（内核）+ Godot 4 + gdext（表现层）
> 最近更新：2026-09-05 (UTC+8)
> **Status**: Implemented
> 分册：每个 Task 一份 `2026-09-05-m4-player-and-spells-plan-taskN.md`（索引见文末）

# M4 玩家与法术 · 实施计划（Task 1–2）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让落沙世界里出现会动的生物和会飞的投射物——生物能跑跳、踩得住刚体、排开水、被火烧死；三条法术原语（直射 / 爆炸 / 喷射）能打出去并改变世界。

**Architecture:** 架构 §4 规范 tick 管线的第 2 步"实体与法术"从空占位变生效，内部分四个子步骤（输入 → 生物运动学 → 弹体 → 施法），插在 ops 与刚体相之间。弹体独立于粒子池，复用 `dda.rs`/`fixed.rs` 两个模块。全部整数/定点，零浮点、零超越函数。

**Tech Stack:** Rust（`sand-core` 纯库 / `sand-harness` CLI）、RON 数据表、xxHash 状态哈希、rayon 有界并行（本计划新增代码全部串行）。

**Spec:** `docs/superpowers/specs/2026-09-05-m4-player-and-spells-design.md`

---

## Global Constraints

摘自总纲 §2/§6 与 spec §7.1，**每个 Task 的验收隐含包含本节**：

- `sand-core` 不依赖 gdext / 网络 / 文件系统 / `std::time`。世界演化 = （状态，输入）的纯函数。
- 网格逻辑纯整数；自研运动学用 `Fx`（Q16.16）定点。**核心禁用系统数学库超越函数**——BAM 角 → 方向向量必须查表。
- 一切逻辑随机 = `rng::rng_u32(fseed, stream, x, y, salt, attempt)`。禁全局顺序消费的 RNG 流。同帧同源的多次掷骰必须靠 `salt`/`attempt` 区分（总纲 §11 翻案第 4 条）。
- 禁 std `HashMap`/`HashSet` 默认 hasher（clippy `disallowed_types` 执法）。一切影响状态的遍历必须定序。
- 数据驱动：法术/生物走 RON 表，禁 if-else 硬编码。RON 写十进制小数，**加载期一次性量化**为整数或 `Fx`（沿用 `quantize_fx` / `quantize_splash_chance` 体例）。
- 一切算术走 `wrapping_*`（`fixed.rs` 已有纪律），保证 dev/release profile 位级一致。
- 限流常量两端必须一致，超限**确定性拒绝、不排队**：`MAX_CREATURES = 16`、`MAX_PROJECTILES = 4096`、`max_displace_per_tick`（模板字段）。
- `MAX_SLOTS = 4`（loadout 槽位数）。
- 完成任何"已通过 / 已修复"断言前必须先跑命令验证（`cargo test` / `cargo clippy`），不得凭推断。
- 每个 Task 结束时提交一次，commit message 结尾附：
  ```
  Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_017GZJc7RrRKLh4XgoJMUb4m
  ```

---

## File Structure

| 文件 | 动作 | 职责 |
|---|---|---|
| `crates/sand-core/src/input.rs` | 新建 | `InputFrame` 定义与位打包编解码 |
| `crates/sand-core/src/fixed.rs` | 改 | 追加 `Bam` 类型 + 1024 项 sin 表 + `dir_of` |
| `crates/sand-core/src/sin_table.rs` | 新建（生成物） | 1024 个 Q16.16 sin 值字面量，`include!` 进 `fixed.rs` |
| `crates/sand-core/src/creature.rs` | 新建 | 生物表：模板、运动学、扫掠碰撞、排开、游泳、接触伤害、HP |
| `crates/sand-core/src/projectile.rs` | 新建 | 弹体表：积分、DDA、命中结算、侵彻、弹跳 |
| `crates/sand-core/src/spell.rs` | 新建 | 法术表 + loadout + 施法闸门 + 三原语派发 |
| `crates/sand-core/src/material.rs` | 改 | 抽出共用硬格谓词 `is_solid` |
| `crates/sand-core/src/body.rs` | 改 | `is_hard` 改为调用 `material::is_solid(.., false)`；新增单点冲量 API |
| `crates/sand-core/src/hash.rs` | 改 | 新增 `combine4` |
| `crates/sand-core/src/lib.rs` | 改 | `Sim` 持新表、`step` 签名扩展、第 2 步四子步骤接线 |
| `crates/sand-core/src/world.rs` | 改 | `Op::SpawnCreature` |
| `crates/sand-harness/src/scenario.rs` | 改 | `creatures.ron` / `spells.ron` 加载与指纹；场景 `inputs` 时间线 |
| `crates/sand-harness/src/{runner,render,main}.rs` | 改 | `step` 调用点补 `inputs` |
| `data/creatures.ron` / `data/spells.ron` | 新建 | 生物模板 / 法术表 |
| `data/scenarios/duel.ron` | 新建 | 验收场景 |
| `crates/sand-core/tests/creature_behavior.rs` | 新建 | 生物行为测试 |
| `crates/sand-core/tests/projectile_behavior.rs` | 新建 | 弹体与法术行为测试 |

---

## Task 索引

按序执行，每份是一个独立可评审、可交付的单元。**每份都以本文的 Global Constraints 为隐含验收项。**

| # | 文件 | 交付物 | 关键闸门 |
|---|---|---|---|
| 1 | `...-plan-task1.md` | 管线与签名骨架（InputFrame、BAM 查表、`combine4`、harness `inputs` 时间线） | **零行为变化**：6 场景 `--grid-only` 哈希流逐位不变；golden 重录一次 |
| 2 | `...-plan-task2.md` | 生物本体与运动学（逐轴扫掠、跨台阶、踩得住刚体） | 硬格谓词抽出为纯搬移，既有 body 测试原样绿 |
| 3 | `...-plan-task3.md` | 生物与世界互动（排开 / 游泳 / 材质接触伤害 / HP 墓碑） | 排开走 M3 同一条脱格通路；水量不得凭空减少 |
| 4 | `...-plan-task4.md` | 弹体载体（SoA 表、DDA 命中、`Bolt` 结算） | 弹体表体例照 `particle.rs`；测试法术表由 `SpellTable::from_defs` 就地构造 |
| 5 | `...-plan-task5.md` | 法术表与施法（`spells.ron`、cooldown + mana 双闸门、`Blast`/`Spray`） | 闸门不通过时**零副作用**；`STREAM_SPREAD` 三维度齐全；golden 因指纹行再重录一次 |
| 6 | `...-plan-task6.md` | 弹体七项扩展（侵彻 / 弹跳 / 阻力 / 穿透 / 排开 / 冲量 / 定时爆） | 侵彻复用 `explode` 抽出的 `destroy_cell`；`dda` 加 `last_axis` 但既有调用方零改动 |
| 7 | `...-plan-task7.md` | 收口（`duel.ron`、SyncTest、分布回归、bench、文档全套） | 六配置零分叉 + 线程 1/8/16 逐位相同；既有场景性能不得回退 |

**执行顺序不可打乱**：1 必须最先（它一次做完全部签名 churn 与哈希结构变更，
后续 Task 不再动既有调用点与 golden 之外的既有文件）；4 依赖 2/3 的生物表；
5 依赖 4 的 `SpellTable` 本体；6 依赖 5 的法术字段全集。
