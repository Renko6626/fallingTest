//! 位图几何工具（M3 spec §4，实施期决定：矩形覆盖替代 marching squares + 耳切）。
//!
//! 全部是**纯整数、纯函数**：布尔掩码进、整数矩形/索引集出。刚体形状与地形
//! 碰撞体都由 [`rect_cover`] 编译成轴对齐矩形的 compound——零多边形/含洞
//! 三角化的坑、天然定序（行主序贪心），矩形碰撞体也比 polyline 更不易穿隧。
//! 顶点转 f32 发生在 `physics` 边界（格角坐标在 f32 里精确可表示）。
//!
//! marching squares / Douglas-Peucker 未落地：bench 证明矩形数成为瓶颈前不上（YAGNI）。

/// 轴对齐矩形，闭区间格坐标 `[x0, x1] × [y0, y1]`（局部或世界坐标由调用方约定）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

/// 掩码 → 不重叠矩形覆盖。贪心：逐行取最大水平行程；若上一行有**完全相同**
/// `[x0, x1]` 的矩形且其 `y1 == y − 1`，向下延伸之（竖向合并）。输出按
/// `(y0, x0)` 序。覆盖恰好等于掩码（不多不少）。
pub(crate) fn rect_cover(mask: &[bool], w: usize, h: usize) -> Vec<Rect> {
    debug_assert_eq!(mask.len(), w * h);
    let mut out: Vec<Rect> = Vec::new();
    // 上一行产出的矩形在 out 里的下标区间，用于竖向合并查找。
    let mut prev_row: Vec<usize> = Vec::new();
    for y in 0..h {
        let mut cur_row: Vec<usize> = Vec::new();
        let mut x = 0;
        while x < w {
            if !mask[y * w + x] {
                x += 1;
                continue;
            }
            let x0 = x;
            while x < w && mask[y * w + x] {
                x += 1;
            }
            let x1 = x - 1;
            let merged = prev_row.iter().copied().find(|&i| {
                let r = out[i];
                r.x0 == x0 as i32 && r.x1 == x1 as i32 && r.y1 == y as i32 - 1
            });
            match merged {
                Some(i) => {
                    out[i].y1 = y as i32;
                    cur_row.push(i);
                }
                None => {
                    out.push(Rect { x0: x0 as i32, y0: y as i32, x1: x1 as i32, y1: y as i32 });
                    cur_row.push(out.len() - 1);
                }
            }
        }
        prev_row = cur_row;
    }
    out.sort_by_key(|r| (r.y0, r.x0));
    out
}

/// 4 连通分量分解：返回各分量的像素索引列表（行主序 `y*w+x`），分量按其
/// **最小索引**升序，分量内索引升序。纯整数 BFS，遍历序固定。
/// （消费者 = Task 4 重提取，接线前定向 allow。）
#[allow(dead_code)]
pub(crate) fn components4(mask: &[bool], w: usize, h: usize) -> Vec<Vec<usize>> {
    debug_assert_eq!(mask.len(), w * h);
    let mut seen = vec![false; w * h];
    let mut out = Vec::new();
    for start in 0..w * h {
        if !mask[start] || seen[start] {
            continue;
        }
        let mut comp = Vec::new();
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(i) = stack.pop() {
            comp.push(i);
            let (x, y) = (i % w, i / w);
            let mut try_push = |j: usize| {
                if mask[j] && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                try_push(i - 1);
            }
            if x + 1 < w {
                try_push(i + 1);
            }
            if y > 0 {
                try_push(i - w);
            }
            if y + 1 < h {
                try_push(i + w);
            }
        }
        comp.sort_unstable();
        out.push(comp);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask_from(rows: &[&str]) -> (Vec<bool>, usize, usize) {
        let h = rows.len();
        let w = rows[0].len();
        let mut m = Vec::with_capacity(w * h);
        for r in rows {
            assert_eq!(r.len(), w);
            m.extend(r.bytes().map(|b| b == b'#'));
        }
        (m, w, h)
    }

    #[test]
    fn rect_cover_solid_block_is_one_rect() {
        let (m, w, h) = mask_from(&["###", "###", "###"]);
        assert_eq!(rect_cover(&m, w, h), vec![Rect { x0: 0, y0: 0, x1: 2, y1: 2 }]);
    }

    #[test]
    fn rect_cover_ring_with_hole_covers_exactly_without_overlap() {
        let (m, w, h) = mask_from(&["#####", "#...#", "#...#", "#####"]);
        let rects = rect_cover(&m, w, h);
        // 覆盖恰好等于掩码：逐格计数每格被覆盖次数 ∈ {0,1} 且与掩码一致
        let mut cover = vec![0u8; w * h];
        for r in &rects {
            for y in r.y0..=r.y1 {
                for x in r.x0..=r.x1 {
                    cover[y as usize * w + x as usize] += 1;
                }
            }
        }
        for i in 0..w * h {
            assert_eq!(cover[i] == 1, m[i], "格 {i} 覆盖不符");
            assert!(cover[i] <= 1, "格 {i} 重叠");
        }
        // 顶行 1 矩形、两侧各竖向合并成 1 矩形、底行 1 矩形
        assert_eq!(rects.len(), 4, "{rects:?}");
        assert!(rects.contains(&Rect { x0: 0, y0: 1, x1: 0, y1: 2 }));
        assert!(rects.contains(&Rect { x0: 4, y0: 1, x1: 4, y1: 2 }));
    }

    #[test]
    fn rect_cover_output_is_sorted_and_pure() {
        let (m, w, h) = mask_from(&["#.#.", ".##.", "####"]);
        let a = rect_cover(&m, w, h);
        let b = rect_cover(&m, w, h);
        assert_eq!(a, b, "纯函数");
        let mut sorted = a.clone();
        sorted.sort_by_key(|r| (r.y0, r.x0));
        assert_eq!(a, sorted, "按 (y0, x0) 序");
        assert!(a.is_sorted_by_key(|r| (r.y0, r.x0)));
    }

    #[test]
    fn components4_splits_diagonal_touch_and_orders_by_min_index() {
        // 两块只在对角相触 ⇒ 4 连通下是两个分量；第二块在右下
        let (m, w, h) = mask_from(&["##..", "##..", "..##", "..##"]);
        let comps = components4(&m, w, h);
        assert_eq!(comps.len(), 2);
        assert_eq!(comps[0], vec![0, 1, 4, 5]);
        assert_eq!(comps[1], vec![10, 11, 14, 15]);
    }

    #[test]
    fn components4_cut_line_gives_two_pieces() {
        // 24×16 矩形去掉中间一整列 ⇒ 左右两块
        let (w, h) = (24usize, 16usize);
        let mut m = vec![true; w * h];
        for y in 0..h {
            m[y * w + 12] = false;
        }
        let comps = components4(&m, w, h);
        assert_eq!(comps.len(), 2);
        assert_eq!(comps[0].len(), 12 * 16);
        assert_eq!(comps[1].len(), 11 * 16);
        assert!(comps[0].iter().all(|&i| i % w < 12));
    }

    #[test]
    fn components4_empty_mask_is_empty() {
        assert!(components4(&[false; 9], 3, 3).is_empty());
    }
}
