"""How a script line's direction reaches the voice.

Two sources, used together: the reference clip (every voice in the library keeps one voice file per emotion
it has a recording for, plus `neutral`, and a line uses its emotion's file when there is one) and the cue the
runner writes in front of the line from its emotion, tone and pace (models/drama3.py).
"""

from __future__ import annotations

NEUTRAL = "neutral"


def pick_prompt(prompts: dict[str, str], emotion: str) -> tuple[str, str] | None:
    """(emotion used, voice file) for a line: its own emotion's recording, else the voice's neutral one."""
    for key in (emotion, NEUTRAL):
        if key in prompts:
            return key, prompts[key]
    return None


def steady(picks: list[tuple[str, str]], emotions: list[str]) -> list[tuple[str, str]]:
    """The lines' (emotion used, voice file) with one-line switches removed: a line whose file differs from both
    neighbours, which agree, takes theirs, so the narrator does not change voice for one line and back. Its own
    cue still directs it. A sad line keeps its file: a lone sad moment is worth the switch."""
    out = list(picks)
    for i in range(1, len(picks) - 1):
        before, after = picks[i - 1], picks[i + 1]
        if before[1] == after[1] != picks[i][1] and emotions[i] != "sad":
            out[i] = before
    return out
