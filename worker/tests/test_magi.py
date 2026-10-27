"""Magi v2 plumbing without the model: tiling of tall images, merging, and the runner's output."""

import asyncio

import numpy as np

from jumbo_worker.models.magi import MagiRunner, to_contract
from jumbo_worker.pipeline.vision.detect import MagiDetector
from jumbo_worker.pipeline.vision.tiling import dedupe, merge_panels, tile_spans
from jumbo_worker.pool.specs import ModelSpec


def test_tall_images_are_cut_into_overlapping_page_shaped_tiles():
    assert tile_spans(1200, 900) == [(0, 1200)]
    spans = tile_spans(13553, 900)
    assert spans[0][0] == 0 and spans[-1][1] == 13553
    assert all(b - a == 1260 for a, b in spans)
    # Every row is covered and neighbours overlap.
    assert all(spans[i + 1][0] < spans[i][1] for i in range(len(spans) - 1))


SPANS = [(0, 1260), (945, 2205), (1890, 3150)]


def test_panel_pieces_across_tiles_merge_but_neighbours_do_not():
    tile_a = [(0, 0, 900, 400), (0, 900, 900, 1260)]  # second one is cut by the tile edge
    tile_b = [(0, 900, 900, 1700), (100, 1800, 500, 2100)]
    merged = merge_panels([tile_a, tile_b, []], SPANS)
    assert merged == [(0, 0, 900, 400), (0, 900, 900, 1700), (100, 1800, 500, 2100)]


def test_stacked_panels_seen_whole_and_cut_in_neighbouring_tiles_stay_separate():
    # Three full-width panels in a strip: 0-1100, 1150-2100, 2150-3100. Each tile sees some whole and
    # the ones crossing its edges cut off. The old rule chained all three into one box.
    tile_a = [(0, 0, 900, 1100), (0, 1150, 900, 1260)]
    tile_b = [(0, 945, 900, 1100), (0, 1150, 900, 2100), (0, 2150, 900, 2205)]
    tile_c = [(0, 1890, 900, 2100), (0, 2150, 900, 3100)]
    merged = merge_panels([tile_a, tile_b, tile_c], SPANS)
    assert merged == [(0, 0, 900, 1100), (0, 1150, 900, 2100), (0, 2150, 900, 3100)]


def test_a_panel_taller_than_the_overlap_is_rebuilt_from_its_pieces():
    tile_a = [(0, 200, 900, 1260)]
    tile_b = [(0, 945, 900, 2205)]
    tile_c = [(0, 1890, 900, 2600)]
    assert merge_panels([tile_a, tile_b, tile_c], SPANS) == [(0, 200, 900, 2600)]


def test_duplicates_in_an_overlap_are_kept_once():
    items = [{"b": (0, 0, 100, 50)}, {"b": (0, 5, 100, 50)}, {"b": (300, 0, 400, 50)}]
    assert len(dedupe(items, lambda i: i["b"])) == 2


async def test_magi_detector_tiles_a_tall_slice_and_shifts_results_back(tmp_path):
    seen = []

    async def call(model, kind, payload):
        seen.append(payload["image"])
        # Each tile reports one panel filling it and a bubble near its top.
        return {"panels": [{"polygon": [[0, 0], [100, 0], [100, 140], [0, 140]]}], "texts": [{"polygon": [[10, 10], [50, 10], [50, 30], [10, 30]], "kind": "speech", "text": "hi"}], "characters": []}

    image = np.zeros((600, 100, 3), np.uint8)
    path = tmp_path / "slice.webp"
    path.write_bytes(b"")
    panels = await MagiDetector(call).detect(image, path)
    assert len(seen) == len(tile_spans(600, 100)) > 1
    # Overlapping full-tile panels chain into one panel covering the slice; tiles are cleaned up.
    assert len(panels) == 1 and panels[0].box == (0, 0, 100, 600)
    assert len(panels[0].bubbles) == len(seen)
    assert list(tmp_path.iterdir()) == [path]


def test_runner_output_follows_the_detector_contract():
    result = {
        "panels": [[0, 0, 100, 200]],
        "texts": [[10, 10, 40, 30], [50, 50, 90, 70]],
        "characters": [[20, 40, 60, 180]],
        "character_cluster_labels": [3],
        "text_character_associations": [[0, 0]],
        "is_essential_text": [True, False],
    }
    out = to_contract(result, ["Who are you?", "BAM"])
    assert out["panels"][0]["polygon"] == [[0, 0], [100, 0], [100, 200], [0, 200]]
    assert out["texts"][0] == {"polygon": [[10, 10], [40, 10], [40, 30], [10, 30]], "kind": "speech", "text": "Who are you?", "speaker": "c_3"}
    assert out["texts"][1]["kind"] == "sfx" and out["texts"][1]["speaker"] is None
    assert out["characters"] == [{"bbox": [20.0, 40.0, 60.0, 180.0], "cluster": "c_3"}]


async def test_runner_reads_the_image_and_calls_the_model(tmp_path):
    import cv2

    path = tmp_path / "p.png"
    cv2.imwrite(str(path), np.full((50, 40, 3), 200, np.uint8))

    class Model:
        def predict_detections_and_associations(self, images):
            assert images[0].shape == (50, 40, 3)
            return [{"panels": [[0, 0, 40, 50]], "texts": [], "characters": [], "character_cluster_labels": [], "text_character_associations": [], "is_essential_text": []}]

        def predict_ocr(self, images, texts):
            raise AssertionError("no texts, no OCR")

    runner = MagiRunner(ModelSpec(name="magi_v2", backend="torch-xla", chips=1, hbm_gb_per_chip=5, phase="vision"), model=Model())
    out = await runner.run("detect", {"image": str(path)})
    assert out["panels"][0]["polygon"][2] == [40.0, 50.0]


async def test_runner_returns_the_crop_embeddings_magi_made_while_grouping(tmp_path):
    import base64

    import cv2

    path = tmp_path / "p.png"
    cv2.imwrite(str(path), np.full((50, 40, 3), 200, np.uint8))
    calls = []

    class Model:
        def predict_crop_embeddings(self, images, crop_bboxes, move_to_device_fn=None):
            calls.append(len(crop_bboxes[0]))
            return [np.array([[0.5] * 768, [-2.0] * 768])]

        def predict_detections_and_associations(self, images):
            # As in Magi: the crop encoder runs inside detection, to group the characters.
            self.predict_crop_embeddings(images, [[[0, 0, 10, 10], [5, 5, 20, 20]]])
            return [{"panels": [], "texts": [], "characters": [[0, 0, 10, 10], [5, 5, 20, 20]], "character_cluster_labels": [0, 1], "text_character_associations": [], "is_essential_text": []}]

    runner = MagiRunner(ModelSpec(name="magi_v2", backend="torch-xla", chips=1, hbm_gb_per_chip=5, phase="vision"), model=Model())
    out = await runner.run("detect", {"image": str(path)})
    # Encoded once, by Magi itself: no second pass for the fingerprints.
    assert calls == [2]
    rows = [np.frombuffer(base64.b64decode(c["fingerprint"]), "<f2") for c in out["characters"]]
    assert [r.shape for r in rows] == [(768,), (768,)] and rows[0][0] == 0.5 and rows[1][-1] == -2.0
