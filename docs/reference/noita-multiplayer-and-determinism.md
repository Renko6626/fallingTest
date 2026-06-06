> 文档路径：`docs/reference/noita-multiplayer-and-determinism.md`
> 运行时版本：调研文档（服务联机架构决策，Phase 2+）
> 最近更新：2026-06-06 (UTC+8)

# Noita 多线程细节、确定性现状与联机模组调研

4 路并行网络调研（Noita 本体 / Noita Together+NoitaMP / Entangled Worlds+Arena / 同类游戏先例）的综合报告。本轮调研纪律：优先一手来源（GitHub 源码/issues/官方 devblog），承重结论给逐字引语；置信度标注沿用 `docs/reference/noita-deep-dive.md` 的体系（[官方] / [社区] / [通用] / [推测] / 未确证）。

配套决策文档：`docs/proposals/2026-06-06-deterministic-parallel-and-netcode.md`（策略与论证）。

---

## 0. TL;DR

1. **Noita 多线程的公开资料已挖尽**：4-pass 棋盘格 + 64×64 chunk + 十字写域 + per-chunk dirty rect 就是全部公开细节；线程池规格、barrier 实现、粒子/刚体是否并行、材质分频更新——所有二手转述圈均无，只剩 GDC 视频原片（9:20 起）和逆向两条深挖路径。
2. **Noita 世界生成是确定性的，但模拟过程大概率不是**：set-seed speedrun 分类证明前者；无 TAS、无 replay 功能、真联机模组放弃输入同步改传像素状态——三条旁证一致指向后者。没有任何来源记载 Noita 的 RNG 架构。
3. **模组光谱四档**：Noita Together（平行世界，不碰地形同步，活到今天）→ NoitaMP（真同步尝试，4 年 WIP 后 2025-10 归档，卡死在"引擎不暴露世界状态"）→ Entangled Worlds（成功的像素级状态同步：chunk authority 状态机 + RLE delta）→ Noita Arena（小竞技场，事件复制路线）。
4. **同类先例三条路线**：Factorio 全 lockstep（确定性工程金标准）/ Terraria server-auth tile diff（地形低频变化才适用）/ **Teardown 官方 MP 确定性命令流混合（与我们场景最像）**。但 falling sand 的地形**持续自演化**（沙一直在流）使纯命令流不够——地形层必须 lockstep 化或 diff 化，这是我们与 Teardown 的本质差异。
5. 对自研引擎最值钱的两件资产：**Entangled Worlds 的同步协议设计**（u16 像素、RLE-of-Option 增量、五态 authority 状态机、host 兜底存储）和 **Factorio 的确定性工程体系**（分层 CRC、desync report、late-join 流程）——都可以直接抄。

---

## 1. Noita 本体：多线程细节补全

### 1.1 已确证事实（汇总，引语见 noita-deep-dive.md §4.1）

| 事实 | 来源 |
|---|---|
| 64×64 模拟 chunk + per-chunk dirty rect | [官方]（80.lv，已逐字核验） |
| 4-pass 棋盘格："we do this 4 times... every other 64×64 chunk" | [官方]（80.lv，已逐字核验） |
| 写域 = chunk + 四正方向各 32px 十字（"plus 32 pixels in each cardinal direction"） | [官方]（80.lv，已逐字核验） |
| 单帧位移上限 32px（"We guarantee that no pixel can be moved more than 32 pixels away"） | [官方]（macuyiko 转述 GDC，已逐字核验） |
| 单缓冲是多线程方案的前提（"There's no two buffers"） | [官方]（macuyiko 转述 GDC，已逐字核验） |

### 1.2 未公开细节清单（本轮专项搜索，全部无果）

| 问题 | 调研结果 |
|---|---|
| worker 线程数 / 线程池设计 | 未确证——80.lv 与 macuyiko 抓取均明确无此信息 |
| pass 间 barrier / 同步原语 | 未确证 |
| 粒子系统 / 刚体是否多线程 | 未确证 |
| 主线程职责划分 | 未确证 |
| 材质分频更新（液体每帧/气体隔帧等） | 未确证，无任何来源提及 |
| 玩家周围实际模拟范围（几屏/多少 chunk） | 仍未确证；macuyiko 仅有 "probably... chunks close enough to the player" 的博主猜测 |

> 旁注：Steam 论坛存在玩家帖标题 "Please make the physics multi threaded."——暗示部分玩家体感认为发售版多线程不充分，与 GDC 描述存在张力。仅标题级证据，不构成结论。[社区传闻]

**结论：文字转述圈已挖尽。** 再深只有两条路：GDC 视频原片多线程段落（约 9:20 起，[YouTube](https://www.youtube.com/watch?v=prXuyMCgbTc) / [GDC Vault](https://www.gdcvault.com/play/1025695/Exploring-the-Tech-and-Design)）逐句听记，或对游戏本体逆向。对我们设计并行方案而言，已确证的事实已经足够（见提案 §2）。

---

## 2. Noita 模拟是确定性的吗？

**结论：世界生成 = 确定（强证据）；模拟过程 = 无正面证据，旁证一致偏向"否"或"无人依赖它"。**

| # | 证据 | 方向 | 强度 |
|---|---|---|---|
| 1 | speedrun.com/noita 存在 **Set Seed / Random Seed 两套分类**（WR 视频标题逐字："Noita Any% Set Seed Speedrun in 0:2:35 [PB]"） | 世界生成确定 ✅ | [社区强证据] |
| 2 | 官方 **Daily Run**：全球玩家 24h 内同 seed 比成绩 | 世界生成确定 ✅ | [社区，摘要级] |
| 3 | speedrun.com **无 TAS 分类**，社区无 Noita TAS 工具链（两轮针对性搜索零命中）。TAS 前提是"同输入回放必得同结果" | 模拟确定性 ❌（缺位旁证） | [社区强证据] |
| 4 | **无 replay / 输入录像功能**的任何证据 | 模拟确定性 ❌（缺位旁证） | 未确证（未发现存在证据） |
| 5 | **最强旁证**：Entangled Worlds 的同步项 README 逐字包含 "**Pixels of the grid world**"——真联机模组把像素本身当网络对象传输。若模拟跨机可确定复现，正确做法是只传输入（lockstep）；实际传像素，说明作者实测下模拟跨机发散 | 模拟确定性 ❌ | [社区强证据，推断] |
| 6 | 两代联机模组（NT 平行世界、NEW 状态同步）**都没走确定性重模拟路线** | 模拟确定性 ❌ | [社区] |
| 7 | Noita 的 RNG 架构（全局顺序流 vs per-chunk/per-cell）**无任何来源记载**——这是决定模拟确定性的关键未知量 | — | 未确证 |

**对我们的含义**（详细论证见提案）：不要指望"Noita 式引擎天然可 lockstep"。Noita 不确定是因为**它不需要确定**（单机游戏）；4-pass 棋盘格架构本身不是障碍——只要 RNG、读写域、遍历顺序按确定性契约设计，同一架构可以做到位级确定。这恰是自研引擎相对 mod 的根本优势。

官方对多人游戏的表态：**未找到可引用原话**（AMA/访谈/HN 检索均无果）。可确证的客观事实：官方从未出过任何 MP 功能，社区用三代模组填补。后续线索：[Fandom 官方访谈索引](https://noita.fandom.com/wiki/Official_Interviews_and_Videos)、Steam 论坛 [Multiplayer?](https://steamcommunity.com/app/881100/discussions/0/3785877016607975751/) 帖。

---

## 3. Noita 联机模组光谱

### 3.1 Noita Together —— "平行世界"路线 [README 直证]

- **设计定位原话**："think of it more like everyone is in different dimensions yet you can still see other players and somewhat interact with each other"；"you can not directly affect other player's worlds"。
- **同步范围**：各玩家独立世界实例；同步玩家幽灵位置 + 元数据（金币/perk 投票/物品互送等，逐项清单未能从一手来源核实）。**像素世界完全不同步**——这是设计定位而非未完成项。
- **架构**：三层——游戏内 Lua mod ↔ Electron 桌面伴侣 app ↔ WebSocket+protobuf 中心服务器（Twitch OAuth + PostgreSQL）。Noita 的 Lua mod 沙箱无原生 socket，网络栈整体外置。
- **状态**：v0.11.6（2024-12），低频维护，社区事实标准的入门联机方案。

### 3.2 NoitaMP —— 真同步尝试的撞墙史 [作者直述]

目标："First synchronous multiplayer mod for Noita!"。LuaJIT + eNet + sock.lua，网络层重写 3 次。**2025-10-26 作者归档，约 4 年始终 "WIP! Not working, atm!"**。

撞墙原话（Issue #39 "World synchronisation"）：

> "Atm the world is synced by **quitting the game**, because the world is saved to disc and can be sent as a zip/archive by network."
>
> "I also had the idea to read all world files (which are binary files) in lua... but I am not sure if the clients will read those files whenever those are changed."
>
> "@willjallen already had a quick look **in memory** to get the world shown..."（被迫读进程内存逆向）

死因解剖：引擎不暴露世界状态 → 只能退出存档发 zip / 读二进制 / 读内存三选一；外加单人长跑 + 质量债（Issue #133："If you do it quick'n'dirty, your code quality will suffer!"）。

### 3.3 Noita Entangled Worlds —— 成功的像素级状态同步（重点）[源码直证]

"True coop multiplayer mod for Noita."（repo tagline）。目前最成功的同世界协作实现。

**架构**：`noita_proxy/`（Rust 独立进程：网络中枢、lobby、world sync 状态机、chunk 存储）+ `quant.ew/`（Lua mod）+ `ewext/`（注入 Noita 的 Rust 原生扩展）+ `shared/`（协议类型）。网络走 Steam networking（lobby code）或直连 ip:port。

**像素同步协议**（`shared/src/world_sync.rs`，逐字）：

| 项 | 设计 |
|---|---|
| 网络 chunk | `pub const CHUNK_SIZE: usize = 128;`（128×128，= 2× Noita 模拟 chunk） |
| 像素编码 | `pub struct Pixel(u16);` —— 12 bit 材质 ID（上限 4096 种）+ 4 bit flags。**velocity/lifetime 等模拟态不上网**，落地后由本地 CA 继续演化 |
| 全量快照 | `ChunkData { runs: Vec<PixelRun<Pixel>> }`（RLE 行程编码） |
| 增量 | `ChunkDelta { runs: Vec<PixelRun<Option<Pixel>>> }` —— **RLE-of-Option**：未变像素 = `None` 行程，变化像素 = `Some(pixel)` 行程；增量与全量共用同一 RLE 管道 |

**Authority 模型**（`noita_proxy/src/net/world.rs` + `docs/distributed_world_sync.drawio`）：

- chunk 级五态状态机：`Unsynced → Request authority → Authority`（本地模拟该 chunk）或 `→ Waiting for chunk → Listener`（只收增量）。
- 冲突 = 优先级抢占：host 仲裁，`priority_state > priority && !can_wait` 触发权威转移，原持有者交出 chunk 数据 + listener 集合（drawio 原话："Authority taken (because higher priority) (And send existing state to new authority)"）。优先级语义≈玩家与 chunk 的空间相关性（"每人对自己周围有权威"，此层为合理推断）。
- **host 只做仲裁 + 兜底存储**（`chunk_storage: FxHashMap<ChunkCoord, ChunkData>`），不模拟全图——host 算力不随玩家数爆炸。
- 实体同步同构：DES（Distributed Entity Sync），"spatial authority model where clients gain authority over entities near them"（[社区]级源码导读）。
- 出错/掉权威是状态机一等公民（"Retry" / "On error" 回 `Unsynced`）。

**已知缺陷**：issue #166 "several desync bugs on long run while doing sun quest"；仓库维护 `docs/perks_that_dont_work_properly.md`——**作者自己承认部分游戏机制无法 100% 联机化**。玩家数上限与带宽数字未公开。

### 3.4 Noita Arena（EvaisaDev）—— 事件复制竞技场 [README 直证 + 推断]

- 底层框架 `evaisa.mp`（"Noita Online"）："A multiplayer gamemode framework for Noita using SteamAPI"，封装 Steam networking/matchmaking/lobby。
- 玩法：rounds 制 PvP 小竞技场 + 自定义地图 + 回合间卡牌成长。
- 同步模型技术细节**未公开**；从产品形态推断为各端本地模拟地形 + 复制施法/位置事件（[推测]，无文字直证）。

### 3.5 光谱总结

| 模组 | 地形同步 | 模型 | 结局 |
|---|---|---|---|
| Noita Together | 无（平行世界） | 元数据同步 | 存活，事实标准 |
| Iota Multiplayer | 无（本地分屏） | 单机共屏 | 存活（小众） |
| NoitaMP | 试图全同步 | 存档 zip 搬运 | **归档（失败）** |
| Entangled Worlds | 像素级 | chunk authority + RLE delta | **存活（成功）** |
| Noita Arena | 推断不同步 | 事件复制 | 存活（PvP 细分） |

规律：**同步野心与引擎可访问性必须匹配**。在不可改引擎上，绕开地形（NT）或重型逆向+状态同步（NEW）是仅有的活路；NoitaMP 死于两头不靠。我们自研引擎没有这堵墙——但前提是把"chunk 序列化/增量/权威"当 day-1 公民设计。

---

## 4. 同类游戏先例

### 4.1 Factorio —— 确定性 lockstep 金标准 [官方 FFF 博客]

- 模型："sending only the inputs"（FFF-188）；"all players' games need to simulate every single tick of the game identically"（官方 wiki）。client-server 中继输入（FFF-149 起弃 P2P）。
- **Desync 检测**：replay/调试模式**每 tick 全图 CRC**；日常多人跑轻量 heuristic CRC。官方原则："Networking-, latency or performance problems do **not** cause desyncs"——desync 一律是模拟 bug，不甩锅网络。
- **Desync report**：分歧即自动打包 server+client 双方状态 zip 供二进制 diff；内部分支在存档里插可读 tag 定位分歧代码段。
- **Late join**（FFF-149 原话）："Map upload is done in the background... The new player is also receiving all the player input, that is saved... it tries to update it as fast as possible to catch up with the server."——后台传档 + 输入缓存 + fast-forward。
- **踩坑实录**（FFF-340）：① Lua 序列化库用 nil 当占位符 → table 键被删 → **迭代顺序漂移**；② unit group 缓存的 max speed **不入存档**、load 时重算 → 新入玩家与老玩家分歧。两案直接映射到我们的 dict 顺序与 `is_static`/dirty-rect 缓存。

### 4.2 Teardown 官方多人 —— 确定性命令流（与我们最像）[开发者博客，Dennis Gustafsson 2026-03]

- 2021 naive 实验直接同步被破坏 voxel 数据 → "used enormous amounts of bandwidth and completely choked the connection"。定调："**Sending large amounts of voxel data wasn't an option because of bandwidth**"。
- 正式方案原话：

> "Each breakage event is split into a stream of **deterministic commands** that are replicated on all clients: 'cut hole in this shape at voxel coord x,y,z', 'change ownership of that shape', 'reconnect joint to this shape', etc."

- 配套工程：**只把破坏子系统重写为 fixed-point 整数运算**（局部确定性，不要求全引擎确定）。
- 双通道混合：场景修改 = reliable ordered 命令流（确定性）；物体 transform/玩家位置 = unreliable 状态同步 + eventual consistency，按可见性排优先，预算 "**one Mbit per client**"；低优先级修正会有 "visible snapping"。player hosting。
- 开发者自评："not particularly elegant; it's just a lot of hard work and a lot of code."
- **与我们的本质差异**：Teardown 的 voxel 不破坏就静止；falling sand 的地形**持续自演化**（沙流水淌火烧）——命令流只能同步"扰动"，扰动之后的演化要么各端确定性重算（→ 地形层 lockstep），要么持续传状态修正（→ NEW 路线）。这个差异是我们整个联机策略的支点（见提案 §4）。

### 4.3 其它速记

| 游戏 | 模型 | 可抄点 | 置信度 |
|---|---|---|---|
| Worms Armageddon | 确定性 lockstep + **TCP**（回合制对延迟不敏感，可靠性优先）；replay = 输入录像（lockstep 免费副产品）；改内存作弊只会让自己 desync | 输入流 replay 既是功能也是确定性回归测试 | [社区] |
| OpenLieroX | 疑似本地模拟+权威修正混合（联机有离线不出现的 glitch） | — | 未确证 |
| King Arthur's Gold | client-server + dedicated server 生态；**任务假设的"开发者 netcode devblog"未找到** | server-auth + 客户端预测的形态参照 | [社区/未确证] |
| Terraria | server-auth，tile 小行 + **RLE** 推送；单权威 server | chunk 快照 + RLE 作 late-join/自愈**兜底通道**；但其前提是地形低频变化，**不适合做我们的主通道** | [社区，协议逆向成熟] |
| Rollback (GGPO) | "The entire physics engine state has to be snapshotted every frame... infeasible to have large worlds"；MKX 补 rollback 序列化耗时约 2 man-years | **地形层绝不 rollback**；最多玩家/弹幕层局部预测回滚 | [社区共识] |

### 4.4 确定性工程技术要点 [通用技术]

- **浮点**（Gaffer On Games，经摘要转述）：跨机浮点一致极难（x87/SSE、FMA、libm 超越函数、fast-math）；但 "As long as you stick to a single compiler, and a single CPU instruction set, it is possible to make floating point fully deterministic"。**结论：sim 内核干脆整数/定点化**（Teardown 实证路线），绕开整个问题域。
- **Counter-based RNG**（Squirrel Eiserloh GDC17）：`hash(seed, position) → 随机数` 的无状态噪声替代顺序流 RNG。顺序流的输出依赖全局取数时序——多线程/分块乱序必破坏；`hash(seed, tick, x, y)` 是 random access，与遍历顺序、线程划分无关。**注意用 SquirrelNoise5**（squirrel3 在极大坐标下有重复弱点，Peter Schmidt-Nielsen 发现）。
- **lockstep 带宽红利**（Gaffer）："you can network a physics simulation of one million objects with the same bandwidth as just one"。

---

## 5. 提炼：对自研引擎的可抄清单

1. **NEW 协议三件套直接抄**：u16 像素编码（材质+flags，模拟态不上网）/ RLE-of-Option 增量（与全量共管道，配 dirty rect 天然生成）/ chunk 五态 authority 状态机（出错回 Unsynced 是一等公民）。
2. **Factorio 工程体系直接抄**：分层 CRC（per-chunk → world）+ replay 每 tick 校验 + desync report 自动打包 + late-join 三步流程 + "网络问题不导致 desync"归因原则。
3. **Teardown 双通道模板**：地形扰动 = 确定性命令流；实体 = 状态同步 + 优先级 + eventual consistency；带宽预算 ~1 Mbit/client 量级参照。
4. **NoitaMP 反面教训**：chunk 序列化/状态导出必须是引擎 day-1 公民；范围失控 + 单人长跑高难网络项目 = 归档。
5. **NEW 的"机制白名单"现实**（perks_that_dont_work_properly.md）：游戏机制按"可联机复制"约束**正向设计**（事件化、局部化），比事后修补省一个量级。
6. **确定性是分层可选的**：Teardown 只确定化破坏子系统；Factorio 全模拟确定；我们的最优解在中间——地形 CA 确定化、实体层放过（见提案）。

---

## 6. 来源索引

**Noita 本体与模组（一手为主）**

| 来源 | 内容 |
|---|---|
| [IntQuant/noita_entangled_worlds](https://github.com/IntQuant/noita_entangled_worlds) | README 同步项清单；`shared/src/world_sync.rs`（CHUNK_SIZE=128、Pixel(u16)、PixelRun）；`noita_proxy/src/net/world.rs`（authority 消息）；`docs/distributed_world_sync.drawio`（状态机）；issue #166 |
| [Ismoh/NoitaMP](https://github.com/Ismoh/NoitaMP) | README（目标/网络栈/3 次重写）；**issue #39**（世界同步撞墙核心一手）；已归档状态 |
| [Noita-Together/noita-together](https://github.com/Noita-Together/noita-together) | README（"different dimensions"）；仓库结构（三层架构）；release 历史 |
| [EvaisaDev/noita-arena](https://github.com/EvaisaDev/noita-arena) / [evaisa.mp](https://github.com/EvaisaDev/evaisa.mp) / [Workshop 页](https://steamcommunity.com/sharedfiles/filedetails/?id=3035468502) | Arena 框架与形态 |
| [speedrun.com/noita](https://www.speedrun.com/noita) | Set Seed / Random Seed 分类（确定性证据） |
| [noita.wiki.gg: Mod:Iota_Multiplayer](https://noita.wiki.gg/wiki/Mod:Iota_Multiplayer) / [Mod:Noita_Together](https://noita.wiki.gg/wiki/Mod:Noita_Together) | 模组 wiki 页 |

**同类先例与技术（一手为主）**

| 来源 | 内容 |
|---|---|
| Factorio FFF [149](https://www.factorio.com/blog/post/fff-149) / [188](https://factorio.com/blog/post/fff-188) / [340](https://www.factorio.com/blog/post/fff-340) + [Wiki: Desynchronization](https://wiki.factorio.com/Desynchronization) | lockstep / desync 检测与 report / late join / 踩坑实录 |
| [Teardown MP devblog（voxagon）](https://blog.voxagon.se/2026/03/13/teardown-multiplayer.html) | 确定性命令流 + 双通道 + fixed-point 局部确定性 |
| [Terraria 协议逆向](https://seancode.com/terrafirma/net.html) / [TShock 文档](https://tshock.readme.io/docs/multiplayer-packet-structure) | tile RLE 同步格式 |
| [Gaffer On Games: Floating Point Determinism](https://gafferongames.com/post/floating_point_determinism/) / [Deterministic Lockstep](https://gafferongames.com/post/deterministic_lockstep/) | 浮点确定性边界（抓取被拦，摘要级） |
| [Squirrel Eiserloh: Noise-Based RNG (GDC17)](https://www.youtube.com/watch?v=LWFzPP8ZbdU) / [SquirrelNoise5](https://gist.github.com/kevinmoran/0198d8e9de0da7057abe8b8b34d50f86) | counter-based RNG |
| [infil.net netcode 系列](https://words.infil.net/w02-netcode-p5.html) | rollback 大状态不可行性 |
| [PCGamingWiki: Worms Armageddon](https://www.pcgamingwiki.com/wiki/Worms_Armageddon) | W:A lockstep/TCP/replay |

**主要未确证项**：Noita 线程池规格与模拟范围；Noita RNG 架构；官方 MP 表态原话；NEW 带宽数字与玩家上限；Arena 同步模型细节；KAG netcode 一手文档；OpenLieroX 同步模型；FFF-47/52 heuristic CRC 细节（摘要级）。
