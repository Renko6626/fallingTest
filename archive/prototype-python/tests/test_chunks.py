from core.chunks import CHUNK, MARGIN, PASS_PARITY, ChunkLayout, Rect


def test_rect_contains():
    r = Rect(0, 0, 4, 4)
    assert r.contains(0, 0)
    assert r.contains(3, 3)
    assert not r.contains(4, 0)
    assert not r.contains(0, 4)
    assert not r.contains(-1, 0)


def test_layout_counts():
    assert (ChunkLayout(128, 128).cw, ChunkLayout(128, 128).ch) == (2, 2)
    assert (ChunkLayout(192, 192).cw, ChunkLayout(192, 192).ch) == (3, 3)
    assert (ChunkLayout(100, 70).cw, ChunkLayout(100, 70).ch) == (2, 2)  # 非整除


def test_chunk_rect_clipping():
    lay = ChunkLayout(100, 70)
    assert lay.chunk_rect(1, 1) == Rect(64, 64, 100, 70)
    assert lay.chunk_rect(0, 0) == Rect(0, 0, 64, 64)


def test_write_rect_values_and_clipping():
    lay = ChunkLayout(192, 192)
    assert lay.write_rect(1, 1) == Rect(32, 32, 160, 160)   # [64−32, 128+32)
    assert lay.write_rect(0, 0) == Rect(0, 0, 96, 96)       # 负向裁剪
    assert lay.write_rect(2, 2) == Rect(96, 96, 192, 192)   # 正向裁剪


def test_passes_cover_each_chunk_exactly_once():
    lay = ChunkLayout(192, 192)
    seen = []
    for p in range(4):
        seen.extend(lay.chunks_for_pass(p))
    assert len(seen) == lay.cw * lay.ch
    assert len(set(seen)) == len(seen)


def test_same_pass_chunks_share_parity():
    lay = ChunkLayout(320, 320)
    for p, (px, py) in enumerate(PASS_PARITY):
        for cx, cy in lay.chunks_for_pass(p):
            assert (cx % 2, cy % 2) == (px, py)


def test_same_pass_write_rects_disjoint():
    """交换律前提（提案 §2.2 条件①）：同 pass 写域两两不相交。"""
    lay = ChunkLayout(320, 320)
    for p in range(4):
        rects = [lay.write_rect(cx, cy) for cx, cy in lay.chunks_for_pass(p)]
        for i in range(len(rects)):
            for j in range(i + 1, len(rects)):
                a, b = rects[i], rects[j]
                overlap = not (
                    a.x1 <= b.x0 or b.x1 <= a.x0 or a.y1 <= b.y0 or b.y1 <= a.y0
                )
                assert not overlap, f"pass {p}: {a} 与 {b} 重叠"
