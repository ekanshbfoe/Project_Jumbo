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


def rasterize(pdf: Path, target_width: int = 1600) -> list[np.ndarray]:
    """Every page as an RGB uint8 array (H, W, 3), all rendered at a scale chosen per page."""
    pages = []
    with fitz.open(pdf) as doc:
        for page in doc:
            rect = page.rect
            scale = max(0.5, min(6.0, target_width / max(rect.width, 1)))
            if rect.width * rect.height * scale * scale > MAX_PIXELS:
                scale = (MAX_PIXELS / (rect.width * rect.height)) ** 0.5
            pix = page.get_pixmap(matrix=fitz.Matrix(scale, scale), alpha=False, colorspace=fitz.csRGB)
            pages.append(np.frombuffer(pix.samples, dtype=np.uint8).reshape(pix.height, pix.width, 3).copy())
    return pages
