"""像素存储的常量定义。

每个像素在平铺数组中占 STRIDE 个 int：
  [type_id, velocity, lifetime, flags]
"""

TYPE_ID = 0
VELOCITY = 1
LIFETIME = 2
FLAGS = 3
STRIDE = 4

FLAG_DIRTY = 0b01
FLAG_STATIC = 0b10

AIR = 0
