> 文档路径：`docs/sessions/2026-06-07-m0-implementation.md`
> 最近更新：2026-06-07 (UTC+8)

# Session 2026-06-07：M0 + M0.5 实施

## 1. 本次做了什么

按 superpowers 全流程：brainstorming（设计 7 节，用户批准）→ spec（`docs/superpowers/specs/2026-06-07-m0-determinism-design.md`）→ writing-plans（9-task TDD 计划）→ executing-plans（逐 task 实施，每 task commit）→ verification-before-completion（fresh 全量验证）。

**M0 完成，验收四件套全过**：

| 验收项 | 证据 |
|---|---|
| ① 污染测试（核心谓词） | `test_pollution_does_not_affect_sim` PASSED——帧间扰动全局 random，120 帧 hash 逐帧不变 |
| ② RNG 金值 | `test_golden_values` 5 锚点 + key 6 分量独立性 + "sim 禁 import random" 防回归 |
| ③ 录放等价 | `test_record_replay_roundtrip` PASSED + replay CLI 两遍输出逐字节一致 |
| ④ benchmark 入档 | 同机同场景：M0 前 27.6 FPS → 后 23.0 FPS（**-17%，预算 20% 内**），`docs/perf/baseline.md` 定版正式基准脚本 |

全套测试 **56 passed**（5.6s）。环境：新建 `venv/`（pytest 9 / numpy 2.4 / pygame 2.6）。

## 2. 实施中的修正（与计划的偏差）

- 计划排序缺口：Task 3 字段改名 `threshold` 会破坏 Task 4 才改的 `_check_reactions` → 把 grid 反应 RNG 化提前合入 Task 3 commit。
- `test_grid_seed_and_fseed` 首版语义错（fseed 在 update 开头按当前帧算，首帧与 init 预置同值）→ 改两次 update 断言，amend。
- replay headless 检查从"源码含 pygame 字符串"改为 `sys.modules` 判据（docstring 误伤）。

## 2b. M0.5 同日完成（4-pass chunk 调度器）

同流程（brainstorm 三决策获批 → spec → plan → execute → verify）。验收：69 passed；**缝隙守恒**（192×128 多 chunk，逐材质计数不变）；产物盖戳**红绿验证**（去掉 stamp 必红）；replay 在新调度器下仍确定；M0 hash 序列按预期作废（pass_id 接线 + 语义切换）。性能意外向好：128² 27.1 FPS（**较 M0 +18%**，删 O(N) 清 flag pass 收益 > 调度开销）；192² 14.0 FPS。三决策落地：①删 FLAG_DIRTY/FLAG_STATIC ②`set_cell(stamp=)` 显式参数（提案 §2.3 row 5 已同步偏离记录）③benchmark 双尺寸。

## 3. 未收尾 / 下一步

1. [ ] **用户手测冒烟**（可选）：`cd prototype && ../venv/bin/python main.py --seed 1 --record /tmp/demo.jsonl`——画沙倒水退出，再 `PYTHONPATH=prototype venv/bin/python prototype/replay.py /tmp/demo.jsonl --extra-frames 60` 看 hash。
2. [ ] **玩法队列开跑**（在 M0.5 语义上实施，deep-dive §6 收益论证 + 提案 §5 顺序）：dispersion rate（半天）→ velocity 积分（8.8 定点，注意写域契约从此真正吃力）→ fire 实施（spec v2 已就绪，burn pass pass_id=4）→ 粉末 inertia → 粒子双轨+爆炸（打击感里程碑 demo）→ 每步 benchmark 对比。
3. [ ] M1（Phase 2 C#）时复用 M0.5 语义做"1/2/4/8 线程同 hash" CI。
4. [x] 遗留小项：demo 脚本（`prototype/demo_fire.py`/`demo_gif.py`）实际早已入库；gif 已进 `.gitignore`（`*.gif`，docs 规范禁二进制图片）。CLAUDE.md §5.1 velocity 行待速度积分时更新（保留）。
