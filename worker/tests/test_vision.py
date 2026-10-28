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
from jumbo_worker.pipeline.vision.detect import Detection, MagiDetector, OpenCVDetector
from jumbo_worker.pipeline.vision.order import reading_order
from jumbo_worker.pipeline.vision.raster import rasterize
from jumbo_worker.pipeline.vision.slicer import merge_slivers, slice_strip, split_tall
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
    assert join.kind == "none" and join.reason == "white gutter on both sides"


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


def test_a_webtoon_cut_into_short_pages_is_routed_as_a_strip_and_never_as_spreads():
    # A borderless scroll exported as page-shaped 1080 x 900 slices: every break runs through the drawing.
    scroll = art(4500, 1080, seed=11)
    pages = cut(scroll, [900, 1800, 2700, 3600])
    assert chapter_layout(pages) == "strip"
    joins = find_joins(pages, layout=chapter_layout(pages))
    assert [j.kind for j in joins] == ["vertical"] * 4
    [canvas] = stitch(pages, joins)
    assert canvas.kind == "strip" and np.array_equal(canvas.image, scroll)
    # Printed pages with white margins never vote for a strip, and a single break is not enough.
    assert chapter_layout([grid_page(seed=s) for s in range(4)]) == "page"
    assert chapter_layout(cut(scroll[:1800], [900])) == "page"


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


def test_a_panel_far_taller_than_wide_is_cut_at_its_quietest_rows():
    width = 500
    shots = [art(1000, width, seed=s) for s in (41, 42, 43)]
    calm = np.full((60, width, 3), 200, np.uint8)
    image = np.vstack([shots[0], calm, shots[1], calm, shots[2]])
    pieces = split_tall(image, (0, 0, width, image.shape[0]))
    assert len(pieces) == 3
    # Each cut lands inside a calm band, and the pieces tile the box.
    for (_, _, _, bottom), (_, top, _, _) in zip(pieces, pieces[1:]):
        assert bottom == top and (1000 <= top <= 1060 or 2060 <= top <= 2120), pieces
    assert pieces[0][1] == 0 and pieces[-1][3] == image.shape[0]
    assert split_tall(image, (0, 0, width, 1400)) == [(0, 0, width, 1400)]


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
    # Each panel's blurred backdrop is made here, once, for the renderer.
    assert [p.backdrop for p in doc.panels] == [f"chapters/ch_0003/panels/backdrops/{p.id}.jpg" for p in doc.panels]
    assert all(f"panels/backdrops/{p.id}.jpg" in manifest["files"] for p in doc.panels)
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
    # Method 2's numbered pack: one PDF page per canvas, each panel's id written on it.
    import fitz

    pack = fitz.open(stream=store.path(manifest["files"]["panels/numbered.pdf"]).read_bytes(), filetype="pdf")
    assert pack.page_count == 2


async def test_images_are_detected_in_parallel_and_kept_in_chapter_order(tmp_path, monkeypatch):
    from jumbo_worker.pipeline.vision import stage

    running = peak = 0
    full = asyncio.Event()

    class SlowDetector:
        name = "slow"

        async def detect(self, image, path):
            nonlocal running, peak
            running += 1
            peak = max(peak, running)
            if running == stage.PARALLEL_IMAGES:
                full.set()
            # Hold until the stage has a full batch in flight, however slow encoding the pages is.
            await asyncio.wait_for(full.wait(), 10)
            # Later pages finish first, so order can only come from the stage, not from timing.
            await asyncio.sleep(0.05 * (1 + (image.shape[0] % 7 == 0)))
            running -= 1
            h, w = image.shape[:2]
            return [Detection([(0, 0), (w, 0), (w, h), (0, h)])]

    monkeypatch.setattr(stage, "make_detector", lambda name, ctx: SlowDetector())
    pages = [grid_page(h=1400 + 7 * i) for i in range(12)]
    store, job = await run_vision(tmp_path, pages)
    assert job.status == "done", job.error
    assert peak == stage.PARALLEL_IMAGES
    panels = json.loads(store.path(job.output).read_bytes())["panels"]["panels"]
    assert [p["page"] for p in panels] == [f"chapters/ch_0003/pages/page_{n:03d}.webp" for n in range(1, 13)]


async def test_magi_is_the_default_and_needs_the_pool(tmp_path):
    store, job = await run_vision(tmp_path, [grid_page()])
    assert job.status == "failed" and "no model pool" in job.error


async def test_character_boxes_get_chapter_ids_clusters_per_image_and_one_fingerprint_row_each(tmp_path, monkeypatch):
    from jumbo_worker.models.magi import encode_fingerprint
    from jumbo_worker.pipeline.vision import stage

    class TwoPerPage:
        name = "magi"
        fingerprint_model = "magi-test"

        async def detect(self, image, path):
            h, w = image.shape[:2]
            seed = float(image.shape[0])
            panel = Detection([(0, 0), (w, 0), (w, h), (0, h)])
            panel.bubbles = [{"kind": "speech", "polygon": [(10, 10), (60, 10), (60, 40), (10, 40)], "text": "Hey", "speaker": "c_0"}]
            panel.characters = [
                {"cluster": "c_0", "bbox": (10, 50, 200, 400), "fingerprint": encode_fingerprint(np.full(768, seed))},
                {"cluster": "c_1", "bbox": (300, 50, 500, 400), "fingerprint": encode_fingerprint(np.full(768, -seed))},
            ]
            return [panel]

    monkeypatch.setattr(stage, "make_detector", lambda name, ctx: TwoPerPage())
    store, job = await run_vision(tmp_path, [grid_page(h=1400), grid_page(h=1407)])
    assert job.status == "done", job.error
    manifest = json.loads(store.path(job.output).read_bytes())
    doc = parse(PanelsDoc, manifest["panels"])
    chars = [c for p in doc.panels for c in p.characters]
    assert [c.id for c in chars] == ["k_0001", "k_0002", "k_0003", "k_0004"]
    # c_0 of page 1 and c_0 of page 2 are different clusters; the bubble points at its own page's.
    assert [c.cluster for c in chars] == ["i001.c_0", "i001.c_1", "i002.c_0", "i002.c_1"]
    assert [p.bubbles[0].speaker for p in doc.panels] == ["i001.c_0", "i002.c_0"]
    fp = doc.fingerprints
    assert (fp.dtype, fp.dim, fp.count, fp.model) == ("float16", 768, 4, "magi-test")
    rows = np.frombuffer(store.path(manifest["files"]["panels/fingerprints.f16"]).read_bytes(), "<f2").reshape(fp.count, fp.dim)
    assert rows[:, 0].tolist() == [1400, -1400, 1407, -1407] and fp.path == "chapters/ch_0003/panels/fingerprints.f16"
    assert manifest["report"]["fingerprints"] == 4


async def test_tiles_of_a_tall_image_keep_their_clusters_apart(tmp_path):
    from jumbo_worker.pipeline.vision.tiling import tile_spans

    async def call(model, kind, payload):
        return {"panels": [], "texts": [{"polygon": [[10, 10], [50, 10], [50, 30], [10, 30]], "kind": "speech", "text": "hi", "speaker": "c_0"}],
                "characters": [{"bbox": [10, 40, 60, 90], "cluster": "c_0", "fingerprint": "AAAA"}]}

    image = art(6000, 800, seed=3)
    assert len(tile_spans(6000, 800)) > 1
    panels = await MagiDetector(call).detect(image, tmp_path / "x.webp")
    chars = [c for p in panels for c in p.characters]
    clusters = {c["cluster"] for c in chars}
    assert len(clusters) == len(chars) and all(c.startswith("t") and c.endswith(".c_0") for c in clusters)
    assert all(c["fingerprint"] == "AAAA" for c in chars)
    assert {b["speaker"] for p in panels for b in p.bubbles} <= clusters


def test_backdrop_is_small_blurred_and_darkened(tmp_path):
    from jumbo_worker.pipeline.vision.backdrop import BRIGHTNESS, WIDTH, backfill, make_backdrop

    img = np.zeros((900, 600, 3), np.uint8)
    img[:, 300:] = 255  # a hard black/white edge
    out = make_backdrop(img)
    assert out.shape == (900 * WIDTH // 600, WIDTH, 3)
    # Darkened: white becomes 40%; blurred: the edge spreads over many pixels.
    assert abs(int(out[:, -1].mean()) - round(255 * BRIGHTNESS)) <= 2
    row = out[out.shape[0] // 2, :, 0].astype(int)
    assert np.count_nonzero((row > 5) & (row < round(255 * BRIGHTNESS) - 5)) > 20

    # Backfill an analyzed chapter that has crops but no backdrops yet.
    chapter = tmp_path / "chapters" / "ch_0001"
    (chapter / "panels" / "crops").mkdir(parents=True)
    cv2.imwrite(str(chapter / "panels/crops/p_0001.webp"), img)
    (chapter / "panels/panels.json").write_text(json.dumps({"panels": [{"id": "p_0001", "crop": "chapters/ch_0001/panels/crops/p_0001.webp"}, {"id": "p_0002"}]}))
    assert backfill(chapter) == 1
    doc = json.loads((chapter / "panels/panels.json").read_text())
    assert doc["panels"][0]["backdrop"] == "chapters/ch_0001/panels/backdrops/p_0001.jpg" and "backdrop" not in doc["panels"][1]
    assert (chapter / "panels/backdrops/p_0001.jpg").stat().st_size > 0


def test_numbered_labels_sit_over_the_gutter_and_never_overlap():
    from jumbo_worker.pipeline.vision.numbered import label_rects

    # p_0001 has room above it; p_0002 and p_0003 share a top-left corner (an inset panel).
    panels = [("p_0001", [[20, 100], [380, 100], [380, 280], [20, 280]]), ("p_0002", [[20, 320], [380, 320], [380, 580], [20, 580]]),
              ("p_0003", [[20, 320], [200, 320], [200, 450], [20, 450]])]
    rects = {pid: rect for pid, rect, _ in label_rects(panels, 400, 600, 30)}
    assert rects["p_0001"][3] <= 100, "above the panel, not over its art"
    boxes = list(rects.values())
    assert not any(a[0] < b[2] and b[0] < a[2] and a[1] < b[3] and b[1] < a[3] for i, a in enumerate(boxes) for b in boxes[i + 1:])
    assert all(0 <= r[0] and r[2] <= 400 and 0 <= r[1] and r[3] <= 600 for r in boxes)


def test_numbered_labels_stay_readable_after_gemini_scales_the_page():
    from jumbo_worker.pipeline.vision.numbered import LABEL_PX, MAX_SIDE, TALL, label_px

    # A 1600 px wide webtoon slice is cut into pages at most TALL widths tall, each scaled to MAX_SIDE.
    px = label_px(1600, 12000)
    assert px * MAX_SIDE / (TALL * 1600) >= LABEL_PX - 1


def test_a_tall_page_becomes_overlapping_pdf_pages_with_the_pack_stamp():
    import fitz
    import numpy as np

    from jumbo_worker.pipeline.vision.numbered import _slices, numbered_pdf, pack_id

    assert _slices(600, 400) == [(0, 600)]
    parts = _slices(4000, 400)
    assert parts[0][0] == 0 and parts[-1][1] == 4000 and all(b[0] < a[1] for a, b in zip(parts, parts[1:]))
    pages = [(np.full((4000, 400, 3), 230, np.uint8), [("p_0001", [[10, 10], [390, 10], [390, 3990], [10, 3990]])])]
    pack = pack_id("ch_0002", pages)
    data, count = numbered_pdf("ch_0002", pack, pages)
    doc = fitz.open(stream=data, filetype="pdf")
    assert count == doc.page_count == len(parts)
    # The id changes when any outline does.
    moved = [(pages[0][0], [("p_0001", [[12, 10], [390, 10], [390, 3990], [10, 3990]])])]
    assert len(pack) == 4 and pack_id("ch_0002", moved) != pack


def test_the_numbered_pdf_is_kept_under_the_size_limit():
    import numpy as np

    from jumbo_worker.pipeline.vision.numbered import numbered_pdf

    rng = np.random.default_rng(1)
    pages = [(rng.integers(0, 255, (1200, 800, 3), np.uint8), [("p_0001", [[10, 10], [790, 10], [790, 1190], [10, 1190]])]) for _ in range(3)]
    big, _ = numbered_pdf("ch_0001", "abcd", pages)
    small, _ = numbered_pdf("ch_0001", "abcd", pages, max_bytes=len(big) // 3)
    assert len(small) <= len(big) // 3
