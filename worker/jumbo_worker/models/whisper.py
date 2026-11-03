"""Whisper large-v3 (faster-whisper, CTranslate2) as a replica runner: hears a spoken take, so the voice job can check
it says its line (pipeline/voice/takes.py hearing).

Requests (paths are on the worker host):

- `hear` {"audio": a 16-bit mono WAV, "language"?: "hi"} -> {"words": [[start, end, word, probability], ...]}

On a T4 it runs int8_float16 (2.0 GiB beside Drama 3, measured 2026-10-31), 0.4 s per second of audio with word times
and beam 5; on a CPU host int8. The audio is passed as samples, not a path: faster-whisper's own decoding goes through
PyAV, whose Kaggle build refused its arguments in the reel prep.
"""

from __future__ import annotations

import asyncio
import os

import numpy as np

from ..pool.specs import ModelSpec
from .wav import read_pcm

MODEL = "large-v3"
RATE = 16000


def to_16k(pcm: np.ndarray, rate: int) -> np.ndarray:
    """Float samples at Whisper's 16 kHz."""
    x = pcm.astype(np.float32) / 32768
    if rate == RATE or len(x) == 0:
        return x
    at = np.arange(0, len(x), rate / RATE)
    return np.interp(at, np.arange(len(x)), x).astype(np.float32)


class WhisperRunner:
    def __init__(self, spec: ModelSpec, model=None) -> None:
        self.spec = spec
        self.lock = asyncio.Lock()
        self.model = model  # tests pass a stand-in with transcribe()

    def _load(self):
        from faster_whisper import WhisperModel

        threads = int(os.environ.get("OMP_NUM_THREADS", "1"))
        if os.environ.get("JUMBO_DEVICE") == "cuda":
            return WhisperModel(MODEL, device="cuda", device_index=0, compute_type="int8_float16", cpu_threads=threads)
        return WhisperModel(MODEL, device="cpu", compute_type="int8", cpu_threads=threads)

    def _hear(self, p: dict) -> dict:
        if self.model is None:
            self.model = self._load()
        pcm, rate = read_pcm(p["audio"])
        segments, _ = self.model.transcribe(
            to_16k(pcm, rate), language=p.get("language", "hi"), beam_size=5, word_timestamps=True, vad_filter=False,
            condition_on_previous_text=False,
        )
        words = [[round(w.start, 2), round(w.end, 2), w.word, round(w.probability, 3)] for s in segments for w in (s.words or [])]
        return {"words": words}

    async def run(self, kind: str, payload: dict) -> object:
        if kind != "hear":
            raise ValueError(f"whisper does not serve {kind!r}")
        async with self.lock:
            return await asyncio.to_thread(self._hear, payload)

    def memory(self) -> dict[int, float]:
        return {}
