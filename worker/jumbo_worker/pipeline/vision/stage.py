"""The `vision` job: chapter PDF in, a manifest blob out.

Rasterize -> join pages split across page breaks (stitch) -> slice strip runs at gutters -> detect
panels -> reading order -> crops -> PanelsDoc, validated against the shared contract.

Params: `chapter_id` (required), `detector` ("magi", the default and the production path, or
"opencv" for local plumbing tests), `target_width` (default 1600), `rtl` (default: true for
printed-page chapters, read as manga, false for strips).

The output manifest is JSON:

    {"schema_version": 1, "chapter_id": "ch_0001",
     "panels": <PanelsDoc>,                       # paths are project-relative (chapters/<id>/...)
     "files": {"pages/page_001.webp": "<blake3>", "panels/crops/p_0001.webp": "<blake3>", ...},
     "report": {"pages": n, "joins": [...], "canvases": [...], "detector": "magi"}}

`files` keys are relative to the chapter folder; the app downloads each blob to that path.
"""

from __future__ import annotations

import asyncio
import json
from dataclasses import asdict

import cv2
import numpy as np

from ...contracts import PanelsDoc
from ...contracts.rules import parse
from ...jobs import JobContext, JobError
from .detect import Detection, Detector, MagiDetector, OpenCVDetector
from .order import reading_order
from .raster import rasterize
from .slicer import slice_strip
from .stitch import chapter_layout, find_joins, stitch

WEBP_QUALITY = 92


def encode_webp(image: np.ndarray) -> bytes:
    ok, buf = cv2.imencode(".webp", cv2.cvtColor(image, cv2.COLOR_RGB2BGR), [cv2.IMWRITE_WEBP_QUALITY, WEBP_QUALITY])
    if not ok:
        raise RuntimeError("webp encoding failed")
    return buf.tobytes()


def _unit(v: float) -> float:
    return round(min(1.0, max(0.0, v)), 6)


def _norm_polygon(points, w: int, h: int) -> list[list[float]]:
    return [[_unit(x / w), _unit(y / h)] for x, y in points]


def _norm_box(x0: float, y0: float, x1: float, y1: float, w: int, h: int) -> dict:
    x0, y0 = max(0.0, x0), max(0.0, y0)
    x1, y1 = min(float(w), x1), min(float(h), y1)
    bx, by = _unit(x0 / w), _unit(y0 / h)
    return {"x": bx, "y": by, "w": max(1e-6, min(1.0 - bx, round((x1 - x0) / w, 6))), "h": max(1e-6, min(1.0 - by, round((y1 - y0) / h, 6)))}


def make_detector(name: str, ctx: JobContext) -> Detector:
    if name == "magi":
        return MagiDetector(ctx.call)
    if name == "opencv":
        return OpenCVDetector()
    raise JobError(f"unknown detector {name!r} (use magi or opencv)")


async def run(ctx: JobContext) -> str:
    chapter = ctx.params.get("chapter_id")
    if not isinstance(chapter, str):
        raise JobError("vision needs params.chapter_id")
    detector = make_detector(ctx.params.get("detector", "magi"), ctx)
    store = ctx.store

    ctx.progress(0.02, "rasterizing")
    pages = await asyncio.to_thread(rasterize, store.path(ctx.input), int(ctx.params.get("target_width", 1600)))
    if not pages:
        raise JobError("the PDF has no pages")
    ctx.progress(0.15, f"checking {len(pages) - 1} page breaks for split drawings")
    source_layout = chapter_layout(pages)
    # Without an explicit direction, printed pages are read as manga (right to left) and strips left to right.
    rtl = bool(ctx.params["rtl"]) if "rtl" in ctx.params else source_layout == "page"
    joins = await asyncio.to_thread(find_joins, pages, rtl, 2.0, source_layout)
    canvases = await asyncio.to_thread(stitch, pages, joins, rtl)
    del pages

    # Everything detection runs on: whole pages and spreads, and the slices of strip runs.
    images: list[tuple[np.ndarray, int]] = []
    strip_rows = page_rows = 0
    for c, canvas in enumerate(canvases):
        if canvas.kind == "strip":
            strip_rows += canvas.image.shape[0]
            for top, bottom in await asyncio.to_thread(slice_strip, canvas.image):
                images.append((canvas.image[top:bottom], c))
        else:
            page_rows += canvas.image.shape[0]
            images.append((canvas.image, c))
    layout = "strip" if strip_rows > page_rows else "page"

    files: dict[str, str] = {}
    found: list[tuple[str, np.ndarray, Detection]] = []
    for n, (image, _) in enumerate(images, 1):
        rel = f"pages/page_{n:03d}.webp"
        files[rel] = await asyncio.to_thread(store.put, await asyncio.to_thread(encode_webp, image))
        detections = await detector.detect(image, store.path(files[rel]))
        for i in reading_order([d.box for d in detections], rtl):
            found.append((rel, image, detections[i]))
        ctx.progress(0.2 + 0.7 * n / len(images), f"{detector.name}: {n}/{len(images)} images, {len(found)} panels")
    if not found:
        raise JobError(f"{detector.name} found no panels in this chapter")

    panels = []
    bubble_n = 0
    for order, (rel, image, det) in enumerate(found):
        h, w = image.shape[:2]
        pid = f"p_{order + 1:04d}"
        x0, y0, x1, y1 = det.box
        crop = image[max(0, int(y0)) : min(h, int(np.ceil(y1))), max(0, int(x0)) : min(w, int(np.ceil(x1)))]
        crop_rel = f"panels/crops/{pid}.webp"
        files[crop_rel] = await asyncio.to_thread(store.put, await asyncio.to_thread(encode_webp, crop))
        bubbles = []
        # Bubbles in reading order, so narration follows the page.
        boxes = [Detection(b["polygon"]).box for b in det.bubbles]
        for b in (det.bubbles[i] for i in reading_order(boxes, rtl)):
            bubble_n += 1
            bubbles.append({"id": f"b_{bubble_n:04d}", "kind": b["kind"], "polygon": _norm_polygon(b["polygon"], w, h), "text": b.get("text"), "speaker": b.get("speaker")})
        panels.append(
            {
                "id": pid,
                "order": order,
                "page": f"chapters/{chapter}/{rel}",
                "page_size": {"width": w, "height": h},
                "bbox": _norm_box(x0, y0, x1, y1, w, h),
                "polygon": _norm_polygon(det.polygon, w, h),
                "crop": f"chapters/{chapter}/{crop_rel}",
                "bubbles": bubbles,
                "characters": [{"cluster": c["cluster"], "bbox": _norm_box(*c["bbox"], w, h)} for c in det.characters],
            }
        )
    doc = parse(PanelsDoc, {"schema_version": 1, "chapter_id": chapter, "layout": layout, "panels": panels})

    report = {
        "pages": sum(len(c.pages) for c in canvases),
        "detector": detector.name,
        "source_layout": source_layout,
        "rtl": rtl,
        "joins": [asdict(j) for j in joins],
        "canvases": [{"pages": [p + 1 for p in c.pages], "kind": c.kind, "size": [int(c.image.shape[1]), int(c.image.shape[0])]} for c in canvases],
        "images": len(images),
        "panels": len(panels),
    }
    manifest = {"schema_version": 1, "chapter_id": chapter, "panels": doc.model_dump(mode="json", exclude_none=True), "files": files, "report": report}
    ctx.progress(1.0, f"{len(panels)} panels from {report['pages']} pages")
    return store.put(json.dumps(manifest).encode())

