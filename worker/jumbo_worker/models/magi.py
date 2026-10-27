"""Magi v2 (ragavsachdeva/magiv2) as a replica runner: panels, text boxes with OCR, speakers,
character clusters and character fingerprints for one image, returned in the contract
`pipeline/vision/detect.py` documents.

Fingerprints are the crop-encoder embeddings Magi already computes to group the characters on a
page; they are kept instead of recomputed, so returning them costs no extra model work.

Weights: personal, research and non-commercial use (see the model card). They come from the staged
copy in `JUMBO_WEIGHTS` when the pool provides one, else from the Hugging Face Hub at a pinned
revision. Runs on the TPU through torch-xla when the replica holds chips, on its CUDA card on a
GPU host, otherwise on CPU.
"""

from __future__ import annotations

import asyncio
import base64
import contextlib
import os

import numpy as np

from ..pool.specs import ModelSpec

MODEL_ID = "ragavsachdeva/magiv2"
# Pinned so the remote code the model ships with cannot change under us.
REVISION = "fbc890fec52977142e8ee00bfe26e9458b65517c"
# Names the encoder behind the fingerprints: those of another encoder or revision are not comparable.
FINGERPRINT_MODEL = f"{MODEL_ID}@{REVISION[:12]}/crop_embedding_model"


def encode_fingerprint(vector) -> str:
    """One embedding as base64 of little-endian float16 (768 values -> 1536 bytes)."""
    return base64.b64encode(np.asarray(vector, dtype="<f2").tobytes()).decode()


def read_image(path: str) -> np.ndarray:
    """Magi was trained on greyscale pages fed as RGB (the model card converts L -> RGB)."""
    import cv2

    grey = cv2.imread(path, cv2.IMREAD_GRAYSCALE)
    if grey is None:
        raise FileNotFoundError(f"cannot read image {path}")
    return cv2.cvtColor(grey, cv2.COLOR_GRAY2RGB)


def to_contract(result: dict, ocr: list[str], fingerprints=None) -> dict:
    """Magi's per-image result -> {"panels", "texts", "characters"} in pixel coordinates.
    `fingerprints` holds one embedding per character, in the order of result["characters"]."""

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
    characters = []
    for i, b in enumerate(result.get("characters", [])):
        character = {"bbox": [float(v) for v in b], "cluster": clusters[i] if i < len(clusters) else f"c_{i}"}
        if fingerprints is not None and i < len(fingerprints):
            character["fingerprint"] = encode_fingerprint(fingerprints[i])
        characters.append(character)
    return {"panels": [{"polygon": rect(b)} for b in result.get("panels", [])], "texts": texts, "characters": characters}


class MagiRunner:
    def __init__(self, spec: ModelSpec, model=None) -> None:
        self.spec = spec
        self.model = model if model is not None else self._load()
        # One forward pass at a time: the model is not safe to call from several threads.
        self.lock = asyncio.Lock()
        self._crops: list = []
        self._keep_crop_embeddings()

    def _keep_crop_embeddings(self) -> None:
        """Wraps the model's crop encoder so the embeddings it makes while grouping characters are
        kept for the result (it is called once per detection, with one entry per image)."""
        encode = getattr(self.model, "predict_crop_embeddings", None)
        if encode is None:
            return

        def keep(*args, **kwargs):
            out = encode(*args, **kwargs)
            self._crops.append(out)
            return out

        self.model.predict_crop_embeddings = keep

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
        self._crops.clear()
        with no_grad():
            [result] = self.model.predict_detections_and_associations([image])
            ocr = self.model.predict_ocr([image], [result["texts"]])[0] if result.get("texts") else []
        fingerprints = None
        if self._crops and self._crops[0] is not None and len(self._crops[0]) == 1:
            embeddings = self._crops[0][0]
            fingerprints = embeddings.float().cpu().numpy() if hasattr(embeddings, "cpu") else np.asarray(embeddings)
            if len(fingerprints) != len(result.get("characters", [])):
                fingerprints = None
        return to_contract(result, ocr, fingerprints)

    async def run(self, kind: str, payload: dict) -> object:
        if kind != "detect":
            raise ValueError(f"magi_v2 does not serve {kind!r}")
        image = await asyncio.to_thread(read_image, payload["image"])
        async with self.lock:
            return await asyncio.to_thread(self._detect, image)

    def memory(self) -> dict[int, float]:
        return {}
