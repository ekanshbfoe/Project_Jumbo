import asyncio
import json

import cv2
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
from jumbo_worker.pipeline.vision.slicer import merge_slivers, slice_strip
from jumbo_worker.pipeline.vision.stitch import chapter_layout, find_joins, stitch

# A webtoon strip: four panels; the PDF pages cut it at 1500 and 3000 rows.
STRIP, SPANS = strip(800, [900, 1200, 700, 1000], gutter=60)


def test_a_page_break_through_a_panel_is_joined_and_one_in_a_gutter_is_not():
    through = next(y for y in range(1400, 1600) if any(a + 50 < y < b - 50 for a, b in SPANS))
    gutter = next((a + b) // 2 for (_, a), (b, _) in zip(SPANS, SPANS[1:]) if b - a >= 40)
    pages = cut(STRIP, sorted([through, gutter]))
    joins = find_joins(pages, layout="strip")
    kinds = {j.after_page: j.kind for j in joins}
    assert sorted(kinds.values()) == ["none", "vertical"], joins
    canvases = stitch(pages, joins)
    assert len(canvases) == 2 and canvases[0].kind in ("strip", "page")


def test_a_break_through_flat_colour_is_joined_but_a_white_gutter_is_not():
    sky = np.full((1200, 800, 3), (90, 140, 210), np.uint8)
    framed = np.full((1400, 800, 3), 255, np.uint8)
    framed[100:1300] = sky
    top, bottom = framed[:700], framed[700:]
    assert [j.kind for j in find_joins([top, bottom], layout="strip")] == ["vertical"]
    white = np.full((300, 800, 3), 255, np.uint8)
    [join] = find_joins([framed, white], layout="strip")
    assert join.kind == "none" and join.reason.startswith("vertical: white gutter on both sides")


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


def test_printed_pages_are_never_joined_vertically_and_busy_edges_are_no_spread():
    # Two manga pages ending and starting in the same black bleed would pass as a vertical cut.
    bleed = np.zeros((1400, 1000, 3), np.uint8)
    bleed[200:1200, 100:900] = 255
    pages = [bleed, bleed.copy()]
    assert chapter_layout(pages) == "page" and chapter_layout([np.zeros((3000, 800, 3), np.uint8)]) == "strip"
    [join] = find_joins(pages)
    assert join.kind == "none" and join.reason.startswith("vertical: printed pages do not continue vertically")
    # Unrelated speed-line edges: different on every line, so the line-to-line change alone would excuse them.
    rng = np.random.default_rng(5)
    noisy = [rng.integers(0, 256, (1400, 1000, 3), dtype=np.uint8) for _ in range(2)]
    [join] = find_joins(noisy, layout="page")
    assert join.kind == "none", join


def test_strips_are_sliced_at_their_gutters():
    slices = slice_strip(STRIP)
    assert [(t, b) for t, b in slices] == SPANS


def test_borderless_shots_fading_through_black_are_sliced_apart():
    width = 600
    shot_a, shot_b = art(700, width, seed=31), art(700, width, seed=32)
    fade = np.linspace(0, 1, 80)[:, None, None]
    black = np.zeros((90, width, 3), np.uint8)
    # A fades out to black, a long black band, B fades in; a short black stripe inside B stays.
    strip_image = np.vstack([shot_a, (shot_a[-1:] * (1 - fade)).astype(np.uint8), black, (shot_b[:1] * fade).astype(np.uint8), shot_b])
    strip_image[1700:1715] = 0
    slices = slice_strip(strip_image)
    assert len(slices) == 2, slices
    assert slices[0][1] <= 700 + 80 + 5 and slices[1][0] >= 700 + 80 + 90 - 5


def test_slivers_are_merged_into_the_neighbour_across_the_narrower_gap():
    # A caption line (20 rows) floating 50 rows under one panel and 200 rows above the next.
    assert merge_slivers([(0, 500), (550, 570), (770, 1300)], 100) == [(0, 570), (770, 1300)]
    assert merge_slivers([(0, 500), (700, 720), (750, 1300)], 100) == [(0, 500), (700, 1300)]
    assert merge_slivers([(0, 30)], 100) == [(0, 30)]
    # A strip with a thin caption between two panels slices into two images, not three.
    image = np.full((1500, 800, 3), 255, np.uint8)
    image[50:650] = art(600, 800, seed=1)
    image[700:730, 200:600] = 0
    image[900:1450] = art(550, 800, seed=2)
    assert slice_strip(image) == [(50, 730), (900, 1450)]


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
            "texts": [
                {"polygon": [[10, 130], [40, 130], [40, 150], [10, 150]], "kind": "speech", "text": "Run!", "speaker": "c_1"},
                # A caption floating below both panels belongs to the nearest one, not nowhere.
                {"polygon": [[10, 228], [90, 228], [90, 238], [10, 238]], "kind": "narration", "text": "Later...", "speaker": None},
            ],
            "characters": [{"bbox": [20, 20, 60, 90], "cluster": "c_1"}],
        }

    panels = await MagiDetector(call).detect(art(240, 200, seed=4), "x.webp")
    assert [len(p.bubbles) for p in panels] == [0, 2] and sorted(b["text"] for b in panels[1].bubbles) == ["Later...", "Run!"]
    assert [len(p.characters) for p in panels] == [1, 0]


def bubble_strip() -> tuple[np.ndarray, dict]:
    """A webtoon slice: a lone speech balloon on white above a drawn scene, as Magi reports it (the
    balloon boxed as a panel of its own, no character in it)."""
    image = np.full((900, 600, 3), 255, np.uint8)
    cv2.ellipse(image, (300, 150), (180, 90), 0, 0, 360, (0, 0, 0), 3)
    cv2.putText(image, "WHERE ARE YOU", (180, 160), cv2.FONT_HERSHEY_SIMPLEX, 0.8, (0, 0, 0), 2)
    image[300:880] = art(580, 600, seed=6)
    result = {
        "panels": [{"polygon": [[100, 50], [500, 50], [500, 250], [100, 250]]}, {"polygon": [[0, 300], [600, 300], [600, 880], [0, 880]]}],
        "texts": [{"polygon": [[175, 135], [425, 135], [425, 170], [175, 170]], "kind": "speech", "text": "Where are you?", "speaker": None}],
        "characters": [{"bbox": [200, 400, 400, 800], "cluster": "c_1"}],
    }
    return image, result


async def test_a_lone_bubble_box_gives_its_text_to_the_real_panel_without_moving_its_box():
    image, result = bubble_strip()

    async def call(model, kind, payload):
        return result

    detector = MagiDetector(call)
    [panel] = await detector.detect(image, "x.webp")
    assert detector.folded == 1
    # Data only: the scene keeps its own box (the crop stays on the artwork) and gains the line.
    assert panel.box == (0, 300, 600, 880)
    assert [b["text"] for b in panel.bubbles] == ["Where are you?"]


async def test_painted_art_without_a_detected_character_is_not_folded():
    # Smooth painted art has few hard edges; its colour keeps it a panel even when Magi finds no one in it.
    image, result = bubble_strip()
    image[50:250, 100:500] = art(200, 400, seed=8)
    detector = MagiDetector(lambda *_: asyncio.sleep(0, result))
    panels = await detector.detect(image, "x.webp")
    assert detector.folded == 0 and len(panels) == 2


def test_rasterize_keeps_the_drawn_pixels(tmp_path):
    pdf = write_pdf([grid_page()], tmp_path / "c.pdf")
    [page] = rasterize(pdf, target_width=1000)
    assert page.shape == (1400, 1000, 3)


def test_full_page_scans_are_not_upscaled(tmp_path):
    pdf = write_pdf([grid_page(w=600, h=840)], tmp_path / "small.pdf")
    [page] = rasterize(pdf, target_width=1600)
    assert page.shape[1] == 600


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
    store, job = await run_vision(tmp_path, [grid_page(), spread[:, :1000], spread[:, 1000:]], detector="opencv", rtl=False)
    assert job.status == "done", job.error
    manifest = json.loads(store.path(job.output).read_bytes())
    assert [c["kind"] for c in manifest["report"]["canvases"]] == ["page", "spread"]
    # 4 grid panels plus the spread as one full-bleed panel, not two halves.
    assert len(manifest["panels"]["panels"]) == 5 and manifest["panels"]["layout"] == "page"


async def test_magi_is_the_default_and_needs_the_pool(tmp_path):
    store, job = await run_vision(tmp_path, [grid_page()])
    assert job.status == "failed" and "no model pool" in job.error
