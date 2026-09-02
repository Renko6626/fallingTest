# map-editor — 手绘场景 preset，改完即渲

> 文档路径：`tools/map-editor/README.md`
> 设计：`docs/superpowers/specs/2026-09-01-map-editor-design.md`

四环之外的开发工具。浏览器里画初始网格 → 保存成 `data/scenarios/<name>.ron`
（`grid` 字段）→ 自动跑 `sand-harness render` → 页面里看 GIF。

## 启动

```bash
cargo build --release -p sand-harness        # 服务调用的二进制
python3 tools/map-editor/serve.py            # 只绑 127.0.0.1:8765；可传端口号
```

本机浏览器：`ssh -L 8765:localhost:8765 sunyunbo@zhustation` → `http://localhost:8765/`。

## 用法

- **新建**：选世界尺寸（chunk 数 × 64），画布出现；chunk 缝线（64 倍数）画成细线——
  镜像轴别压在缝上（`docs/proposals/2026-08-31-powder-scan-direction-bias.md` 的残留偏置）。
- **工具**：笔刷（1/2/4/8）、矩形、油漆桶（4 连通）、橡皮（= air）、吸管；Ctrl+Z / Ctrl+Y。
- **加载**：下拉里任何既有场景都能进来（走 `sand-harness rasterize`，老的纯 `Fill`
  场景也行）；seed / ticks / `script` 从原文轻量提取预填。
- **保存并渲染**：写 `data/scenarios/<name>.ron` + 出 `tools/map-editor/out/<name>.gif`
  （目录已 .gitignore）。渲染参数 ticks/every/scale/from 在侧栏。
- **仅导出**：下载 `.ron`，不落盘不渲染。

## `grid` 格式速览

```ron
grid: (
    legend: { '.': "air", 'W': "wall", 'O': "oil" },
    rows: [ "256.", "2W 60. 40O 154.", … ],   // 行数 = 高度，每行展开 = 宽度
),
```

`<count><char>`，count 省略即 1，空白可选；`grid` 先铺、`setup` 再叠。加载期全部显式
报错（图例重复/未知材质/行数/宽度/未知字符）。**图例字符池**：页面与
`sand-harness/src/scenario.rs::default_legend` 使用同一规则（air `'.'`，其余优先材质名
首字母大写→小写，冲突再按 `LEGEND_POOL`）；改一处必须同步另一处。

## 第一版不做

时间线可视化编辑（`script` 只是原样透传的文本框）、wasm 实时预览（页面内存网格
`Uint8Array(w*h)` 已为它留门）、多用户/鉴权。
