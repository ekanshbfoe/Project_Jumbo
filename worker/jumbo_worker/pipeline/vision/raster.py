"""PDF pages to images. Pages are rendered so their width lands near `target_width`, which keeps
crops sharp for 1080p video without blowing up memory on tall webtoon pages."""

from __future__ import annotations

from pathlib import Path

import fitz  # PyMuPDF
import numpy as np

# A rendered page never exceeds this many pixels (a very tall strip page is scaled down instead).
MAX_PIXELS = 60_000_000


def page_count(pdf: Path) -> int:
    with fitz.open(pdf) as doc:
        return doc.page_count


def native_width(page: fitz.Page) -> int | None:
    """Pixel width of the scan when the page is one embedded image covering it (typical for
    webtoon and scanlation PDFs); rendering wider than that only adds blur and memory."""
    images = page.get_images(full=True)
    if len(images) != 1:
        return None
    rects = page.get_image_rects(images[0][0])
    if not rects or rects[0].width < 0.95 * page.rect.width:
        return None
    return int(images[0][2] * page.rect.width / rects[0].width)


def rasterize(pdf: Path, target_width: int = 1600) -> list[np.ndarray]:
    """Every page as an RGB uint8 array (H, W, 3), all rendered at a scale chosen per page: about
    `target_width` wide, but never past the resolution of a full-page scan."""
    pages = []
    with fitz.open(pdf) as doc:
        for page in doc:
            rect = page.rect
            width = min(target_width, native_width(page) or target_width)
            scale = max(0.5, min(6.0, width / max(rect.width, 1)))
            if rect.width * rect.height * scale * scale > MAX_PIXELS:
                scale = (MAX_PIXELS / (rect.width * rect.height)) ** 0.5
            pix = page.get_pixmap(matrix=fitz.Matrix(scale, scale), alpha=False, colorspace=fitz.csRGB)
            pages.append(np.frombuffer(pix.samples, dtype=np.uint8).reshape(pix.height, pix.width, 3).copy())
    return pages
