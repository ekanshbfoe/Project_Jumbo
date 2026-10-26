"""Cuts strip canvases (webtoon runs) at their blank gutters, so each slice holds one or a few panels."""

from __future__ import annotations

import numpy as np

from .stitch import background_level, gray

# A pixel is ink when its grey level is this far from the background.
INK_TOLERANCE = 18
# A row is gutter when at most this share of it is ink (specks, scan noise).
GUTTER_INK = 0.005


def ink_rows(image: np.ndarray) -> np.ndarray:
    """Share of ink pixels in every row."""
    g = gray(image).astype(np.int16)
    return (np.abs(g - background_level(image)) > INK_TOLERANCE).mean(axis=1)


def slice_strip(image: np.ndarray, min_gap: int | None = None, min_height: int | None = None) -> list[tuple[int, int]]:
    """(top, bottom) row ranges of the drawn parts, split at gutters of at least `min_gap` rows."""
    width = image.shape[1]
    min_gap = min_gap or max(8, width // 100)
    min_height = min_height or max(24, width // 30)
    drawn = ink_rows(image) > GUTTER_INK
    slices: list[tuple[int, int]] = []
    start, gap = None, 0
    for y, ink in enumerate(drawn):
        if ink:
            if start is None:
                start = y
            gap = 0
        elif start is not None:
            gap += 1
            if gap >= min_gap:
                slices.append((start, y - gap + 1))
                start, gap = None, 0
    if start is not None:
        slices.append((start, len(drawn) - gap))
    return [(top, bottom) for top, bottom in slices if bottom - top >= min_height]
