"""The numbered pack for Method 2: every page with each panel outlined and labelled with its id (p_0007), as one PDF.

The person drops it into Google AI Studio, so Gemini writes narration against the same panel ids the renderer uses
and never has to locate panels itself. Every PDF page carries a header naming the chapter and the pack id; Gemini
repeats it in its answer, and the app refuses an answer for another chapter or an older export (pack ids change
whenever the panels do).

Built so Gemini can read every label:
- Gemini scales a PDF page to fit MAX_SIDE, so labels are sized for what is left after that, and pages taller than
  TALL widths (webtoon slices) are split into overlapping PDF pages instead of being shrunk to a sliver.
- Labels sit just outside the panel's outline where there is room, over the gutter rather than the art, are
  semi-transparent, and are moved apart when two would overlap.
- The PDF is kept under MAX_BYTES (AI Studio refuses files over 50 MB) by lowering JPEG quality, then resolution.
"""

from __future__ import annotations

import json

import blake3
import cv2
import numpy as np

# Purple, as in the app (BGR).
COLOR = (255, 92, 124)
# Gemini scales PDF pages to fit this many pixels on the long side.
MAX_SIDE = 3072
# Height of a label's text, in pixels, after that scaling.
LABEL_PX = 30
# A page taller than this many widths becomes several overlapping PDF pages.
TALL = 2.2
OVERLAP = 0.15
MAX_BYTES = 40_000_000
QUALITIES = (85, 70, 55)
FONT = cv2.FONT_HERSHEY_SIMPLEX
# FONT at scale 1 is about this many pixels tall.
FONT_PX = 22

Panels = list[tuple[str, list[list[float]]]]


def pack_id(chapter: str, pages: list[tuple[np.ndarray, Panels]]) -> str:
    """Four hex digits naming this export: they change whenever a panel id or outline changes."""
    outline = [[pid, np.round(np.asarray(poly, np.float64)).astype(int).tolist()] for _, panels in pages for pid, poly in panels]
    return blake3.blake3(json.dumps([chapter, outline]).encode()).hexdigest()[:4]


def label_px(w: int, h: int) -> int:
    """Text height that is LABEL_PX once Gemini has scaled this page's PDF pages to MAX_SIDE."""
    tallest = max(w, min(h, int(TALL * w)))
    return max(LABEL_PX, round(LABEL_PX * tallest / MAX_SIDE), round(w / 60))


def _overlaps(a: tuple[int, int, int, int], b: tuple[int, int, int, int]) -> bool:
    return a[0] < b[2] and b[0] < a[2] and a[1] < b[3] and b[1] < a[3]


def label_rects(panels: Panels, w: int, h: int, text_px: int) -> list[tuple[str, tuple[int, int, int, int], tuple[int, int]]]:
    """Where each panel's label goes: (id, box, text origin). Above the panel over the gutter where there is room,
    else inside its top, else below it; nudged right past labels already placed, so no two overlap."""
    scale, thick = text_px / FONT_PX, max(2, round(text_px / 10))
    pad = max(4, text_px // 4)
    placed: list[tuple[int, int, int, int]] = []
    out = []
    for pid, polygon in panels:
        pts = np.round(np.asarray(polygon, np.float64)).astype(np.int32)
        (tw, th), base = cv2.getTextSize(pid, FONT, scale, thick)
        bw, bh = tw + 2 * pad, th + base + 2 * pad
        x0, y0, y1 = int(pts[:, 0].min()), int(pts[:, 1].min()), int(pts[:, 1].max())
        rect = None
        for y in (y0 - bh, y0, y1):
            if y < 0 or y + bh > h:
                continue
            x = int(np.clip(x0, 0, max(0, w - bw)))
            while any(_overlaps((x, y, x + bw, y + bh), p) for p in placed) and x + bw < w:
                x += bw // 2
            if x + bw <= w and not any(_overlaps((x, y, x + bw, y + bh), p) for p in placed):
                rect = (x, y, x + bw, y + bh)
                break
        if rect is None:
            # Nowhere free near the panel: stack it under the lowest label.
            y = min(h - bh, max((p[3] for p in placed), default=0))
            x = int(np.clip(x0, 0, max(0, w - bw)))
            rect = (x, y, x + bw, y + bh)
        placed.append(rect)
        out.append((pid, rect, (rect[0] + pad, rect[1] + pad + th)))
    return out


def draw_page(image: np.ndarray, panels: Panels, text_px: int | None = None) -> np.ndarray:
    """`image` RGB; `panels` (id, polygon in pixels). Returns a BGR copy with outlines and labels."""
    out = cv2.cvtColor(image, cv2.COLOR_RGB2BGR)
    h, w = out.shape[:2]
    text_px = text_px or label_px(w, h)
    scale, thick = text_px / FONT_PX, max(2, round(text_px / 10))
    for _, polygon in panels:
        cv2.polylines(out, [np.round(np.asarray(polygon, np.float64)).astype(np.int32)], True, COLOR, thick, cv2.LINE_AA)
    labels = label_rects(panels, w, h, text_px)
    overlay = out.copy()
    for _, rect, _ in labels:
        cv2.rectangle(overlay, rect[:2], rect[2:], COLOR, -1)
    # Semi-transparent, so a label over art still shows what is under it.
    out = cv2.addWeighted(overlay, 0.8, out, 0.2, 0)
    for pid, _, origin in labels:
        cv2.putText(out, pid, origin, FONT, scale, (255, 255, 255), thick, cv2.LINE_AA)
    return out


def _slices(h: int, w: int) -> list[tuple[int, int]]:
    """Row ranges of the PDF pages one image becomes: itself, or overlapping parts at most TALL widths tall."""
    if h <= TALL * w:
        return [(0, h)]
    size, step = int(TALL * w), int((TALL - OVERLAP) * w)
    out, top = [], 0
    while top + size < h:
        out.append((top, top + size))
        top += step
    out.append((max(0, h - size), h))
    return out


def _header(page: np.ndarray, text: str, text_px: int) -> np.ndarray:
    """A white band above the page with the pack stamp, so the stamp never covers the art."""
    band = np.full((int(text_px * 1.8), page.shape[1], 3), 255, np.uint8)
    scale = text_px / FONT_PX
    cv2.putText(band, text, (max(4, text_px // 2), int(text_px * 1.35)), FONT, scale, (40, 40, 40), max(2, round(text_px / 12)), cv2.LINE_AA)
    return np.vstack([band, page])


def numbered_pdf(chapter: str, pack: str, pages: list[tuple[np.ndarray, Panels]], max_bytes: int = MAX_BYTES) -> tuple[bytes, int]:
    """The numbered pack as PDF bytes, and its page count."""
    import fitz

    drawn = [draw_page(image, panels) for image, panels in pages]
    parts = [(d, label_px(d.shape[1], d.shape[0]), s) for d in drawn for s in _slices(*d.shape[:2])]
    resize = 1.0
    while True:
        for quality in QUALITIES:
            doc = fitz.open()
            for i, (image, text_px, (top, bottom)) in enumerate(parts, 1):
                page = _header(image[top:bottom], f"{chapter} . pack {pack} . page {i}/{len(parts)}", text_px)
                if resize < 1:
                    page = cv2.resize(page, (round(page.shape[1] * resize), round(page.shape[0] * resize)), interpolation=cv2.INTER_AREA)
                ok, jpg = cv2.imencode(".jpg", page, [cv2.IMWRITE_JPEG_QUALITY, quality])
                if not ok:
                    raise ValueError("cannot encode a numbered page")
                pdf_page = doc.new_page(width=page.shape[1], height=page.shape[0])
                pdf_page.insert_image(pdf_page.rect, stream=jpg.tobytes())
            data = doc.tobytes(garbage=3, deflate=True)
            if len(data) <= max_bytes or resize < 0.3:
                return data, len(parts)
        resize *= 0.75
