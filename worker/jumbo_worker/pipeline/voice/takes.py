"""Checking a spoken take, so no one has to fix lines or panel timings by hand.

Drama 3 has no duration control: the panel holds as long as its line (buildEpisodeSpec). What can still go
wrong is the take itself (a stall, silence, words skipped or a line that runs on), so each take is judged and
spoken again with another seed when it fails (or stalls into a long silence mid-line). Nothing is time-stretched or re-encoded: the only edit is
cutting digital silence off both ends, which removes no sample of speech.

Then the take is heard (Whisper, models/whisper.py) and compared with its text: what was said before the line's first
word or after its last is cut off, and a take that leaves words out or says something else mid-line is spoken again.
Live test 2026-10-30: with a voice cloned from English reels, Drama 3 sometimes reads the end of the voice's own
reference transcript before the line (up to 19 s of "Now the creepy part, some cookies..."), at a length the
checks above let through.
"""

from __future__ import annotations

import re
from difflib import SequenceMatcher

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
# A line that fails every take is spoken PLAIN_ATTEMPTS more times as plain text with no cue. Voice A/B 2026-10-28: with
# the new fast voice, a line opening "तभी, एक नया दृश्य: “तीन साल पहले...”" under "[mysterious, low voice, energetic,
# quickly]" stopped after about 3 s of its 12 in all three takes.
PLAIN_ATTEMPTS = 2


def plain(text: str) -> str:
    """The line without what can derail a take: quote marks, dashes and ellipses (each a comma), and no cues."""
    text = _CUE.sub("", text)
    text = re.sub(r"[\"“”‘’]|(?<![A-Za-z])'|'(?![A-Za-z])", "", text)
    text = re.sub(r"\s*(?:—|–|―|-{2,}|…|\.{2,})\s*", ", ", text)
    text = re.sub(r"\s*,(\s*,)+", ",", text)
    return re.sub(r"\s+", " ", re.sub(r",\s*([।?!])", r"\1", text)).strip(" ,")


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


# Hearing. Whisper's words and the line's text meet on a consonant skeleton, so Devanagari and Latin spellings of one
# word (interview / इंटरव्यू, lift / लिफ़्ट) match and vowel signs Whisper hears differently do not count. Calibrated
# on 110 clips of ch_0001 (old and new voices) and 36 test takes with large-v3: the clean lines heard 0.89-1.0 of
# their text (median 0.95) and nothing extra inside it; the old voice's worst glitch (l_0009) heard 0.67.
MIN_HEARD = 0.8
MAX_EXTRA = 0.2
# Matching runs shorter than this many skeleton letters are chance.
_MIN_RUN = 3
# Whisper's word times are about this loose: a cut is made at the quietest 10 ms within this far before the line's
# first heard word (or after its last), so it falls in the breath between what was read out and the line.
HEAR_MARGIN_S = 0.5

_DEVA = {
    "क": "k", "ख": "k", "ग": "g", "घ": "g", "ङ": "n", "च": "c", "छ": "c", "ज": "j", "झ": "j", "ञ": "n",
    "ट": "t", "ठ": "t", "ड": "d", "ढ": "d", "ण": "n", "त": "t", "थ": "t", "द": "d", "ध": "d", "न": "n",
    "प": "p", "फ": "f", "ब": "b", "भ": "b", "म": "m", "य": "y", "र": "r", "ल": "l", "व": "v", "श": "s",
    "ष": "s", "स": "s", "ह": "h", "ं": "n", "ँ": "n",
}
# Nukta letters: क़ ख़ ग़ ज़ ड़ ढ़ फ़ य़, written as letter + nukta or precomposed.
_NUKTA = {"क": "k", "ख": "k", "ग": "g", "ज": "j", "ड": "d", "ढ": "d", "फ": "f", "य": "y"}
_PRECOMPOSED = dict(zip("\u0958\u0959\u095a\u095b\u095c\u095d\u095e\u095f", "kkgjddfy"))
_LATIN = [("ph", "f"), ("sh", "s"), ("ch", "c"), ("th", "t"), ("dh", "d"), ("kh", "k"), ("gh", "g"), ("bh", "b"), ("ck", "k"),
          ("q", "k"), ("x", "ks"), ("w", "v"), ("z", "j"), ("c", "k")]


def skeleton(word: str) -> str:
    """A word's consonants as rough Latin classes, doubled letters once: "interview" and "इंटरव्यू" are both "ntrvy"."""
    w = word.lower()
    for a, b in _LATIN:
        w = w.replace(a, b)
    out: list[str] = []
    for i, ch in enumerate(w):
        if ch in _PRECOMPOSED:
            out.append(_PRECOMPOSED[ch])
        elif ch in _DEVA:
            out.append(_NUKTA[ch] if ch in _NUKTA and w[i + 1 : i + 2] == "\u093c" else _DEVA[ch])
        elif "a" <= ch <= "z" and ch not in "aeiou":
            out.append(ch)
    return re.sub(r"(.)\1+", r"\1", "".join(out))


def spoken_words(text: str) -> list[str]:
    return [w for w in re.split(r"[^\w\u0900-\u097F]+", _CUE.sub("", text)) if w]


def hearing(text: str, words: list[list]) -> dict:
    """How a take's heard words (Whisper's [start, end, word, ...]) match its text: the share of the text heard, the
    share of what was heard that is not in the text, and the time of the text's first and last word (None when none
    of it was heard)."""
    want = "".join(skeleton(w) for w in spoken_words(text))
    pieces = [skeleton(str(w[2])) for w in words]
    owner = [i for i, piece in enumerate(pieces) for _ in piece]
    got = "".join(pieces)
    if not want or not got:
        return {"heard": 0.0, "extra": 1.0 if got else 0.0, "start": None, "end": None}
    runs = [b for b in SequenceMatcher(None, want, got, autojunk=False).get_matching_blocks() if b.size >= _MIN_RUN]
    used = sorted({owner[b.b + k] for b in runs for k in range(b.size)})
    if not used:
        return {"heard": 0.0, "extra": 1.0, "start": None, "end": None}
    first, last = used[0], used[-1]
    inside = "".join(pieces[first : last + 1])
    matched = sum(b.size for b in runs)
    return {"heard": round(matched / len(want), 3), "extra": round(max(0.0, 1 - matched / len(inside)), 3),
            "start": float(words[first][0]), "end": float(words[last][1])}


def heard_problem(match: dict) -> str | None:
    """Why a heard take must be spoken again, or None when it says its line."""
    if match["start"] is None:
        return "the line was not heard"
    if match["heard"] < MIN_HEARD:
        return f"words missing ({match['heard']:.0%} of the line heard)"
    if match["extra"] > MAX_EXTRA:
        return f"says something else mid-line ({match['extra']:.0%} of it is not in the line)"
    return None


def _quietest(pcm: np.ndarray, rate: int, lo: float, hi: float) -> int:
    """The sample at the middle of the quietest 10 ms window between lo and hi seconds."""
    win = max(1, int(_TIGHT_WINDOW_S * rate))
    a, b = max(0, int(lo * rate)), min(len(pcm), int(hi * rate))
    n = (b - a) // win
    if n < 1:
        return min(max(a, 0), len(pcm))
    frames = pcm[a : a + n * win].reshape(n, win).astype(np.float64)
    return a + int(np.argmin(np.mean(frames**2, axis=1))) * win + win // 2


def cut_to_line(pcm: np.ndarray, rate: int, match: dict) -> tuple[np.ndarray, float]:
    """The take from the pause before its line's first heard word to the pause after its last: (pcm, seconds cut)."""
    if match["start"] is None:
        return pcm, 0.0
    seconds = len(pcm) / rate
    start = _quietest(pcm, rate, match["start"] - HEAR_MARGIN_S, match["start"]) if match["start"] > HEAR_MARGIN_S else 0
    end = _quietest(pcm, rate, match["end"], match["end"] + HEAR_MARGIN_S) if match["end"] < seconds - HEAR_MARGIN_S else len(pcm)
    if start == 0 and end == len(pcm):
        return pcm, 0.0
    out = pcm[start:end].astype(np.float64)
    fade = min(len(out), max(1, int(_FADE_S * rate)))
    if start:
        out[:fade] *= np.linspace(0.0, 1.0, fade)
    if end < len(pcm):
        out[-fade:] *= np.linspace(1.0, 0.0, fade)
    return np.clip(np.round(out), -32768, 32767).astype("<i2"), round((len(pcm) - (end - start)) / rate, 3)
