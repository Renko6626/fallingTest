"""像素存储的常量定义。

每个像素在平铺数组中占 STRIDE 个 int：
  [type_id, velocity, lifetime, flags, updated_at]

- updated_at：最后一次行动（移动/被动交换/in-update 转化）的帧号，-1 = 从未行动。
  扫描遇 updated_at == 当前帧 即跳过——跨缝移入未处理 chunk 的像素不会同帧二动
  （确定性提案 §2.3 顺序账本 row 5）。
- flags：预留给 fire spec v2 的 FLAG_BURNING。
  FLAG_DIRTY 已被世代戳取代；FLAG_STATIC 被调研否决
  （per-chunk dirty rect 路线，见 noita-deep-dive §2.1）——两者已删除。
"""

TYPE_ID = 0
VELOCITY = 1
LIFETIME = 2
FLAGS = 3
UPDATED_AT = 4
STRIDE = 5

AIR = 0
