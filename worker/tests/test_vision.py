import asyncio
import json

import numpy as np
import pytest
from vision_fixtures import art, cut, grid_page, strip, write_pdf

from jumbo_worker.blobs import BlobStore
from jumbo_worker.contracts import PanelsDoc
from jumbo_worker.contracts.rules import parse
from jumbo_worker.events import EventBus
from jumbo_worker.jobs import JobContext, JobRunner
from jumbo_worker.pipeline.vision.detect import MagiDetector, OpenCVDetector
from jumbo_worker.pipeline.vision.order import reading_order
from jumbo_worker.pipeline.vision.raster import rasterize
from jumbo_worker.pipeline.vision.slicer import slice_strip
from jumbo_worker.pipeline.vision.stitch import find_joins, stitch

# A webtoon strip: four panels; the PDF pages cut it at 1500 and 3000 rows.
STRIP, SPANS = strip(800, [900, 1200, 700, 1000], gutter=60)


def test_a_page_break_through_a_panel_is_joined_and_one_in_a_gutter_is_not():
    through = next(y for y in range(1400, 1600) if any(a + 50 < y < b - 50 for a, b in SPANS))
    gutter = next((a + b) // 2 for (_, a), (b, _) in zip(SPANS, SPANS[1:]) if b - a >= 40)
    pages = cut(STRIP, sorted([through, gutter]))
    joins = find_joins(pages)
    kinds = {j.after_page: j.kind for j in joins}
    assert sorted(kinds.values()) == ["none", "vertical"], joins
    canvases = stitch(pages, joins)
    assert len(canvases) == 2 and canvases[0].kind in ("strip", "page")


def test_a_split_spread_is_joined_left_to_right_and_unrelated_pages_are_not():
    spread = art(1400, 2000, seed=3)
    left, right = spread[:, :1000], spread[:, 1000:]
    other = art(1400, 1000, seed=99)
    joins = find_joins([grid_page(), left, right, other])
    assert [j.kind for j in joins] == ["none", "horizontal", "none"], joins
    canvases = stitch([grid_page(), left, right, other], joins)
    assert [c.kind for c in canvases] == ["page", "spread", "page"]
    assert np.array_equal(canvases[1].image, spread)
    # Manga order: the later page is on the left.
    rtl = stitch([right, left], find_joins([right, left], rtl=True), rtl=True)
    assert len(rtl) == 1 and np.array_equal(rtl[0].image, spread)


def test_strips_are_sliced_at_their_gutters():
    slices = slice_strip(STRIP)
    assert [(t, b) for t, b in slices] == SPANS


def test_reading_order_rows_then_columns():
    boxes = [(500, 0, 900, 400), (0, 10, 450, 400), (0, 450, 900, 800)]
    assert reading_order(boxes) == [1, 0, 2]
    assert reading_order(boxes, rtl=True) == [0, 1, 2]


async def test_opencv_finds_the_panels_of_a_printed_page(tmp_path):
    found = await OpenCVDetector().detect(grid_page(), tmp_path / "unused.webp")
    assert len(found) == 4
    assert reading_order([d.box for d in found]) is not None


async def test_magi_results_are_attached_to_their_panels():
    async def call(model, kind, payload):
        assert (model, kind) == ("magi_v2", "detect")
        return {
            "panels": [{"polygon": [[0, 0], [100, 0], [100, 100], [0, 100]]}, {"polygon": [[0, 120], [100, 120], [100, 220], [0, 220]]}],
            "texts": [{"polygon": [[10, 130], [40, 130], [40, 150], [10, 150]], "kind": "speech", "text": "Run!", "speaker": "c_1"}],
            "characters": [{"bbox": [20, 20, 60, 90], "cluster": "c_1"}],
        }

    panels = await MagiDetector(call).detect(np.zeros((240, 120, 3), np.uint8), "x.webp")
    assert [len(p.bubbles) for p in panels] == [0, 1] and panels[1].bubbles[0]["text"] == "Run!"
    assert [len(p.characters) for p in panels] == [1, 0]


def test_rasterize_keeps_the_drawn_pixels(tmp_path):
    pdf = write_pdf([grid_page()], tmp_path / "c.pdf")
    [page] = rasterize(pdf, target_width=1000)
    assert page.shape == (1400, 1000, 3)


async def run_vision(tmp_path, images, **params):
    store = BlobStore(tmp_path / "blobs")
    pdf = write_pdf(images, tmp_path / "chapter.pdf")
    runner = JobRunner(store, EventBus())
    job = runner.submit("vision", store.put(pdf.read_bytes()), {"chapter_id": "ch_0003", "target_width": images[0].shape[1], **params})
    await asyncio.wait_for(job.task, 120)
    return store, job


async def test_vision_stage_recovers_every_panel_of_a_strip_cut_mid_panel(tmp_path):
    through = next(y for y in range(1400, 1600) if any(a + 50 < y < b - 50 for a, b in SPANS))
    store, job = await run_vision(tmp_path, cut(STRIP, [through, 3000]), detector="opencv")
    assert job.status == "done", job.error
    manifest = json.loads(store.path(job.output).read_bytes())
    doc = parse(PanelsDoc, manifest["panels"])
    # Four drawn panels, not five halves.
    assert len(doc.panels) == 4 and doc.layout == "strip"
    assert [p.id for p in doc.panels] == ["p_0001", "p_0002", "p_0003", "p_0004"]
    assert all(p.page.startswith("chapters/ch_0003/pages/") and p.crop.startswith("chapters/ch_0003/panels/crops/") for p in doc.panels)
    assert all(store.has(h) for h in manifest["files"].values())
    assert any(j["kind"] == "vertical" for j in manifest["report"]["joins"])


async def test_vision_stage_on_pages_and_a_spread(tmp_path):
    spread = art(1400, 2000, seed=5)
    store, job = await run_vision(tmp_path, [grid_page(), spread[:, :1000], spread[:, 1000:]], detector="opencv")
    assert job.status == "done", job.error
    manifest = json.loads(store.path(job.output).read_bytes())
    assert [c["kind"] for c in manifest["report"]["canvases"]] == ["page", "spread"]
    # 4 grid panels plus the spread as one full-bleed panel, not two halves.
    assert len(manifest["panels"]["panels"]) == 5 and manifest["panels"]["layout"] == "page"


async def test_magi_is_the_default_and_needs_the_pool(tmp_path):
    store, job = await run_vision(tmp_path, [grid_page()])
    assert job.status == "failed" and "no model pool" in job.error
