"""Panel detectors behind one interface.

- `MagiDetector` is the production detector: Magi v2 running in the model pool on Kaggle. It also
  returns speech bubbles with their text and speakers, and character clusters.
- `OpenCVDetector` finds drawn regions separated by background gutters. It exists to build and test
  the stage's plumbing without the model; it does not survive complex art (borderless panels, art
  bleeding across gutters, overlapping panels) and is never the production path.

Magi replica contract (`magi_v2` model, request kind `detect`, payload `{"image": <path on the
worker host>}`), all coordinates in pixels of that image (jumbo_worker/models/magi.py serves it):

    {"panels":     [{"polygon": [[x, y], ...]}],
     "texts":      [{"polygon": [[x, y], ...], "kind": "speech|thought|narration|sfx", "text": "...", "speaker": "c_1" | null}],
     "characters": [{"bbox": [x0, y0, x1, y1], "cluster": "c_1"}]}
"""

from __future__ import annotations

import shutil
import tempfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Awaitable, Callable, Protocol

import cv2
import numpy as np

from .slicer import INK_TOLERANCE
from .stitch import background_level, gray
from .tiling import dedupe, merge_panels, tile_spans

Point = tuple[float, float]

# Drawn regions smaller than this share of the image (or of either side) are specks, not panels.
MIN_AREA = 0.015
MIN_SIDE = 0.04


@dataclass
class Detection:
    polygon: list[Point]
    bubbles: list[dict] = field(default_factory=list)  # {"kind", "polygon" (px), "text"?, "speaker"?}
    characters: list[dict] = field(default_factory=list)  # {"cluster", "bbox" (x0, y0, x1, y1 px)}

    @property
    def box(self) -> tuple[float, float, float, float]:
        xs, ys = [p[0] for p in self.polygon], [p[1] for p in self.polygon]
        return min(xs), min(ys), max(xs), max(ys)


class Detector(Protocol):
    name: str

    async def detect(self, image: np.ndarray, path: Path) -> list[Detection]: ...


def drawn_regions(image: np.ndarray) -> list[list[Point]]:
    """Outlines of the drawn areas separated by background gutters, simplified to a few corners."""
    h, w = image.shape[:2]
    ink = (np.abs(gray(image).astype(np.int16) - background_level(image)) > INK_TOLERANCE).astype(np.uint8)
    k = max(3, min(h, w) // 150)
    closed = cv2.morphologyEx(ink, cv2.MORPH_CLOSE, cv2.getStructuringElement(cv2.MORPH_RECT, (k, k)))
    contours, _ = cv2.findContours(closed, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
    out = []
    for contour in contours:
        x, y, cw, ch = cv2.boundingRect(contour)
        if cw * ch < MIN_AREA * w * h or cw < MIN_SIDE * w or ch < MIN_SIDE * h:
            continue
        hull = cv2.convexHull(contour)
        poly = cv2.approxPolyDP(hull, 0.01 * cv2.arcLength(hull, True), True).reshape(-1, 2)
        if len(poly) < 3 or len(poly) > 12:
            poly = np.array([[x, y], [x + cw, y], [x + cw, y + ch], [x, y + ch]])
        out.append([(float(px), float(py)) for px, py in poly])
    return out


# Regions whose boxes overlap by this share of the smaller one are one panel split by light art.
MERGE_OVERLAP = 0.2


def merge_overlapping(polygons: list[list[Point]]) -> list[list[Point]]:
    """Unions regions whose bounding boxes overlap (a highlight in the art can cut a panel in two);
    merged regions become their combined box."""
    items = [(Detection(p).box, p) for p in polygons]
    merged = True
    while merged:
        merged = False
        for i in range(len(items)):
            for j in range(i + 1, len(items)):
                (a, pa), (b, pb) = items[i], items[j]
                ix = min(a[2], b[2]) - max(a[0], b[0])
                iy = min(a[3], b[3]) - max(a[1], b[1])
                smaller = min((a[2] - a[0]) * (a[3] - a[1]), (b[2] - b[0]) * (b[3] - b[1]))
                if ix > 0 and iy > 0 and ix * iy >= MERGE_OVERLAP * smaller:
                    box = (min(a[0], b[0]), min(a[1], b[1]), max(a[2], b[2]), max(a[3], b[3]))
                    rect = [(box[0], box[1]), (box[2], box[1]), (box[2], box[3]), (box[0], box[3])]
                    items[i] = (box, rect)
                    del items[j]
                    merged = True
                    break
            if merged:
                break
    return [p for _, p in items]


class OpenCVDetector:
    """Local plumbing detector (see the module docs): no bubbles, no characters."""

    name = "opencv"

    async def detect(self, image: np.ndarray, path: Path) -> list[Detection]:
        return [Detection(poly) for poly in merge_overlapping(drawn_regions(image))]


def _center(points: list[list[float]]) -> Point:
    xs, ys = [p[0] for p in points], [p[1] for p in points]
    return (min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2


def _inside(point: Point, polygon: list[Point]) -> bool:
    return cv2.pointPolygonTest(np.array(polygon, dtype=np.float32), point, False) >= 0


def _poly_box(points) -> tuple[float, float, float, float]:
    xs, ys = [p[0] for p in points], [p[1] for p in points]
    return min(xs), min(ys), max(xs), max(ys)


def _shift(points, dy: float) -> list[list[float]]:
    return [[float(x), float(y) + dy] for x, y in points]


class MagiDetector:
    """Magi v2 through the model pool. `call` is `JobContext.call`.

    Images taller than a page are detected in overlapping tiles (see tiling.py); the tile images are
    written next to `path` for the replica to read.
    """

    name = "magi"

    def __init__(self, call: Callable[[str, str, dict], Awaitable[object]], model: str = "magi_v2") -> None:
        self.call = call
        self.model = model

    async def _detect_file(self, path: Path) -> dict:
        result = await self.call(self.model, "detect", {"image": str(path)})
        if not isinstance(result, dict) or not isinstance(result.get("panels"), list):
            raise ValueError(f"{self.model} returned an unexpected result: {str(result)[:200]}")
        return result

    async def detect(self, image: np.ndarray, path: Path) -> list[Detection]:
        spans = tile_spans(*image.shape[:2])
        if len(spans) == 1:
            results = [(0, await self._detect_file(path))]
        else:
            results = []
            folder = Path(tempfile.mkdtemp(prefix="tiles-", dir=Path(path).parent))
            try:
                for n, (top, bottom) in enumerate(spans):
                    tile = folder / f"tile_{n:03d}.png"
                    cv2.imwrite(str(tile), cv2.cvtColor(image[top:bottom], cv2.COLOR_RGB2BGR))
                    results.append((top, await self._detect_file(tile)))
            finally:
                shutil.rmtree(folder, ignore_errors=True)

        boxes = merge_panels([[_poly_box(_shift(p["polygon"], top)) for p in r["panels"] if len(p.get("polygon", [])) >= 3] for top, r in results])
        panels = [Detection([(b[0], b[1]), (b[2], b[1]), (b[2], b[3]), (b[0], b[3])]) for b in boxes]
        texts = dedupe([{**t, "polygon": _shift(t["polygon"], top)} for top, r in results for t in r.get("texts", [])], lambda t: _poly_box(t["polygon"]))
        chars = dedupe(
            [{**c, "bbox": (c["bbox"][0], c["bbox"][1] + top, c["bbox"][2], c["bbox"][3] + top)} for top, r in results for c in r.get("characters", [])],
            lambda c: c["bbox"],
        )
        # Bubbles and characters belong to the panel that contains their centre.
        for text in texts:
            owner = next((d for d in panels if _inside(_center(text["polygon"]), d.polygon)), None)
            if owner:
                owner.bubbles.append({"kind": text.get("kind", "speech"), "polygon": text["polygon"], "text": text.get("text"), "speaker": text.get("speaker")})
        for char in chars:
            x0, y0, x1, y1 = char["bbox"]
            owner = next((d for d in panels if _inside(((x0 + x1) / 2, (y0 + y1) / 2), d.polygon)), None)
            if owner:
                owner.characters.append({"cluster": str(char["cluster"]), "bbox": (x0, y0, x1, y1)})
        return panels
