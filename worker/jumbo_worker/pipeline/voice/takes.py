"""Checking a spoken take, so no one has to fix lines or panel timings by hand.

Drama 3 has no duration control: the panel holds as long as its line (buildEpisodeSpec). What can still go
wrong is the take itself (a stall, silence, words skipped or a line that runs on), so each take is judged and
spoken again with another seed when it fails (or stalls into a long silence mid-line). Nothing is time-stretched or re-encoded: the only edit is
cutting digital silence off both ends, which removes no sample of speech.
"""

from __future__ import annotations

import re

import numpy as np

# Devanagari and Mixed Hinglish narration in Drama 3's native-reference voices speaks about 8.5-10 of
# spoken_chars() a second (spike 4, all 22 takes).
CHARS_PER_SECOND = 9.0
# A take this far off the text's expected length skipped words or ran on.
SHORT, LONG = 0.4, 2.5
# Seconds of slack for very short lines, whose breaths and pauses weigh more.
SLACK_S = 0.6
# Below this RMS (dBFS) a take is silence, whatever its length.
SILENT_DBFS = -45.0
# Trimming: a sample under -60 dBFS is digital silence; keep 40 ms of it before and after the speech.
TRIM_DBFS = -60.0
KEEP_S = 0.04
ATTEMPTS = 3
# A pause inside a take longer than this is a stall, not acting: the longest in spikes 4 and 5's good takes
# was 2.45 s (a dramatic beat after [sighs]); spike 5's 4-bit run stalled for 7.9 s mid-line at a
# normal-looking length.
MAX_PAUSE_S = 3.5
PAUSE_DBFS = -45.0
_WINDOW_S = 0.05

_CUE = re.compile(r"\[[^\]]*\]|<\|[^|]*\|>")


def spoken_chars(text: str) -> int:
    """Characters that are spoken: no cues, speaker tags, spaces or punctuation."""
    return sum(1 for ch in _CUE.sub("", text) if ch.isalnum() or "ऀ" <= ch <= "ॿ")


def expected_seconds(text: str) -> float:
    return spoken_chars(text) / CHARS_PER_SECOND


def dbfs(pcm: np.ndarray) -> float:
    if len(pcm) == 0:
        return -120.0
    rms = float(np.sqrt(np.mean((pcm.astype(np.float64) / 32768) ** 2)))
    return 20 * np.log10(max(rms, 1e-6))


def trim(pcm: np.ndarray, rate: int) -> tuple[np.ndarray, int, int]:
    """The take without its silent ends: (pcm, samples cut at the start, samples cut at the end)."""
    floor = 32768 * 10 ** (TRIM_DBFS / 20)
    loud = np.flatnonzero(np.abs(pcm.astype(np.int32)) > floor)
    if len(loud) == 0:
        return pcm, 0, 0
    keep = int(KEEP_S * rate)
    start, end = max(0, loud[0] - keep), min(len(pcm), loud[-1] + 1 + keep)
    return pcm[start:end], int(start), int(len(pcm) - end)


def longest_pause(pcm: np.ndarray, rate: int) -> float:
    """Seconds of the longest run of quiet 50 ms windows between the first and last sound."""
    win = max(1, int(_WINDOW_S * rate))
    n = len(pcm) // win
    if n == 0:
        return 0.0
    frames = pcm[: n * win].reshape(n, win).astype(np.float64) / 32768
    quiet = np.sqrt(np.mean(frames**2, axis=1)) < 10 ** (PAUSE_DBFS / 20)
    loud = np.flatnonzero(~quiet)
    if len(loud) < 2:
        return 0.0
    inner, longest, run = quiet[loud[0] : loud[-1] + 1], 0, 0
    for q in inner:
        run = run + 1 if q else 0
        longest = max(longest, run)
    return longest * win / rate


def problem(text: str, pcm: np.ndarray, rate: int, stop: str | None) -> str | None:
    """Why a take must be spoken again, or None when it is fine."""
    if stop not in (None, "complete"):
        return f"stopped early ({stop})"
    if dbfs(pcm) < SILENT_DBFS:
        return "silent"
    seconds, want = len(pcm) / rate, expected_seconds(text)
    if seconds < SHORT * want - SLACK_S:
        return f"too short ({seconds:.1f} s for ~{want:.1f} s of text)"
    if seconds > LONG * want + SLACK_S:
        return f"too long ({seconds:.1f} s for ~{want:.1f} s of text)"
    if (pause := longest_pause(pcm, rate)) > MAX_PAUSE_S:
        return f"stalls ({pause:.1f} s of silence mid-line)"
    return None


def distance(text: str, pcm: np.ndarray, rate: int) -> float:
    """How far a take's length is from the text's, to pick the best of several failed takes."""
    return abs(np.log(max(len(pcm) / rate, 0.05) / max(expected_seconds(text), 0.05)))
