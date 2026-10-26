"""Tall images in page-shaped tiles, and the detections put back together.

Detectors trained on pages (Magi resizes every image to at most 1333 px on its long side) cannot
see a 900 x 13000 webtoon slice: it would shrink to about 90 px wide. So tall images are cut into
overlapping tiles of page proportions, each tile is detected on its own, and the results are
shifted back and merged: panel pieces from neighbouring tiles that overlap become one panel,
and texts or characters seen twice in an overlap are kept once.
"""

from __future__ import annotations

Box = tuple[float, float, float, float]  # x0, y0, x1, y1

# Tile height as a multiple of the width: about a manga page.
TILE_ASPECT = 1.4
# Share of a tile repeated in the next one, so anything cut at a tile edge is whole in one of them.
OVERLAP = 0.25
# Images up to this much taller than one tile are not worth cutting.
SLACK = 1.15
# Panel pieces from neighbouring tiles are one panel when, inside the band the two tiles share, they
# cover the same area this closely (IoU). Comparing only inside the shared band matters: a panel cut
# at one tile's edge must not be joined to the next panel down that the other tile also sees.
SAME_PANEL_IN_OVERLAP = 0.5
# Texts or characters overlapping this much (IoU) are the same detection seen twice.
DUPLICATE_IOU = 0.5


def tile_spans(height: int, width: int, aspect: float = TILE_ASPECT, overlap: float = OVERLAP) -> list[tuple[int, int]]:
    """(top, bottom) rows of each tile; a single span when the image is already page-shaped."""
    tile = max(1, round(aspect * width))
    if height <= tile * SLACK:
        return [(0, height)]
    step = max(1, round(tile * (1 - overlap)))
    tops = list(range(0, height - tile, step)) + [height - tile]
    return [(top, top + tile) for top in sorted(set(tops))]


def _iou(a: Box, b: Box) -> float:
    ix = max(0.0, min(a[2], b[2]) - max(a[0], b[0]))
    iy = max(0.0, min(a[3], b[3]) - max(a[1], b[1]))
    inter = ix * iy
    union = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter
    return inter / union if union > 0 else 0.0


def merge_panels(per_tile: list[list[Box]], spans: list[tuple[int, int]]) -> list[Box]:
    """Unions panel boxes from neighbouring tiles that are the same panel (seen twice in the shared
    band, or cut by a tile edge and continued in the next tile)."""
    boxes = [(t, b) for t, tile in enumerate(per_tile) for b in tile]
    parent = list(range(len(boxes)))

    def find(i: int) -> int:
        while parent[i] != i:
            parent[i] = parent[parent[i]]
            i = parent[i]
        return i

    for i, (ti, a) in enumerate(boxes):
        for j in range(i + 1, len(boxes)):
            tj, b = boxes[j]
            if abs(ti - tj) != 1:
                continue
            upper, lower = min(ti, tj), max(ti, tj)
            band_top, band_bottom = spans[lower][0], spans[upper][1]
            clip = lambda box: (box[0], max(box[1], band_top), box[2], min(box[3], band_bottom))  # noqa: E731
            ca, cb = clip(a), clip(b)
            if ca[3] <= ca[1] or cb[3] <= cb[1]:
                continue
            if _iou(ca, cb) >= SAME_PANEL_IN_OVERLAP:
                parent[find(i)] = find(j)
    groups: dict[int, Box] = {}
    for i, (_, b) in enumerate(boxes):
        root = find(i)
        g = groups.get(root)
        groups[root] = b if g is None else (min(g[0], b[0]), min(g[1], b[1]), max(g[2], b[2]), max(g[3], b[3]))
    return sorted(groups.values(), key=lambda b: (b[1], b[0]))


def dedupe(items: list[dict], box_of) -> list[dict]:
    """Keeps one of each group of overlapping detections, preferring the larger (less cut off) box."""
    area = lambda b: (b[2] - b[0]) * (b[3] - b[1])  # noqa: E731
    kept: list[dict] = []
    for item in sorted(items, key=lambda it: -area(box_of(it))):
        if all(_iou(box_of(item), box_of(k)) < DUPLICATE_IOU for k in kept):
            kept.append(item)
    return kept
