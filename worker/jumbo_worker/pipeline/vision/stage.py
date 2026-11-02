"""The `vision` job: chapter PDF in, a manifest blob out.

Rasterize -> join pages split across page breaks (stitch) -> slice strip runs at gutters -> detect
panels -> reading order -> crops -> PanelsDoc, validated against the shared contract.

Params: `chapter_id` (required), `detector` ("magi", the default and the production path, or
"opencv" for local plumbing tests), `target_width` (default 1600), `rtl` (default: true for
printed-page chapters, read as manga, false for strips).

The output manifest is JSON:

    {"schema_version": 1, "chapter_id": "ch_0001",
     "panels": <PanelsDoc>,                       # paths are project-relative (chapters/<id>/...)
     "files": {"pages/page_001.webp": "<blake3>", "panels/crops/p_0001.webp": "<blake3>", ...,
               "panels/numbered.pdf": "<blake3>",   # every page, panels outlined and labelled (Method 2)
               "panels/numbered.json": "<blake3>"},  # {"chapter", "pack", "order", "pages", "bytes"} for checking answers
     "report": {"pages": n, "joins": [...], "canvases": [...], "detector": "magi"}}

`files` keys are relative to the chapter folder; the app downloads each blob to that path.
"""

from __future__ import annotations

import asyncio
import base64
import json
import re
from dataclasses import asdict

import cv2
import numpy as np

from ...contracts import PanelsDoc
from ...contracts.rules import parse
from ...jobs import JobContext, JobError
from .backdrop import encode_jpeg, make_backdrop
from .detect import Detection, Detector, MagiDetector, OpenCVDetector
from .numbered import numbered_pdf, pack_id
from .order import reading_order
from .raster import rasterize
from .slicer import slice_strip
from .stitch import chapter_layout, find_joins, stitch

WEBP_QUALITY = 92
# Images detected at once. The pool scales replicas from its backlog, so a chapter only spreads over
# several replicas (and GPUs) when this many requests are in flight together.
PARALLEL_IMAGES = 8
# Pages thinner than this many pixels are dropped before anything else.
MIN_PAGE_SIDE = 16


def encode_webp(image: np.ndarray) -> bytes:
    ok, buf = cv2.imencode(".webp", cv2.cvtColor(image, cv2.COLOR_RGB2BGR), [cv2.IMWRITE_WEBP_QUALITY, WEBP_QUALITY])
    if not ok:
        raise RuntimeError("webp encoding failed")
    return buf.tobytes()


def _unit(v: float) -> float:
    return round(min(1.0, max(0.0, v)), 6)


def _norm_polygon(points, w: int, h: int) -> list[list[float]]:
    return [[_unit(x / w), _unit(y / h)] for x, y in points]


# Most a panel's crop grows past its border to take in a bubble spilling over it, as a share of the panel's side.
BUBBLE_REACH = 0.15


def grow_to_bubbles(det: Detection, w: int, h: int) -> tuple[float, float, float, float]:
    """The panel's box grown to cover the bubbles that spill over its border, as webtoon text often does (a line
    cut mid-word at the frame's edge in the first real video, 2026-11-02); only text with words counts. Each side
    grows by at most
    BUBBLE_REACH of the panel, so a caption floating far off never drags the crop along."""
    x0, y0, x1, y1 = det.box
    rx, ry = (x1 - x0) * BUBBLE_REACH, (y1 - y0) * BUBBLE_REACH
    gx0, gy0, gx1, gy1 = x0, y0, x1, y1
    for b in det.bubbles:
        if not any(c.isalnum() for c in b.get("text") or ""):
            continue  # a mark like "....." or "!!??" is drawn art that spills on purpose, not words to keep whole
        bx0, by0, bx1, by1 = Detection(b["polygon"]).box
        if bx1 <= x0 or bx0 >= x1 or by1 <= y0 or by0 >= y1:
            continue  # wholly outside: a caption given to the nearest panel, not one crossing its border
        gx0, gy0 = min(gx0, max(bx0, x0 - rx)), min(gy0, max(by0, y0 - ry))
        gx1, gy1 = max(gx1, min(bx1, x1 + rx)), max(gy1, min(by1, y1 + ry))
    return max(0.0, gx0), max(0.0, gy0), min(float(w), gx1), min(float(h), gy1)


def _norm_box(x0: float, y0: float, x1: float, y1: float, w: int, h: int) -> dict:
    x0, y0 = max(0.0, x0), max(0.0, y0)
    x1, y1 = min(float(w), x1), min(float(h), y1)
    bx, by = _unit(x0 / w), _unit(y0 / h)
    return {"x": bx, "y": by, "w": max(1e-6, min(1.0 - bx, round((x1 - x0) / w, 6))), "h": max(1e-6, min(1.0 - by, round((y1 - y0) / h, 6)))}


# Text that only a credits or scanlation page carries.
CREDITS = re.compile(r"download|scanlat|@\w{3,}|discord|patreon|t\.me/|\.(com|org|net)\b|read (it )?(at|on)\b|mangastore", re.I)


def is_credits(det: Detection) -> bool:
    """Every text on the box is a credit. Drawn figures do not count against it: credits pages often carry a mascot,
    which Magi reports as a character. Only the first and last image are ever checked."""
    texts = [b.get("text") or "" for b in det.bubbles]
    return bool(texts) and all(CREDITS.search(t) for t in texts)


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
    # A sliver page (a 1080 x 2 leftover at the end of an export) holds nothing to detect or join.
    pages = [p for p in pages if min(p.shape[:2]) >= MIN_PAGE_SIDE]
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
    per_image: list[list[Detection]] = [[] for _ in images]
    gate = asyncio.Semaphore(PARALLEL_IMAGES)
    done = 0

    async def detect_one(n: int, image: np.ndarray) -> None:
        nonlocal done
        rel = f"pages/page_{n + 1:03d}.webp"
        async with gate:
            files[rel] = await asyncio.to_thread(store.put, await asyncio.to_thread(encode_webp, image))
            per_image[n] = await detector.detect(image, store.path(files[rel]))
        done += 1
        ctx.progress(0.2 + 0.7 * done / len(images), f"{detector.name}: {done}/{len(images)} images")

    tasks = [asyncio.ensure_future(detect_one(n, image)) for n, (image, _) in enumerate(images)]
    try:
        await asyncio.gather(*tasks)
    finally:
        # One failed image fails the job: stop the others instead of leaving them running.
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
    # Credits and scanlation pages (first or last image only): every text on them is a credit.
    dropped_credits = 0
    for n in {0, len(images) - 1}:
        kept = [d for d in per_image[n] if not is_credits(d)]
        dropped_credits += len(per_image[n]) - len(kept)
        per_image[n] = kept
    # Back in chapter order, whatever order the replicas finished in.
    found: list[tuple[int, str, np.ndarray, Detection]] = []
    for n, ((image, _), detections) in enumerate(zip(images, per_image)):
        for i in reading_order([d.box for d in detections], rtl):
            found.append((n, f"pages/page_{n + 1:03d}.webp", image, detections[i]))
    if not found:
        raise JobError(f"{detector.name} found no panels in this chapter")

    panels = []
    bubble_n = 0
    # Character boxes get chapter-wide ids (k_0001, ...) and their fingerprints rows in that order.
    # Clusters are per image, so they are prefixed with it: i003.c_1 is not i004.c_1.
    rows: list[bytes] = []
    box_n = 0
    for order, (n, rel, image, det) in enumerate(found):
        prefix = f"i{n + 1:03d}."
        h, w = image.shape[:2]
        pid = f"p_{order + 1:04d}"
        x0, y0, x1, y1 = grow_to_bubbles(det, w, h)
        # A grown box is shown as a plain rectangle: the outline cannot be grown to match.
        grown = (x0, y0, x1, y1) != grow_to_bubbles(Detection(det.polygon), w, h)
        polygon = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] if grown else det.polygon
        crop = image[max(0, int(y0)) : min(h, int(np.ceil(y1))), max(0, int(x0)) : min(w, int(np.ceil(x1)))]
        crop_rel = f"panels/crops/{pid}.webp"
        files[crop_rel] = await asyncio.to_thread(store.put, await asyncio.to_thread(encode_webp, crop))
        backdrop_rel = f"panels/backdrops/{pid}.jpg"
        files[backdrop_rel] = await asyncio.to_thread(store.put, await asyncio.to_thread(lambda c=crop: encode_jpeg(make_backdrop(c))))
        bubbles = []
        # Bubbles in reading order, so narration follows the page.
        boxes = [Detection(b["polygon"]).box for b in det.bubbles]
        for b in (det.bubbles[i] for i in reading_order(boxes, rtl)):
            bubble_n += 1
            bubbles.append({"id": f"b_{bubble_n:04d}", "kind": b["kind"], "polygon": _norm_polygon(b["polygon"], w, h), "text": b.get("text"), "speaker": prefix + b["speaker"] if b.get("speaker") else None})
        characters = []
        for c in det.characters:
            box_n += 1
            characters.append({"id": f"k_{box_n:04d}", "cluster": prefix + c["cluster"], "bbox": _norm_box(*c["bbox"], w, h)})
            if c.get("fingerprint"):
                rows.append(base64.b64decode(c["fingerprint"]))
        panels.append(
            {
                "id": pid,
                "order": order,
                "page": f"chapters/{chapter}/{rel}",
                "page_size": {"width": w, "height": h},
                "bbox": _norm_box(x0, y0, x1, y1, w, h),
                "polygon": _norm_polygon(polygon, w, h),
                "crop": f"chapters/{chapter}/{crop_rel}",
                "backdrop": f"chapters/{chapter}/{backdrop_rel}",
                "bubbles": bubbles,
                "characters": characters,
                **({"text_only": True} if det.text_only else {}),
            }
        )
    # Method 2's pack for Gemini: every page with its panels outlined and labelled by id.
    by_image: dict[int, list[tuple[str, list[list[float]]]]] = {}
    for order, (n, _, _, det) in enumerate(found):
        by_image.setdefault(n, []).append((f"p_{order + 1:04d}", det.polygon))
    numbered_pages = [(images[n][0], by_image[n]) for n in sorted(by_image)]
    pack = pack_id(chapter, numbered_pages)
    pdf, pdf_pages = await asyncio.to_thread(numbered_pdf, chapter, pack, numbered_pages)
    files["panels/numbered.pdf"] = await asyncio.to_thread(store.put, pdf)
    # What the app checks a pasted Gemini answer against: this chapter, this export, these panels in reading order.
    numbered = {"chapter": chapter, "pack": pack, "order": [f"p_{i + 1:04d}" for i in range(len(found))], "pages": pdf_pages, "bytes": len(pdf)}
    files["panels/numbered.json"] = await asyncio.to_thread(store.put, json.dumps(numbered).encode())
    fingerprints = None
    # Only a complete set is kept: a row missing would shift every box after it.
    if box_n and len(rows) == box_n and len({len(r) for r in rows}) == 1:
        files["panels/fingerprints.f16"] = await asyncio.to_thread(store.put, b"".join(rows))
        fingerprints = {"path": f"chapters/{chapter}/panels/fingerprints.f16", "dtype": "float16", "dim": len(rows[0]) // 2, "count": box_n,
                        "model": getattr(detector, "fingerprint_model", detector.name)}
    doc = parse(PanelsDoc, {"schema_version": 1, "chapter_id": chapter, "layout": layout, "panels": panels, **({"fingerprints": fingerprints} if fingerprints else {})})

    report = {
        "pages": sum(len(c.pages) for c in canvases),
        "detector": detector.name,
        "source_layout": source_layout,
        "rtl": rtl,
        "joins": [asdict(j) for j in joins],
        "canvases": [{"pages": [p + 1 for p in c.pages], "kind": c.kind, "size": [int(c.image.shape[1]), int(c.image.shape[0])]} for c in canvases],
        "images": len(images),
        "panels": len(panels),
        "text_only_folded": getattr(detector, "folded", 0),
        "credits_dropped": dropped_credits,
        "fingerprints": len(rows) if fingerprints else 0,
    }
    manifest = {"schema_version": 1, "chapter_id": chapter, "panels": doc.model_dump(mode="json", exclude_none=True), "files": files, "report": report}
    ctx.progress(1.0, f"{len(panels)} panels from {report['pages']} pages")
    return store.put(json.dumps(manifest).encode())

