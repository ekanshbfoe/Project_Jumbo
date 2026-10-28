"""The numbered pack for Method 2: every page with each panel outlined and labelled with its id (p_0007), as one PDF.

The person drops it into Google AI Studio, so Gemini writes narration against the same panel ids the renderer uses
and never has to locate panels itself.
"""

from __future__ import annotations

import cv2
import numpy as np

# Purple, as in the app (BGR).
COLOR = (255, 92, 124)
JPEG_QUALITY = 85


def draw_page(image: np.ndarray, panels: list[tuple[str, list[list[float]]]]) -> np.ndarray:
    """`image` RGB; `panels` (id, polygon in pixels). Returns a BGR copy with outlines and labels."""
    out = cv2.cvtColor(image, cv2.COLOR_RGB2BGR)
    h, w = out.shape[:2]
    thick = max(2, round(w / 400))
    scale = max(0.6, w / 1400)
    for pid, polygon in panels:
        pts = np.round(np.asarray(polygon, np.float64)).astype(np.int32)
        cv2.polylines(out, [pts], True, COLOR, thick, cv2.LINE_AA)
        (tw, th), base = cv2.getTextSize(pid, cv2.FONT_HERSHEY_SIMPLEX, scale, thick)
        pad = thick * 2
        x = int(np.clip(pts[:, 0].min(), 0, max(0, w - tw - 2 * pad)))
        y = int(np.clip(pts[:, 1].min(), 0, max(0, h - th - base - 2 * pad)))
        cv2.rectangle(out, (x, y), (x + tw + 2 * pad, y + th + base + 2 * pad), COLOR, -1)
        cv2.putText(out, pid, (x + pad, y + pad + th), cv2.FONT_HERSHEY_SIMPLEX, scale, (255, 255, 255), thick, cv2.LINE_AA)
    return out


def numbered_pdf(pages: list[tuple[np.ndarray, list[tuple[str, list[list[float]]]]]]) -> bytes:
    """One PDF page per image, at the image's own size, with its panels drawn on."""
    import fitz

    doc = fitz.open()
    for image, panels in pages:
        drawn = draw_page(image, panels)
        ok, jpg = cv2.imencode(".jpg", drawn, [cv2.IMWRITE_JPEG_QUALITY, JPEG_QUALITY])
        if not ok:
            raise ValueError("cannot encode a numbered page")
        h, w = drawn.shape[:2]
        page = doc.new_page(width=w, height=h)
        page.insert_image(page.rect, stream=jpg.tobytes())
    return doc.tobytes(garbage=3, deflate=True)
