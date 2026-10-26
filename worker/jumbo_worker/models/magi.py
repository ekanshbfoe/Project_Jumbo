"""Magi v2 (ragavsachdeva/magiv2) as a replica runner: panels, text boxes with OCR, speakers and
character clusters for one image, returned in the contract `pipeline/vision/detect.py` documents.

Weights: personal, research and non-commercial use (see the model card). They come from the staged
copy in `JUMBO_WEIGHTS` when the pool provides one, else from the Hugging Face Hub at a pinned
revision. Runs on the TPU through torch-xla when the replica holds chips, on its CUDA card on a
GPU host, otherwise on CPU.
"""

from __future__ import annotations

import asyncio
import contextlib
import os

import numpy as np

from ..pool.specs import ModelSpec

MODEL_ID = "ragavsachdeva/magiv2"
# Pinned so the remote code the model ships with cannot change under us.
REVISION = "fbc890fec52977142e8ee00bfe26e9458b65517c"


def read_image(path: str) -> np.ndarray:
    """Magi was trained on greyscale pages fed as RGB (the model card converts L -> RGB)."""
    import cv2

    grey = cv2.imread(path, cv2.IMREAD_GRAYSCALE)
    if grey is None:
        raise FileNotFoundError(f"cannot read image {path}")
    return cv2.cvtColor(grey, cv2.COLOR_GRAY2RGB)


def to_contract(result: dict, ocr: list[str]) -> dict:
    """Magi's per-image result -> {"panels", "texts", "characters"} in pixel coordinates."""

    def rect(b) -> list[list[float]]:
        x0, y0, x1, y1 = (float(v) for v in b)
        return [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]

    clusters = [f"c_{int(label)}" for label in result.get("character_cluster_labels", [])]
    speaker = {int(t): int(c) for t, c in result.get("text_character_associations", [])}
    essential = result.get("is_essential_text", [])
    texts = []
    for i, box in enumerate(result.get("texts", [])):
        c = speaker.get(i)
        texts.append(
            {
                "polygon": rect(box),
                # Magi marks dialogue and narration essential; the rest is sound effects and background text.
                "kind": "speech" if i < len(essential) and essential[i] else "sfx",
                "text": ocr[i] if i < len(ocr) else None,
                "speaker": clusters[c] if c is not None and c < len(clusters) else None,
            }
        )
    return {
        "panels": [{"polygon": rect(b)} for b in result.get("panels", [])],
        "texts": texts,
        "characters": [{"bbox": [float(v) for v in b], "cluster": clusters[i] if i < len(clusters) else f"c_{i}"} for i, b in enumerate(result.get("characters", []))],
    }


class MagiRunner:
    def __init__(self, spec: ModelSpec, model=None) -> None:
        self.spec = spec
        self.model = model if model is not None else self._load()
        # One forward pass at a time: the model is not safe to call from several threads.
        self.lock = asyncio.Lock()

    @staticmethod
    def _load():
        import torch
        from transformers import AutoModel

        torch.set_num_threads(max(1, int(os.environ.get("OMP_NUM_THREADS", "1"))))
        staged = os.environ.get("JUMBO_WEIGHTS")
        model = AutoModel.from_pretrained(staged or MODEL_ID, revision=None if staged else REVISION, trust_remote_code=True)
        if os.environ.get("PJRT_DEVICE") == "TPU":
            import torch_xla.core.xla_model as xm

            model = model.to(xm.xla_device())
        elif os.environ.get("JUMBO_DEVICE") == "cuda" and torch.cuda.is_available():
            # The pool pins this replica to one card with CUDA_VISIBLE_DEVICES and caps its allocator
            # at the ceiling's share of the card (the card is shared, so the watchdog checks the sum).
            torch.cuda.set_per_process_memory_fraction(float(os.environ.get("JUMBO_GPU_MEMORY_FRACTION", "0.8")), 0)
            model = model.to("cuda")
        return model.eval()

    def _detect(self, image: np.ndarray) -> dict:
        try:
            import torch

            no_grad = torch.no_grad
        except ImportError:  # only stand-in models in tests run without torch
            no_grad = contextlib.nullcontext
        with no_grad():
            [result] = self.model.predict_detections_and_associations([image])
            ocr = self.model.predict_ocr([image], [result["texts"]])[0] if result.get("texts") else []
        return to_contract(result, ocr)

    async def run(self, kind: str, payload: dict) -> object:
        if kind != "detect":
            raise ValueError(f"magi_v2 does not serve {kind!r}")
        image = await asyncio.to_thread(read_image, payload["image"])
        async with self.lock:
            return await asyncio.to_thread(self._detect, image)

    def memory(self) -> dict[int, float]:
        return {}
