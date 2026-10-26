"""16-bit mono WAV files, as every voice runner writes them."""

from __future__ import annotations

import wave
from pathlib import Path

import numpy as np


def write_wav(path: str | Path, audio: np.ndarray, sample_rate: int) -> float:
    """Writes float samples in [-1, 1] as 16-bit mono; returns the duration in seconds."""
    pcm = (np.clip(np.asarray(audio, dtype=np.float32), -1.0, 1.0) * 32767).astype("<i2")
    return write_pcm(path, pcm, sample_rate)


def write_pcm(path: str | Path, pcm: np.ndarray, sample_rate: int) -> float:
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(path), "wb") as f:
        f.setnchannels(1)
        f.setsampwidth(2)
        f.setframerate(sample_rate)
        f.writeframes(np.asarray(pcm, "<i2").tobytes())
    return len(pcm) / sample_rate


def read_pcm(path: str | Path) -> tuple[np.ndarray, int]:
    with wave.open(str(path)) as f:
        return np.frombuffer(f.readframes(f.getnframes()), "<i2"), f.getframerate()
