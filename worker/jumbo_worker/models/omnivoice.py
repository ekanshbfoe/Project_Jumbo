"""OmniVoice (k2-fsa/OmniVoice) as a replica runner: text to speech in a cloned or designed voice.

Requests (paths are on the worker host):

- `speak`  {"text", "out", "voice"?: saved clone prompt (.pt), "instruct"?: voice design attributes,
            "language"?, "speed"?, "duration"?, "num_step"?}
           -> {"duration": seconds, "sample_rate": 24000}; writes a 16-bit mono WAV to `out`.
           With neither `voice` nor `instruct` the model picks a voice.
- `enroll` {"ref_audio", "ref_text"?, "out"} -> {"out"}: encodes a 3-10 s reference clip into a clone
           prompt saved at `out`, so a voice is cloned once and reused for every line after.

There is no emotion parameter in OmniVoice: emotion comes from the reference clip a line is cloned
from (pipeline/voice keeps one prompt per emotion per voice) and from the non-verbal tags the model
was trained on, which `clean_text` keeps while dropping any other bracketed text.

Weights: CC-BY-NC (non-commercial), code Apache-2.0. The model needs transformers >= 5.3, which
clashes with Magi, so its spec lists `pip` requirements and the pool runs it in its own env.
"""

from __future__ import annotations

import asyncio
import os
import re
import wave
from pathlib import Path

import numpy as np

from ..pool.specs import ModelSpec

MODEL_ID = "k2-fsa/OmniVoice"
# Pinned so the weights and their remote config cannot change under us.
REVISION = "c5fdb5ccb189668d56333f77ba2629f4cd7535f4"
SAMPLE_RATE = 24000

# Non-verbal sounds the model was trained on (README "Non-Verbal & Pronunciation Control").
NONVERBAL_TAGS = frozenset(
    {
        "laughter",
        "sigh",
        "confirmation-en",
        "question-en",
        "question-ah",
        "question-oh",
        "question-ei",
        "question-yi",
        "surprise-ah",
        "surprise-oh",
        "surprise-wa",
        "surprise-yo",
        "dissatisfaction-hnn",
    }
)
# [B EY1 S]: CMU phonemes that override an English word's pronunciation.
_PHONEMES = re.compile(r"^[A-Z]{1,3}[0-2]?( [A-Z]{1,3}[0-2]?)*$")
_BRACKETS = re.compile(r"\[([^\[\]]*)\]")


def clean_text(text: str) -> str:
    """Keeps supported non-verbal tags and CMU phoneme overrides; drops any other [bracketed] text
    (a script note the model would read aloud) and collapses the spacing it leaves."""

    def keep(m: re.Match) -> str:
        inner = m.group(1).strip()
        return f"[{inner.lower()}]" if inner.lower() in NONVERBAL_TAGS else (f"[{inner}]" if _PHONEMES.match(inner) else "")

    return re.sub(r"\s{2,}", " ", _BRACKETS.sub(keep, text)).strip()


def write_wav(path: str | Path, audio: np.ndarray, sample_rate: int = SAMPLE_RATE) -> float:
    """16-bit mono WAV; returns the duration in seconds."""
    pcm = (np.clip(np.asarray(audio, dtype=np.float32), -1.0, 1.0) * 32767).astype("<i2")
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(path), "wb") as f:
        f.setnchannels(1)
        f.setsampwidth(2)
        f.setframerate(sample_rate)
        f.writeframes(pcm.tobytes())
    return len(pcm) / sample_rate


class OmniVoiceRunner:
    def __init__(self, spec: ModelSpec, model=None) -> None:
        self.spec = spec
        self.model = model if model is not None else self._load()
        self.lock = asyncio.Lock()
        self._prompts: dict[str, object] = {}

    @staticmethod
    def _load():
        import torch
        from huggingface_hub import snapshot_download
        from omnivoice import OmniVoice

        torch.set_num_threads(max(1, int(os.environ.get("OMP_NUM_THREADS", "1"))))
        path = os.environ.get("JUMBO_WEIGHTS") or snapshot_download(MODEL_ID, revision=REVISION)
        if os.environ.get("JUMBO_DEVICE") == "cuda" and torch.cuda.is_available():
            # One card per replica (CUDA_VISIBLE_DEVICES); the allocator is capped at the ceiling's share.
            torch.cuda.set_per_process_memory_fraction(float(os.environ.get("JUMBO_GPU_MEMORY_FRACTION", "0.8")), 0)
            return OmniVoice.from_pretrained(path, device_map="cuda:0", dtype=torch.float16, asr_device="cpu")
        # torch-xla support is unproven (S0 spike 3); until then TPU hosts run it on CPU.
        return OmniVoice.from_pretrained(path, device_map="cpu", dtype=torch.float32, asr_device="cpu")

    def _prompt(self, path: str):
        if path not in self._prompts:
            from omnivoice import VoiceClonePrompt

            self._prompts[path] = VoiceClonePrompt.load(path)
        return self._prompts[path]

    def _speak(self, payload: dict) -> dict:
        kwargs: dict = {"text": clean_text(payload["text"])}
        if payload.get("voice"):
            kwargs["voice_clone_prompt"] = self._prompt(payload["voice"])
        elif payload.get("instruct"):
            kwargs["instruct"] = payload["instruct"]
        for key in ("language", "speed", "duration", "num_step"):
            if payload.get(key) is not None:
                kwargs[key] = payload[key]
        if payload.get("duration") is not None:
            # Fitted to a panel: trimming silence would undercut it and the default 0.1 s padding on
            # each side would overshoot it (spike 3: 4.0 s asked, 4.2 s returned).
            kwargs["postprocess_output"] = False
            kwargs["pad_duration"] = 0.0
        [audio] = self.model.generate(**kwargs)
        return {"duration": round(write_wav(payload["out"], audio), 4), "sample_rate": SAMPLE_RATE}

    def _enroll(self, payload: dict) -> dict:
        prompt = self.model.create_voice_clone_prompt(ref_audio=payload["ref_audio"], ref_text=payload.get("ref_text"))
        Path(payload["out"]).parent.mkdir(parents=True, exist_ok=True)
        prompt.save(payload["out"])
        self._prompts.pop(payload["out"], None)
        return {"out": payload["out"]}

    async def run(self, kind: str, payload: dict) -> object:
        handlers = {"speak": self._speak, "enroll": self._enroll}
        if kind not in handlers:
            raise ValueError(f"omnivoice does not serve {kind!r}")
        async with self.lock:
            return await asyncio.to_thread(handlers[kind], payload)

    def memory(self) -> dict[int, float]:
        return {}
