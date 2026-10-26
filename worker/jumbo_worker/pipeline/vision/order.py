"""Reading order of the panels on one canvas: rows top to bottom, then left to right within a row
(right to left for manga)."""

from __future__ import annotations

# Two panels share a row when they overlap vertically by at least this share of the shorter one.
ROW_OVERLAP = 0.5


def reading_order(boxes: list[tuple[float, float, float, float]], rtl: bool = False) -> list[int]:
    """Indices of `boxes` (x0, y0, x1, y1) in reading order."""
    rows: list[list[int]] = []
    spans: list[tuple[float, float]] = []
    for i in sorted(range(len(boxes)), key=lambda i: (boxes[i][1], boxes[i][0])):
        x0, y0, x1, y1 = boxes[i]
        for r, (top, bottom) in enumerate(spans):
            overlap = min(bottom, y1) - max(top, y0)
            if overlap >= ROW_OVERLAP * min(bottom - top, y1 - y0):
                rows[r].append(i)
                spans[r] = (min(top, y0), max(bottom, y1))
                break
        else:
            rows.append([i])
            spans.append((y0, y1))
    out: list[int] = []
    for r in sorted(range(len(rows)), key=lambda r: spans[r][0]):
        out += sorted(rows[r], key=lambda i: -boxes[i][2] if rtl else boxes[i][0])
    return out
