"""Cuts strip canvases (webtoon runs) at their gutters, so each slice holds one or a few panels.

Two kinds of gutter: blank background rows, and long dark bands with almost no detail. The second
is how borderless webtoons separate shots: one scene fades to black, a caption floats on black, the
next scene fades in. Detectors trained on bordered pages see such a run as one giant panel.
"""

from __future__ import annotations

import numpy as np

from .stitch import background_level, gray

# A pixel is ink when its grey level is this far from the background.
INK_TOLERANCE = 18
# A row is gutter when at most this share of it is ink (specks, scan noise).
GUTTER_INK = 0.005
# A dark separator row: mean grey at most this, and at most QUIET_BUSY of its pixels on an edge
# (a horizontal step above QUIET_EDGE grey levels), so fog and faint sparkle still count as quiet.
QUIET_DARK_MEAN = 60
QUIET_EDGE = 12
QUIET_BUSY = 0.01


def ink_rows(image: np.ndarray) -> np.ndarray:
    """Share of ink pixels in every row."""
    g = gray(image).astype(np.int16)
    return (np.abs(g - background_level(image)) > INK_TOLERANCE).mean(axis=1)


def quiet_dark_rows(image: np.ndarray) -> np.ndarray:
    """Rows that are dark and nearly featureless."""
    g = gray(image).astype(np.int16)
    busy = (np.abs(np.diff(g, axis=1)) > QUIET_EDGE).mean(axis=1)
    return (busy <= QUIET_BUSY) & (g.mean(axis=1) <= QUIET_DARK_MEAN)


def _long_runs(mask: np.ndarray, min_len: int) -> np.ndarray:
    """`mask` keeping only runs of at least `min_len` consecutive True rows."""
    out = np.zeros_like(mask)
    start = None
    for y, v in enumerate([*mask.tolist(), False]):
        if v and start is None:
            start = y
        elif not v and start is not None:
            if y - start >= min_len:
                out[start:y] = True
            start = None
    return out


def slice_strip(image: np.ndarray, min_gap: int | None = None, min_height: int | None = None) -> list[tuple[int, int]]:
    """(top, bottom) row ranges of the drawn parts, split at gutters of at least `min_gap` rows."""
    width = image.shape[1]
    min_gap = min_gap or max(8, width // 100)
    min_height = min_height or max(24, width // 30)
    gutter = ink_rows(image) <= GUTTER_INK
    # Dark separators must be long: a short black stripe is usually part of the drawing.
    gutter |= _long_runs(quiet_dark_rows(image), max(45, width // 20))
    drawn = ~gutter
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
