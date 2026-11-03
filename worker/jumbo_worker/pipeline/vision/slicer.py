"""Cuts strip canvases (webtoon runs) at their gutters, so each slice holds one or a few panels.

Two kinds of gutter: blank background rows, and long dark bands with almost no detail. The second
is how borderless webtoons separate shots: one scene fades to black, a caption floats on black, the
next scene fades in. Detectors trained on bordered pages see such a run as one giant panel.

Slices much shorter than the strip is wide are not panels: a caption line floating between scenes,
or the thin edge of a dark drawing cut off by a dark band. They are merged into the neighbour across
the narrower gap, so their text stays with the story and no sliver becomes a panel of its own.
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
# Slices shorter than this share of the strip width are merged into a neighbour.
MIN_SLICE = 0.2
# A slice longer than this many widths (a long scene with no gutter) is cut at its calmest rows, and never
# longer than MAX_SLICE_ROWS: WebP cannot hold an image over 16383 pixels tall (Solo Leveling Ragnarok ch1,
# 2026-11-03, had a 21236-row slice).
MAX_SLICE_ASPECT = 8.0
MAX_SLICE_ROWS = 16000


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
    slices = merge_slivers([(top, bottom) for top, bottom in slices if bottom - top >= min_height], round(MIN_SLICE * width))
    longest = min(MAX_SLICE_ASPECT * width, MAX_SLICE_ROWS)
    return [piece for top, bottom in slices for piece in _cut(image, top, bottom, longest)]


def _cut(image: np.ndarray, top: int, bottom: int, longest: float) -> list[tuple[int, int]]:
    """(top, bottom) in even pieces no longer than `longest`, each cut moved to a calm row nearby."""
    if bottom - top <= longest:
        return [(top, bottom)]
    pieces = int(np.ceil((bottom - top) / longest))
    edges = [0, *calm_cuts(image[top:bottom], pieces), bottom - top]
    return [(top + a, top + b) for a, b in zip(edges, edges[1:])]


def merge_slivers(slices: list[tuple[int, int]], min_slice: int) -> list[tuple[int, int]]:
    """Merges every slice shorter than `min_slice` into the neighbour across the narrower gap."""
    slices = list(slices)
    while len(slices) > 1:
        short = [i for i, (top, bottom) in enumerate(slices) if bottom - top < min_slice]
        if not short:
            break
        i = min(short, key=lambda i: slices[i][1] - slices[i][0])
        gap_above = slices[i][0] - slices[i - 1][1] if i > 0 else None
        gap_below = slices[i + 1][0] - slices[i][1] if i + 1 < len(slices) else None
        j = i - 1 if gap_below is None or (gap_above is not None and gap_above <= gap_below) else i + 1
        a, b = sorted((i, j))
        slices[a:b + 1] = [(slices[a][0], slices[b][1])]
    return slices


# A panel taller than this many widths is cut into pieces (a borderless scroll that Magi saw as one shot in
# every tile, chained together across the tiles); pieces aim at TALL_PIECE widths, which pan well.
MAX_PANEL_ASPECT = 3.0
TALL_PIECE = 2.2
# How far from an even cut a piece edge may move to find a quiet row, as a share of a piece.
CUT_SLACK = 0.25
# A pixel step above this many grey levels is drawing detail.
DETAIL_STEP = 10


def busy_rows(image: np.ndarray) -> np.ndarray:
    """Share of each row's pixels on a drawing edge (horizontal or vertical step)."""
    g = gray(image).astype(np.int16)
    across = (np.abs(np.diff(g, axis=1)) > DETAIL_STEP).mean(axis=1)
    down = np.r_[(np.abs(np.diff(g, axis=0)) > DETAIL_STEP).mean(axis=1), 0.0]
    return np.maximum(across, down)


def split_tall(image: np.ndarray, box: tuple[float, float, float, float]) -> list[tuple[float, float, float, float]]:
    """`box` (x0, y0, x1, y1), cut into pieces at its quietest rows when it is far taller than wide."""
    x0, y0, x1, y1 = box
    width, height = x1 - x0, y1 - y0
    if width <= 0 or height <= MAX_PANEL_ASPECT * width:
        return [box]
    pieces = int(np.ceil(height / (TALL_PIECE * width)))
    cuts = calm_cuts(image[max(0, int(y0)) : int(np.ceil(y1)), max(0, int(x0)) : int(np.ceil(x1))], pieces)
    edges = [0, *cuts, int(height)]
    return [(x0, y0 + a, x1, y0 + b) for a, b in zip(edges, edges[1:])]


def calm_cuts(image: np.ndarray, pieces: int) -> list[int]:
    """Rows that cut `image` into `pieces` about even pieces, each moved to the calmest row near its even place."""
    step = image.shape[0] / pieces
    # Smoothed over a few rows, so a cut lands in a calm band, not between two busy lines.
    busy = np.convolve(busy_rows(image), np.ones(15) / 15, mode="same")
    cuts = []
    for k in range(1, pieces):
        lo, hi = int(k * step - CUT_SLACK * step), int(k * step + CUT_SLACK * step)
        window = busy[lo:hi]
        # The calmest row; among equally calm ones, the closest to the even cut.
        calm = np.flatnonzero(window <= window.min() + 1e-3) + lo
        cuts.append(int(calm[np.argmin(np.abs(calm - k * step))]))
    return cuts
