"""Text-only "panels": boxes the detector draws around a speech bubble or caption floating on blank
background between scenes (webtoons do this all the time). They are not shots, so their texts are
moved into the nearest real panel and the box is dropped. Only the text data moves: the receiving
panel's box, polygon and crop stay exactly as detected, tight on the artwork.

A box is text-only when no character was found in it and, outside its balloons and texts, it has
almost no drawing: few edges (line art, screentone) and few mid-tone or coloured pixels (painted
art, which has soft edges). Thresholds were set on five real chapters (two manga, three webtoons),
where every box they fold was checked by eye to hold no artwork; anything uncertain stays a panel.
"""

from __future__ import annotations

import cv2
import numpy as np

Box = tuple[float, float, float, float]

# Shares of the box's pixels, outside balloons and texts.
MAX_DETAIL = 0.03  # drawing edges (grey step above EDGE)
MAX_TONE = 0.10  # mid-tone or coloured pixels away from edges
EDGE = 24
MID_TONE = (40, 215)
COLOURED = 60  # HSV saturation
# A balloon is the area around a text whose grey level stays this close to the text's background.
BALLOON_RANGE = 8


def measure(image: np.ndarray, box: Box, text_boxes: list[Box]) -> tuple[float, float]:
    """(detail, tone) of the box outside its balloons and texts; `image` is RGB."""
    x0, y0, x1, y1 = (int(round(v)) for v in box)
    x0, y0 = max(0, x0), max(0, y0)
    rgb = image[y0:y1, x0:x1]
    h, w = rgb.shape[:2]
    if h < 4 or w < 4:
        return 0.0, 0.0
    g = cv2.cvtColor(rgb, cv2.COLOR_RGB2GRAY)
    covered = np.zeros((h, w), np.uint8)
    for tx0, ty0, tx1, ty1 in text_boxes:
        a0, b0, a1, b1 = max(0, int(tx0 - x0)), max(0, int(ty0 - y0)), min(w, int(tx1 - x0)), min(h, int(ty1 - y0))
        if a1 <= a0 or b1 <= b0:
            continue
        covered[b0:b1, a0:a1] = 1
        # Flood the balloon from a background pixel of the text box.
        region = g[b0:b1, a0:a1]
        level = int(np.bincount(region.ravel(), minlength=256).argmax())
        ys, xs = np.nonzero(np.abs(region.astype(np.int16) - level) <= 2)
        if len(ys):
            mask = np.zeros((h + 2, w + 2), np.uint8)
            flags = cv2.FLOODFILL_FIXED_RANGE | cv2.FLOODFILL_MASK_ONLY | (1 << 8)
            cv2.floodFill(g.copy(), mask, (int(xs[0] + a0), int(ys[0] + b0)), 0, BALLOON_RANGE, BALLOON_RANGE, flags)
            covered |= mask[1:-1, 1:-1]
    # Close over the letters and grow over the balloon outlines.
    k = max(5, min(h, w) // 40)
    covered = cv2.morphologyEx(covered, cv2.MORPH_CLOSE, np.ones((15, 15), np.uint8))
    free = cv2.dilate(covered, np.ones((k, k), np.uint8)) == 0
    gi = g.astype(np.int16)
    edge = np.zeros((h, w), bool)
    edge[:, 1:] |= np.abs(np.diff(gi, axis=1)) > EDGE
    edge[1:, :] |= np.abs(np.diff(gi, axis=0)) > EDGE
    near_edge = cv2.dilate(edge.astype(np.uint8), np.ones((3, 3), np.uint8)) > 0
    saturation = cv2.cvtColor(rgb, cv2.COLOR_RGB2HSV)[:, :, 1]
    tone = (((g > MID_TONE[0]) & (g < MID_TONE[1])) | (saturation > COLOURED)) & ~near_edge
    area = float(h * w)
    return float((edge & free).sum()) / area, float((tone & free).sum()) / area


def is_text_only(image: np.ndarray, box: Box, text_boxes: list[Box], characters: int) -> bool:
    if characters or not text_boxes:
        return False
    detail, tone = measure(image, box, text_boxes)
    return detail < MAX_DETAIL and tone < MAX_TONE
