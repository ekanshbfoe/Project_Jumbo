"""Synthetic chapters for the vision stage: drawn art with gutters, cut into PDF pages."""

from __future__ import annotations

from pathlib import Path

import cv2
import fitz
import numpy as np

WHITE = 255


def art(h: int, w: int, seed: int) -> np.ndarray:
    """A 'drawing': smooth colour field with blurred shapes, continuous everywhere inside it."""
    rng = np.random.default_rng(seed)
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float32)
    base = np.stack([60 + 120 * xx / w, 50 + 150 * yy / h, 160 - 80 * (xx + yy) / (w + h)], axis=-1)
    canvas = base.astype(np.uint8)
    for _ in range(12):
        center = (int(rng.integers(0, w)), int(rng.integers(0, h)))
        color = tuple(int(c) for c in rng.integers(0, 255, 3))
        cv2.circle(canvas, center, int(rng.integers(w // 12, w // 4)), color, -1)
        p1 = (int(rng.integers(0, w)), int(rng.integers(0, h)))
        cv2.line(canvas, center, p1, (20, 20, 20), max(2, w // 200))
    return cv2.GaussianBlur(canvas, (0, 0), 3)


def strip(width: int, heights: list[int], gutter: int, seed: int = 1) -> tuple[np.ndarray, list[tuple[int, int]]]:
    """A white webtoon strip with one full-width panel per height; returns it and the panel row spans."""
    total = gutter + sum(h + gutter for h in heights)
    image = np.full((total, width, 3), WHITE, np.uint8)
    spans, y = [], gutter
    for i, h in enumerate(heights):
        image[y : y + h] = art(h, width, seed + i)
        spans.append((y, y + h))
        y += h + gutter
    return image, spans


def grid_page(w: int = 1000, h: int = 1400, margin: int = 50, gutter: int = 30, seed: int = 7) -> np.ndarray:
    """A printed page: 2 x 2 bordered panels with white margins and gutters."""
    page = np.full((h, w, 3), WHITE, np.uint8)
    cw, ch = (w - 2 * margin - gutter) // 2, (h - 2 * margin - gutter) // 2
    for r in range(2):
        for c in range(2):
            x, y = margin + c * (cw + gutter), margin + r * (ch + gutter)
            page[y : y + ch, x : x + cw] = art(ch, cw, seed + 2 * r + c)
            cv2.rectangle(page, (x, y), (x + cw - 1, y + ch - 1), (0, 0, 0), 3)
    return page


def write_pdf(images: list[np.ndarray], path: Path) -> Path:
    """One PDF page per image at 1 pt per pixel, so rasterizing at the image width is lossless-ish."""
    with fitz.open() as doc:
        for image in images:
            ok, png = cv2.imencode(".png", cv2.cvtColor(image, cv2.COLOR_RGB2BGR))
            assert ok
            page = doc.new_page(width=image.shape[1], height=image.shape[0])
            page.insert_image(page.rect, stream=png.tobytes())
        doc.save(path)
    return path


def cut(image: np.ndarray, rows: list[int]) -> list[np.ndarray]:
    edges = [0, *rows, image.shape[0]]
    return [image[a:b] for a, b in zip(edges, edges[1:])]
