# M0 骨架与执法 · 实现级设计

> 文档路径：`docs/superpowers/specs/2026-08-29-m0-skeleton-design.md`
> 运行时版本：Rust（sand-core / sand-harness）
> 最近更新：2026-08-29 (UTC+8)
> **Status**: Trial（2026-08-29 用户批准设计六节 + 两项裁决：M0 即上 rayon、水走简版横流）
> 上游：`docs/overview/kernel-charter.md`（总纲）§4/§6/§11、`docs/overview/program-architecture.md`（架构）§2–§4

## 0. 目标与验收

总纲 M0 行：**chunk 寻址存储 + 四相调度器 + 沙、水两种材质 + SyncTest 双实例框架**，
外加已批准的占位 GIF 渲染器。验收：

1. `cargo test` 全绿（含 CI 级 SyncTest ≥1 万 tick 与 golden replay）。
2. `sand-harness synctest`：同场景 1 线程 vs N 线程、休眠跳过开 vs 关，10 万 tick 哈希流零分叉。
3. 双机（台式机 × 笔记本）`sand-harness hashrun` 输出逐字一致（用户手动执行）。
4. `sand-harness render` 产出沙堆 + 倒水场景 GIF，肉眼行为正确。

### 明确不做（M0 之外）

液体 dispersion 多格探测（M2 前）；反应表 / Layer F / Layer P / 刚体 / 实体；
`sand-session`（GGRS）与 `sand-bridge`；正式 bench 场景（M1 后）；InputFrame 正式编码（M4）。

## 1. Cell 与存储

### 1.1 Cell = u32（总纲 §4 "逻辑 cell ≤ 4 字节"）

| 位 | 字段 | 说明 |
|---|---|---|
| 0–7 | `material` | 材料 id；`AIR = 0` ⇒ 空 cell 全零 |
| 8–15 | `stamp` | **8 位世代戳** = 盖戳时的 `tick % 256`。扫描时 `stamp == tick % 256` ⇒ 本 tick 已更新，跳过（防同 tick 二次移动，含跨相位）。只在**移动/交换/brush 写入**时盖戳，评估被堵不盖——保证「休眠跳过开 vs 关」两种配置的戳状态逐位一致（§1.4/§5.3 的 SyncTest 前提）。 |
| 16 | `flag_dir` | 横向方向记忆（0=左 1=右）；**方向承诺不变量**：侧移成功后必须置为实际移动方向（Python 2026-06-14"液面冻结"教训） |
| 17–31 | `aux` | M0 恒 0（留给 lifetime / fire_hp / velocity 累加器） |

访问用 newtype `Cell(u32)` + 位段方法，不暴露裸位运算。

> 为什么不是 1-bit tick 奇偶位（总纲 §4 的原始表述）：cell 在 T 移动盖戳、T+1 被堵不动、
> T+2 时奇偶恰好回到 `tick & 1`，会被误跳过一拍。8 位世代戳把该假阳性稀释到 1/256 且仍
> 完全确定（Python M0.5 决策①同款结论）。属对总纲的实现级精化，非语义偏离。

### 1.2 材料 id（load-order 确定性，R1 教训）

id 在 `data/materials.ron` 中**显式声明**，不从声明顺序派生：AIR=0、WALL=1、SAND=2、WATER=3。
加载器校验 id 连续、无重复、AIR 必须为 0。

### 1.3 Chunk 与世界

- `CHUNK = 64`；`Chunk { cells: [Cell; 4096], dirty: DirtyRect, next_dirty: AtomicDirtyRect }`。
- 世界 = `width_chunks × height_chunks` 的 flat `Vec<Chunk>` + 行主序静态页表（总纲 §9 缝 1：
  API 按 chunk 寻址，后端小图即整块分配）。世界像素尺寸必须是 64 的倍数。
- 越界读返回 `WALL` 哨兵；越界写为逻辑错误（debug 断言）。
- M0 尺寸约定：单测 128×128 起；CI SyncTest 256×192（4×3 chunk，多相邻缝）；验收跑 640×384
  （总纲基准 640×360 向上取整到 chunk 倍数，底部 24 行由场景填 WALL）。

### 1.4 脏矩形与休眠

- 每 chunk 双矩形：`dirty`（本 tick 扫描范围）、`next_dirty`（本 tick 写入积累，tick 末交换清空）。
- 任何 cell 写 (x,y) ⇒ 在**所属 chunk** 的 `next_dirty` 并入 (x,y)±1 邻域；邻域越过 chunk 边界的部分
  并入邻 chunk（唤醒）。M0 单步移动 ≤1 格，±1 保守充分；引入更远移动时此常数随 r 审计。
- `dirty` 为空的 chunk 本 tick 跳过。**语义不变性由 SyncTest「跳过开 vs 关」比对执法**（§5）。
- `next_dirty` 合并用原子 min/max（可交换可结合 ⇒ 调度无关，P4 合规）。
  **这是相内唯一允许的跨任务共享写**——同相两 chunk 的 cell 写域不相交，但可同时唤醒同一个
  异相邻 chunk 的矩形元数据，故元数据必须走可交换合并，cell 本体绝不允许。

## 2. 材料表（P5 从第一天生效）

- `data/materials.ron`：`[{ id, name, category, density: u16, color: (r,g,b) }]`，M0 四条。
- **sand-core 不碰文件系统**（Ring 0）：harness 读 RON → 构造 `MaterialTable`（按 id 索引的定长数组）
  → 连同世界尺寸、seed 一起作为 `InitConfig` 传入 `core::init`。RON/serde 依赖只在 harness。
- harness 对 RON 文件内容算 xxh3 指纹，进入 replay/scenario 文件头（哈希不匹配拒绝回放——
  沿用 Python M0 的 toml-sha256 口径）。
- `category` M0 仅区分 Static（AIR/WALL 不扫描）与 Powder/Liquid 分派运动规则；
  density 驱动沙水置换（沙 40 > 水 16，沿用整数标度）。

## 3. 四相调度器 + rayon

### 3.1 tick 管线（架构 §4 的 M0 子集，顺序即协议）

1. **输入应用**：本 tick 的脚本化 brush 操作（场景文件驱动），按脚本序执行。
2. **网格四相 pass**（下述 3.2）。
3. **封帧**：交换/清空脏矩形；按需计算状态哈希。

### 3.2 相位结构

- 相 = `(cx & 1, cy & 1)`，四相。相序 = `PHASE_ROTATION[tick % 4]`（固定 const 轮换表，
  摊平边界各向异性）；相间有 rayon 屏障。
- 相内：按 (cy, cx) 固定序收集本相**活跃** chunk → `par_iter` 任意调度。每任务持有
  「本 chunk + 四周 16px halo」的写窗口：同相 chunk 中心间距 ≥128px、窗口半径 ≤16px，
  几何上两两不相交（总纲 §4 r≤16 论证的实现面）。
- 实现：窗口 = 含不变量安全注释的 unsafe 指针包装（`WriteWindow`）。**debug 构建每次
  cell 读写断言坐标落在窗口内**——Python M0.5 写域拒绝测试的 Rust 版，越界即 panic。
- 块内扫描：自下而上；行内方向按 `(y + tick) & 1` 交替；只扫 `dirty` 矩形行列范围。
- 线程数 = `InitConfig.threads`（rayon 有界池），**只影响快慢不影响结果**——由 SyncTest 执法。

### 3.3 r ≤ 16 契约

M0 实际影响半径 = 1（单步移动）。`WriteWindow` 半径常量 16 与总纲一致；新增任何移动/探测规则时
必须自证半径 ≤16 并更新 §1.4 的脏矩形扩张常数——写入本 spec 作为评审检查项。

## 4. 规则与 RNG

### 4.1 rng 模块（先于规则落地）

- SquirrelNoise5 移植（沿用 Python `archive/prototype-python/core/rng.py` 常数，金值锚定测试
  ——注意：Rust 版 key 打包不同，金值以 Rust 首次实现为准重新锚定，Python 金值仅作算法核对）。
- key = `(seed, tick, x, y, salt, stream)` 打包进 hash 输入；**`stream` 显式编码调用点序**
  （相当于旧 pass_id + attempt，总纲 §11 翻案 4：同帧同格多次掷骰必须不同流）。
  M0 流常量：`STREAM_DIAG = 0`；后续调用点在枚举中追加，禁止复用。
- API 纯函数无状态：`rng_u32(seed, tick, x, y, salt, stream) -> u32` 及 `rng_bool` 等薄封装。

### 4.2 沙（Powder）

下方为 AIR → 落；下方为密度更小的非 Static（M0 即水）→ 置换交换；否则斜下两格按
RNG 平局裁决顺序（`STREAM_DIAG` 取 1 bit 定先试侧）尝试落/置换；都不行则静止。
移动/交换双方盖世代戳。

### 4.3 水（Liquid，简版横流）

下 → 斜下（同沙的裁决）→ 横移 1 格：先试 `flag_dir` 方向，堵则试反向；**任一侧移成功后
`flag_dir` 置为实际移动方向**（方向承诺不变量，单测钉死）；两侧都堵则静止且方向位不变。

### 4.4 扫描跳过

cell 的 `stamp == tick % 256` ⇒ 本 tick 已移动过，跳过。AIR/WALL（Static）不进规则分派。
brush 写入的 cell 同样盖戳 ⇒ 产物同帧不动（沿用 Python M0.5 决策②口径）。

## 5. 哈希、SyncTest 与 golden replay

### 5.1 哈希（core 内置）

- per-chunk：`xxh3_64(cells 字节序列 ‖ cx ‖ cy)`；全局：`xxh3_64(tick ‖ 各 chunk 哈希按页表序)`。
- stamp/dir 位入哈希（它们是逻辑状态）；哈希不入状态本身。
- chunk 哈希即总纲 §3 哈希树的叶层，desync 定位（M5）与大世界（§9 缝 2）直接复用。

### 5.2 场景文件（RON，harness 侧）

`Scenario { name, world: (w_chunks, h_chunks), seed, threads_hint, ticks, setup: [Fill{...}],
script: [At{tick, Brush{material, x, y, r}}] }`——setup/script 复用同一套确定性写入路径
（Python `ops.py` 的教训：刷子与场景初始化共用代码，防"测试专用路径"分叉）。

### 5.3 harness 子命令

| 子命令 | 行为 |
|---|---|
| `synctest <scenario>` | 同进程多实例（1 线程 vs N 线程 × 跳过开 vs 关，共 4 配置）逐 tick 比全局哈希；分叉即停，报 tick + 首个不一致 chunk 坐标 + 双方 chunk 哈希 |
| `replay <scenario> [--golden <file>]` | 跑完输出终态哈希与周期哈希（每 256 tick）；`--golden` 断言与入库值一致 |
| `hashrun <scenario>` | 打印周期哈希流到 stdout（双机验收：两台机器输出 diff 为空即过） |
| `render <scenario> -o out.gif [--every K] [--scale N]` | 每 K tick 采帧 → GIF；调色板取 materials.ron color；×N 整数最近邻放大（默认 K=4、N=4） |

### 5.4 测试金字塔

1. **单测**（sand-core）：相几何互斥穷举（同相任意两窗口不相交 + 四相覆盖全图）；Cell 位段；
   规则表（沙落/堆/沉水、水流/方向承诺）；RNG 金值 + 流独立性；哈希稳定性（同状态双算同值）。
2. **CI SyncTest**（cargo test 内嵌）：256×192、≥1 万 tick、4 配置比对。
3. **golden replay**：≥2 个场景（沙堆、沙+水混合）入库 `crates/sand-harness/tests/golden/`。
4. **写域执法**：debug 断言（§3.2）+ 一个刻意越界写的 `#[should_panic]` 测试。
5. **验收**（人工）：`synctest` 10 万 tick + 双机 `hashrun` + `render` GIF 目检。

## 6. 依赖与布局

- `sand-core` 新增：`rayon`、`xxhash-rust`（均纯计算，Ring 0 合规；依赖清单仍禁 I/O/时钟/网络）。
- `sand-harness` 新增：`ron`、`serde`、`gif`。
- sand-core 模块：`cell` / `chunk` / `world` / `material` / `rng` / `rules` / `scheduler` / `hash` /
  `config`（架构 §3 Ring 0 表中 state+scheduler+grid+rng 的 M0 子集；particles/fields/stamp/
  entities/events 待各自里程碑建档）。
- 性能：M0 只在 harness 打印 tick 耗时均值/最坏参考值，正式 bench 与 `docs/perf/` Rust 基线
  M1 后建立（总纲 §7 校准）。

## 7. 风险与缓解

| 风险 | 缓解 |
|---|---|
| unsafe 窗口写出 bug | debug 全写断言 + 穷举几何单测 + should_panic 执法测试 |
| 脏矩形唤醒漏（休眠 chunk 该醒没醒） | SyncTest 跳过开/关比对为常驻配置；发现即是红 |
| rayon 引入非确定（元数据竞争） | 唯一共享写 = 原子 min/max 矩形合并（§1.4）；评审检查项 |
| 金值/哈希跨机不一致 | 全整数路径无浮点；双机 hashrun 是验收第 3 条 |
