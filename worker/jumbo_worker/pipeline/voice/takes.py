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


# Pacing. Recap narrators breathe quickly: a pause between phrases keeps at most PHRASE_PAUSE_S and one long enough
# to be a sentence break (SENTENCE_BREAK_S or more) at most SENTENCE_PAUSE_S. Only the middle of a pause is cut, so
# no sample of speech is touched; then the take plays TEMPO times faster at the same pitch. Video test 1 (2026-10-28):
# 29% of the narration was pauses over 0.25 s, at 134 words a minute against the reference channels' ~250.
PHRASE_PAUSE_S = 0.18
SENTENCE_PAUSE_S = 0.35
SENTENCE_BREAK_S = 0.6
TIGHT_DBFS = -40.0
TEMPO = 1.1
_TIGHT_WINDOW_S = 0.01
_FADE_S = 0.005


def tighten(pcm: np.ndarray, rate: int) -> tuple[np.ndarray, float]:
    """The take with every pause between its first and last sound shortened to its cap: (pcm, seconds removed)."""
    win = max(1, int(_TIGHT_WINDOW_S * rate))
    n = len(pcm) // win
    if n < 3:
        return pcm, 0.0
    frames = pcm[: n * win].reshape(n, win).astype(np.float64) / 32768
    quiet = np.sqrt(np.mean(frames**2, axis=1)) < 10 ** (TIGHT_DBFS / 20)
    loud = np.flatnonzero(~quiet)
    if len(loud) < 2:
        return pcm, 0.0
    cuts = []  # (first sample removed, first sample kept after)
    i, last = loud[0], loud[-1]
    while i <= last:
        if not quiet[i]:
            i += 1
            continue
        j = i
        while j <= last and quiet[j]:
            j += 1
        length = (j - i) * win / rate
        cap = SENTENCE_PAUSE_S if length >= SENTENCE_BREAK_S else PHRASE_PAUSE_S
        if length > cap:
            keep = int(cap * rate / 2)
            cuts.append((i * win + keep, j * win - keep))
        i = j
    if not cuts:
        return pcm, 0.0
    fade = max(1, int(_FADE_S * rate))
    parts, start = [], 0
    for a, b in cuts:
        parts.append(pcm[start:a].astype(np.float64))
        start = b
    parts.append(pcm[start:].astype(np.float64))
    out = parts[0]
    ramp = np.linspace(0.0, 1.0, fade)
    for part in parts[1:]:
        # A 5 ms crossfade inside the silence, so the join cannot click.
        f = min(fade, len(out), len(part))
        joined = out[-f:] * ramp[::-1][-f:] + part[:f] * ramp[:f] if f else np.empty(0)
        out = np.concatenate([out[: len(out) - f], joined, part[f:]])
    removed = (len(pcm) - len(out)) / rate
    return np.clip(np.round(out), -32768, 32767).astype("<i2"), round(removed, 3)


def tempo(pcm: np.ndarray, rate: int, factor: float = TEMPO) -> np.ndarray:
    """The take `factor` times faster at the same pitch (ffmpeg's atempo, WSOLA). 1.0 returns it unchanged."""
    if abs(factor - 1.0) < 1e-6:
        return pcm
    import shutil
    import subprocess

    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        raise RuntimeError("ffmpeg is needed to speed up a take")
    raw = subprocess.run(
        [ffmpeg, "-v", "error", "-f", "s16le", "-ar", str(rate), "-ac", "1", "-i", "pipe:0", "-filter:a", f"atempo={factor}",
         "-f", "s16le", "-ar", str(rate), "-ac", "1", "pipe:1"],
        input=np.asarray(pcm, "<i2").tobytes(), capture_output=True, check=True,
    ).stdout
    return np.frombuffer(raw, "<i2").copy()
