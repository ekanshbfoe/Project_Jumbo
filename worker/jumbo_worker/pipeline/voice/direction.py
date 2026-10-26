"""How a script line's direction becomes OmniVoice settings.

OmniVoice has no emotion or pace parameter. Emotion comes from the reference clip: every voice in
the library keeps one clone prompt per emotion it has a clip for, plus `neutral`, and a line uses the
prompt for its emotion when there is one. Pace becomes the model's speed factor; a target duration
(the time a panel is on screen) overrides it, as it does in the model.
"""

from __future__ import annotations

NEUTRAL = "neutral"

PACE_SPEED = {"slow": 0.9, "normal": 1.0, "fast": 1.12}


def pick_prompt(prompts: dict[str, str], emotion: str) -> tuple[str, str] | None:
    """(emotion used, prompt) for a line: its own emotion's clip, else the voice's neutral one."""
    for key in (emotion, NEUTRAL):
        if key in prompts:
            return key, prompts[key]
    return None


def speed_for(pace: str) -> float:
    return PACE_SPEED.get(pace, 1.0)
