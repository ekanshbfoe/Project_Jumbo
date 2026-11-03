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
     "characters": [{"bbox": [x0, y0, x1, y1], "cluster": "c_1", "fingerprint"?: "<base64 float16>"}]}

Clusters only group the characters of one image; a tall image detected in tiles gets its clusters
prefixed per tile ("t1.c_0"), since c_0 of one tile is not c_0 of the next.
"""

from __future__ import annotations

import asyncio
import shutil
import tempfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Awaitable, Callable, Protocol

import cv2
import numpy as np

from ...models.magi import FINGERPRINT_MODEL
from .slicer import INK_TOLERANCE, split_tall
from .stitch import background_level, gray
from .textonly import is_empty, is_text_only
from .tiling import dedupe, merge_panels, tile_spans

Point = tuple[float, float]

# Drawn regions smaller than this share of the image (or of either side) are specks, not panels.
MIN_AREA = 0.015
MIN_SIDE = 0.04


@dataclass
class Detection:
    polygon: list[Point]
    bubbles: list[dict] = field(default_factory=list)  # {"kind", "polygon" (px), "text"?, "speaker"?}
    characters: list[dict] = field(default_factory=list)  # {"cluster", "bbox" (x0, y0, x1, y1 px), "fingerprint"?}
    # A text card kept because its image had no real panel to take its text: the video shows it only briefly.
    text_only: bool = False

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


def _nearest(panels: list[Detection], point: Point) -> Detection | None:
    """The panel whose box is closest to `point`."""

    def distance(d: Detection) -> float:
        x0, y0, x1, y1 = d.box
        return float(np.hypot(max(x0 - point[0], 0, point[0] - x1), max(y0 - point[1], 0, point[1] - y1)))

    return min(panels, key=distance, default=None)


def _poly_box(points) -> tuple[float, float, float, float]:
    xs, ys = [p[0] for p in points], [p[1] for p in points]
    return min(xs), min(ys), max(xs), max(ys)


def _shift(points, dy: float) -> list[list[float]]:
    return [[float(x), float(y) + dy] for x, y in points]


def _tile_clusters(result: dict, tile: int) -> dict:
    """Prefixes a tile's cluster ids (and the speakers that point at them) with the tile number."""
    def name(c):
        return None if c is None else f"t{tile}.{c}"

    return {
        **result,
        "texts": [{**t, "speaker": name(t.get("speaker"))} for t in result.get("texts", [])],
        "characters": [{**c, "cluster": name(c["cluster"])} for c in result.get("characters", [])],
    }


# A box whose shorter side is under this share of the image width is a sliver, not a shot.
MIN_SHOT_SIDE = 0.18
# A box lying this much inside a bigger one is part of it (Magi boxing a figure inside its panel).
NESTED = 0.8


def _area(b: tuple[float, float, float, float]) -> float:
    return max(0.0, b[2] - b[0]) * max(0.0, b[3] - b[1])


def keep_shots(boxes: list[tuple[float, float, float, float]], width: int) -> list[tuple[float, float, float, float]]:
    """Panel boxes without slivers and boxes nested in a bigger one. An image whose only boxes are slivers
    keeps them: something must carry its texts."""
    def nested(b):
        for o in boxes:
            if o is b or _area(o) <= _area(b):
                continue
            ix = max(0.0, min(b[2], o[2]) - max(b[0], o[0]))
            iy = max(0.0, min(b[3], o[3]) - max(b[1], o[1]))
            if ix * iy >= NESTED * _area(b):
                return True
        return False

    kept = [b for b in boxes if min(b[2] - b[0], b[3] - b[1]) >= MIN_SHOT_SIDE * width and not nested(b)]
    return kept or boxes


# Room left around a text card's words, as a share of the image width.
CARD_PAD = 0.03


def _frame_text(card: Detection, image: np.ndarray) -> None:
    """A text card whose words lie outside its own box (Magi boxed the black beside a caption, Solo Leveling Ragnarok
    ch1, 2026-11-03) is reframed on its words, so the video shows the caption, not an empty frame."""
    x0, y0, x1, y1 = card.box
    words = [_poly_box(b["polygon"]) for b in card.bubbles]
    if not any(bx1 <= x0 or bx0 >= x1 or by1 <= y0 or by0 >= y1 for bx0, by0, bx1, by1 in words):
        return
    h, w = image.shape[:2]
    pad = CARD_PAD * w
    gx0, gy0 = max(0.0, min(b[0] for b in words) - pad), max(0.0, min(b[1] for b in words) - pad)
    gx1, gy1 = min(float(w), max(b[2] for b in words) + pad), min(float(h), max(b[3] for b in words) + pad)
    card.polygon = [(gx0, gy0), (gx1, gy0), (gx1, gy1), (gx0, gy1)]


class MagiDetector:
    """Magi v2 through the model pool. `call` is `JobContext.call`.

    Images taller than a page are detected in overlapping tiles (see tiling.py); the tile images are
    written next to `path` for the replica to read, and all tiles are sent at once so the pool can
    spread them over replicas. A panel left far taller than wide is cut at its quietest rows (slicer.split_tall).

    Boxes that are not shots are dropped, their texts going to the nearest real panel: slivers (a thin edge of
    a drawing), boxes lying inside a bigger one, boxes around a lone bubble or caption (see textonly.py), and
    near-empty boxes (a white or flat strip with nothing drawn).
    """

    name = "magi"
    fingerprint_model = FINGERPRINT_MODEL

    def __init__(self, call: Callable[[str, str, dict], Awaitable[object]], model: str = "magi_v2") -> None:
        self.call = call
        self.model = model
        # Boxes that were not shots, dropped with their texts moved into real panels, for the stage report.
        self.folded = 0

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
            folder = Path(tempfile.mkdtemp(prefix="tiles-", dir=Path(path).parent))
            try:
                tiles = []
                for n, (top, bottom) in enumerate(spans):
                    tile = folder / f"tile_{n:03d}.png"
                    cv2.imwrite(str(tile), cv2.cvtColor(image[top:bottom], cv2.COLOR_RGB2BGR))
                    tiles.append(tile)
                found = await asyncio.gather(*(self._detect_file(tile) for tile in tiles))
                results = [(top, _tile_clusters(r, n)) for n, ((top, _), r) in enumerate(zip(spans, found))]
            finally:
                shutil.rmtree(folder, ignore_errors=True)

        boxes = merge_panels(
            [[_poly_box(_shift(p["polygon"], top)) for p in r["panels"] if len(p.get("polygon", [])) >= 3] for top, r in results],
            [(top, top + (spans[n][1] - spans[n][0])) for n, (top, _) in enumerate(results)],
        )
        boxes = keep_shots([piece for b in boxes for piece in split_tall(image, b)], image.shape[1])
        panels = [Detection([(b[0], b[1]), (b[2], b[1]), (b[2], b[3]), (b[0], b[3])]) for b in boxes]
        texts = dedupe([{**t, "polygon": _shift(t["polygon"], top)} for top, r in results for t in r.get("texts", [])], lambda t: _poly_box(t["polygon"]))
        chars = dedupe(
            [{**c, "bbox": (c["bbox"][0], c["bbox"][1] + top, c["bbox"][2], c["bbox"][3] + top)} for top, r in results for c in r.get("characters", [])],
            lambda c: c["bbox"],
        )
        # Bubbles and characters belong to the panel that contains their centre. Text outside every
        # panel (a caption floating between scenes) goes to the nearest one, so no story text is lost.
        for text in texts:
            center = _center(text["polygon"])
            owner = next((d for d in panels if _inside(center, d.polygon)), None) or _nearest(panels, center)
            if owner:
                owner.bubbles.append({"kind": text.get("kind", "speech"), "polygon": text["polygon"], "text": text.get("text"), "speaker": text.get("speaker")})
        for char in chars:
            x0, y0, x1, y1 = char["bbox"]
            owner = next((d for d in panels if _inside(((x0 + x1) / 2, (y0 + y1) / 2), d.polygon)), None)
            if owner:
                owner.characters.append({"cluster": str(char["cluster"]), "bbox": (x0, y0, x1, y1), **({"fingerprint": char["fingerprint"]} if char.get("fingerprint") else {})})
        return self._fold_text_only(image, panels)

    def _fold_text_only(self, image: np.ndarray, panels: list[Detection]) -> list[Detection]:
        """Moves the texts of text-only and empty boxes into the nearest real panel and drops the boxes.
        Only text data moves: the real panels' boxes are left exactly as detected."""
        cards = [is_text_only(image, d.box, [_poly_box(b["polygon"]) for b in d.bubbles], len(d.characters)) for d in panels]
        text_only = [
            card or is_empty(image, d.box, [_poly_box(b["polygon"]) for b in d.bubbles], len(d.characters))
            for d, card in zip(panels, cards)
        ]
        real = [d for d, t in zip(panels, text_only) if not t]
        if not real:
            # Nothing to fold into (a webtoon slice holding only a caption): keep the boxes, marking the text cards.
            for d, card in zip(panels, cards):
                d.text_only = card
                if card:
                    _frame_text(d, image)
            return panels
        if len(real) == len(panels):
            return panels
        for d, t in zip(panels, text_only):
            if t:
                for bubble in d.bubbles:
                    target = _nearest(real, _center(bubble["polygon"]))
                    assert target is not None
                    target.bubbles.append(bubble)
        self.folded += len(panels) - len(real)
        return real
