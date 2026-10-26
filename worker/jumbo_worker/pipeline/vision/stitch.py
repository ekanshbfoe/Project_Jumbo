"""Joins pages whose drawing continues across a PDF page break, before anything looks for panels.

Third-party PDFs often cut one continuous image in two: a webtoon strip chopped into page-sized
pieces (the break runs through a panel), or a two-page spread split down the middle. Treated as
separate pages, one panel becomes two half-panels. So every page boundary is tested for a vertical
join (bottom edge of page i continues into the top of page i+1) and a horizontal join (the inner
edges of a spread), and continuous pages are stitched into one canvas.

A boundary is continuous when both edges carry ink (a blank edge is a natural gutter) and the two
edge lines differ about as little as neighbouring lines inside each page do, or their gradient
profiles line up. Every decision, with its numbers, goes into the stage report so the thresholds
can be checked against real chapters.
"""

from __future__ import annotations

from dataclasses import dataclass, field

import cv2
import numpy as np

# Edges whose size differs by more than this cannot be one image cut in two.
SIZE_TOLERANCE = 0.03
# Mean grey-level difference (0-255) a cut through a drawing may show on top of its own line-to-line change.
CROSS_SLACK = 4.0
CROSS_FLOOR = 10.0
# Gradient correlation that marks the same structures on both sides of the cut.
MIN_CORRELATION = 0.6
MAX_CORRELATED_DIFF = 30.0
# An edge line counts as inked when it varies or sits clearly off the page background.
INK_STD = 6.0
INK_OFFSET = 20.0


def gray(image: np.ndarray) -> np.ndarray:
    return cv2.cvtColor(image, cv2.COLOR_RGB2GRAY) if image.ndim == 3 else image


def background_level(image: np.ndarray) -> float:
    """Grey level of the page background: the flattest of the four border lines. (Art often runs to
    the side edges of a strip, so a median over the whole border can land on the drawing.)"""
    g = gray(image)
    flattest = min((g[0], g[-1], g[:, 0], g[:, -1]), key=lambda line: float(line.std()))
    return float(np.median(flattest))


@dataclass(frozen=True)
class Join:
    after_page: int  # boundary between page `after_page` and the next one (0-based)
    kind: str  # "vertical" | "horizontal" | "none"
    reason: str
    cross: float = 0.0
    inner: float = 0.0
    correlation: float = 0.0


@dataclass
class Canvas:
    """One image the rest of the stage works on: a page, a stitched strip run, or a spread."""

    pages: list[int]
    kind: str  # "page" | "strip" | "spread"
    image: np.ndarray = field(repr=False)


def _lines(a: np.ndarray, b: np.ndarray, vertical: bool) -> tuple[np.ndarray, ...]:
    """The last two lines of `a` and the first two of `b` across the boundary, as float grey."""
    ga, gb = gray(a).astype(np.float32), gray(b).astype(np.float32)
    if not vertical:
        ga, gb = ga.T, gb.T
    if ga.shape[1] != gb.shape[1]:
        gb = cv2.resize(gb, (ga.shape[1], gb.shape[0]), interpolation=cv2.INTER_AREA)
    smooth = lambda line: np.convolve(line, np.ones(3) / 3, mode="same")  # noqa: E731 - tames JPEG noise
    return smooth(ga[-1]), smooth(ga[-2]), smooth(gb[0]), smooth(gb[1])


def _inked(line: np.ndarray, background: float) -> bool:
    return float(line.std()) > INK_STD or abs(float(line.mean()) - background) > INK_OFFSET


def judge(a: np.ndarray, b: np.ndarray, vertical: bool) -> tuple[bool, str, float, float, float]:
    """Whether `b` continues `a` across their shared edge (bottom/top, or right/left)."""
    size_a, size_b = (a.shape[1], b.shape[1]) if vertical else (a.shape[0], b.shape[0])
    if abs(size_a - size_b) > SIZE_TOLERANCE * max(size_a, size_b):
        return False, "edge sizes differ", 0.0, 0.0, 0.0
    ea, ea2, eb, eb2 = _lines(a, b, vertical)
    ink_a, ink_b = _inked(ea, background_level(a)), _inked(eb, background_level(b))
    if not ink_a and not ink_b:
        return False, "blank gutter on both sides", 0.0, 0.0, 0.0
    if ink_a != ink_b:
        return False, "only one side is drawn to the edge", 0.0, 0.0, 0.0
    cross = float(np.abs(ea - eb).mean())
    inner = float((np.abs(ea - ea2).mean() + np.abs(eb - eb2).mean()) / 2)
    ga, gb = np.diff(ea), np.diff(eb)
    correlation = float(np.corrcoef(ga, gb)[0, 1]) if ga.std() > 0 and gb.std() > 0 else 0.0
    if not vertical and max(ea.std(), eb.std()) <= INK_STD:
        # Two flat-coloured edges side by side prove nothing; spreads are only joined on real structure.
        return False, "flat edges", cross, inner, correlation
    if cross <= max(CROSS_SLACK + 2.5 * inner, CROSS_FLOOR):
        return True, "edges continue", cross, inner, correlation
    if correlation >= MIN_CORRELATION and cross <= MAX_CORRELATED_DIFF:
        return True, "edge structures line up", cross, inner, correlation
    return False, "edges do not match", cross, inner, correlation


def find_joins(pages: list[np.ndarray], rtl: bool = False, strip_aspect: float = 2.0) -> list[Join]:
    joins = []
    for i in range(len(pages) - 1):
        a, b = pages[i], pages[i + 1]
        ok, reason, cross, inner, corr = judge(a, b, vertical=True)
        if ok:
            joins.append(Join(i, "vertical", reason, cross, inner, corr))
            continue
        if any(p.shape[0] / p.shape[1] >= strip_aspect for p in (a, b)):
            joins.append(Join(i, "none", f"vertical: {reason}; strip pages are never spreads", cross, inner, corr))
            continue
        # Spreads: the earlier page is on the left for left-to-right reading (manhwa), right for manga.
        left, right = (b, a) if rtl else (a, b)
        h_ok, h_reason, h_cross, h_inner, h_corr = judge(left, right, vertical=False)
        if h_ok:
            joins.append(Join(i, "horizontal", h_reason, h_cross, h_inner, h_corr))
        else:
            joins.append(Join(i, "none", f"vertical: {reason}; horizontal: {h_reason}", cross, inner, corr))
    return joins


def _vstack(images: list[np.ndarray]) -> np.ndarray:
    width = images[0].shape[1]
    return np.vstack([im if im.shape[1] == width else cv2.resize(im, (width, round(im.shape[0] * width / im.shape[1])), interpolation=cv2.INTER_AREA) for im in images])


def _hstack(images: list[np.ndarray]) -> np.ndarray:
    height = images[0].shape[0]
    return np.hstack([im if im.shape[0] == height else cv2.resize(im, (round(im.shape[1] * height / im.shape[0]), height), interpolation=cv2.INTER_AREA) for im in images])


def stitch(pages: list[np.ndarray], joins: list[Join], rtl: bool = False, strip_aspect: float = 2.0) -> list[Canvas]:
    """Groups pages along the joins: vertical runs become one strip, a horizontal pair one spread."""
    canvases: list[Canvas] = []
    group: list[int] = [0] if pages else []
    kind = "single"
    for join in joins:
        nxt = join.after_page + 1
        if join.kind == "vertical" and kind in ("single", "vertical"):
            group.append(nxt)
            kind = "vertical"
        elif join.kind == "horizontal" and kind == "single":
            group.append(nxt)
            kind = "horizontal"
        else:
            canvases.append(_compose(pages, group, kind, rtl, strip_aspect))
            group, kind = [nxt], "single"
    if group:
        canvases.append(_compose(pages, group, kind, rtl, strip_aspect))
    return canvases


def _compose(pages: list[np.ndarray], group: list[int], kind: str, rtl: bool, strip_aspect: float) -> Canvas:
    if kind == "vertical":
        return Canvas(group, "strip", _vstack([pages[i] for i in group]))
    if kind == "horizontal":
        order = list(reversed(group)) if rtl else group
        return Canvas(group, "spread", _hstack([pages[i] for i in order]))
    image = pages[group[0]]
    return Canvas(group, "strip" if image.shape[0] / image.shape[1] >= strip_aspect else "page", image)
