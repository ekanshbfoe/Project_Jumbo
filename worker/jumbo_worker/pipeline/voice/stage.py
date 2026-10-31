"""The `voice` job: script lines in, one WAV per line out, spoken in the project's voices.

Input blob (JSON):

    {"script": ScriptDoc,
     "voices": {"narrator": {"design": "deep, calm Indian male narrator voice",  # used once, to create it
                             "prompts": {"neutral": "<blob>", "sad": "<blob>"}}},  # voice files
     "seed": 7,                                                                  # optional
     "pacing": true, "tempo": 1.1,                                               # optional (defaults shown)
     "hearing": true,                                                            # optional (default shown)
     "delivery": "energetic"}                                                    # or "as_directed"

A voice with no prompts yet is created on first use: the model speaks the voice's first line in a voice
it invents from the description, and that take becomes the voice's neutral reference. The new prompt comes back in
the manifest for the app to keep in the project's voice library, so the voice sounds the same in
every later chapter. Emotion clips (`sad`, `tense`, ...) are added to a voice from reference audio;
until then lines fall back to neutral.

Each take is checked (pipeline/voice/takes.py) and spoken again with the next seed when it stopped early, is
silent, or is far off the length its text should take; the best of ATTEMPTS is kept and the report says so.
Silence is cut off both ends. Unless `hearing` is false, each take is then heard (the whisper model) and cut to its line, and
spoken again when words are missing or something else is said mid-line. Then, unless `pacing` is false, the pauses inside the line are shortened (takes.tighten:
only the middle of a silence is cut) and the line plays `tempo` times faster at the same pitch (takes.tempo), the
pace of the recap channels the project follows. The lines' lengths then set the panels' (buildEpisodeSpec), so
nothing is fitted to a duration.

Output manifest:

    {"chapter_id", "audio": AudioManifest, "files": {"audio/l_0001.wav": "<blob>"},
     "voices": {"narrator": {"neutral": "<blob>"}},  # voice files created by this run
     "report": {"lines": [{"line_id", "voice", "emotion_clip", "duration", "attempts", "seed", "trimmed_s", "cut_s", "heard"?, "tightened_s", "problem"?}]}}
"""

from __future__ import annotations

import asyncio
import json
import logging
import shutil
import tempfile
from pathlib import Path

import numpy as np

from ...contracts import AudioManifest, ScriptDoc
from ...contracts.rules import parse, script_problems
from ...jobs import JobContext, JobError
from ...models.wav import read_pcm, write_pcm
from . import takes
from .direction import NEUTRAL, pick_prompt, steady

log = logging.getLogger(__name__)

MODEL = "drama3"
EAR = "whisper"
PARALLEL_LINES = 8
DEFAULT_DESIGN = "calm Indian male narrator voice"
DEFAULT_SEED = 7


async def run(ctx: JobContext) -> str:
    try:
        request = json.loads(ctx.store.path(ctx.input).read_bytes())
    except (OSError, ValueError) as e:
        raise JobError(f"voice needs a JSON request blob: {e}") from e
    script = parse(ScriptDoc, request.get("script"))
    if problems := script_problems(script):
        raise JobError("script is not Devanagari or Mixed Hinglish: " + "; ".join(problems))
    chapter = script.chapter_id
    voices: dict[str, dict] = request.get("voices") or {}
    seed = int(request.get("seed", DEFAULT_SEED))
    pacing = bool(request.get("pacing", True))
    speed = float(request.get("tempo", takes.TEMPO))
    hear = bool(request.get("hearing", True))
    energetic = request.get("delivery", "energetic") == "energetic"
    store = ctx.store
    work = Path(tempfile.mkdtemp(prefix="voice-", dir=store.root))
    try:
        if hear:
            # Whisper wakes first: speech.cpp sizes Drama 3's share from the cards' free memory, so loaded after it,
            # whisper would push a card past the ceiling (model_specs.json).
            ctx.progress(0.01, "waking whisper")
            quiet = work / "quiet.wav"
            await asyncio.to_thread(write_pcm, quiet, np.zeros(16000, "<i2"), 16000)
            await ctx.call(EAR, "hear", {"audio": str(quiet), "language": "hi"})
        prompts: dict[str, dict[str, str]] = {}  # voice -> emotion -> prompt path on this host
        created: dict[str, dict[str, str]] = {}
        for voice in dict.fromkeys(_speaker(line) for line in script.lines):
            spec = voices.get(voice, {})
            missing = [h for h in (spec.get("prompts") or {}).values() if not store.has(h)]
            if missing:
                raise JobError(f"voice {voice}: prompt blobs not uploaded: {missing}")
            prompts[voice] = {emotion: str(store.path(h)) for emotion, h in (spec.get("prompts") or {}).items()}
            if NEUTRAL not in prompts[voice]:
                ctx.progress(0.02, f"creating voice {voice}")
                first = next(line for line in script.lines if _speaker(line) == voice)
                prompt = await _create_voice(ctx, work, voice, spec, first.narration)
                prompts[voice][NEUTRAL] = str(prompt)
                created.setdefault(voice, {})[NEUTRAL] = await asyncio.to_thread(store.put, prompt.read_bytes())

        # Voice files are content-addressed blobs, so two emotions sharing one recording pick the same path.
        picks: dict[str, tuple[str, str]] = {}
        for voice, files in prompts.items():
            mine = [line for line in script.lines if _speaker(line) == voice]
            chosen = [pick_prompt(files, line.emotion) or (NEUTRAL, files[NEUTRAL]) for line in mine]
            picks.update(zip((line.id for line in mine), steady(chosen, [line.emotion for line in mine])))

        rates: set[int] = set()
        files: dict[str, str] = {}
        clips: dict[str, dict] = {}
        report: dict[str, dict] = {}
        gate = asyncio.Semaphore(PARALLEL_LINES)
        done = 0

        async def speak(line) -> None:
            nonlocal done
            voice, line_id = _speaker(line), line.id
            used, prompt = picks[line_id]
            direction = line.voice
            payload = {"text": line.narration, "voice": prompt, "emotion": line.emotion,
                       "tone": direction.tone if direction else None, "pace": direction.pace if direction else None}
            if energetic:
                payload.update(_energetic(line.emotion, payload["tone"]))
            async with gate:
                take = await _best_take(ctx, work, line_id, payload, seed, hear=hear)
            tightened = 0.0
            if pacing:
                take["pcm"], tightened = await asyncio.to_thread(_pace, take["pcm"], take["rate"], speed)
                await asyncio.to_thread(write_pcm, take["path"], take["pcm"], take["rate"])
            rates.add(take["rate"])
            rel = f"audio/{line_id}.wav"
            files[rel] = await asyncio.to_thread(store.put, Path(take["path"]).read_bytes())
            duration = round(len(take["pcm"]) / take["rate"], 4)
            clips[line_id] = {"line_id": line_id, "src": f"chapters/{chapter}/{rel}", "duration": duration}
            report[line_id] = {"line_id": line_id, "voice": voice, "emotion_clip": used, "duration": duration,
                               "attempts": take["attempts"], "seed": take["seed"], "trimmed_s": take["trimmed_s"], "cut_s": take["cut_s"], "tightened_s": tightened}
            if take["heard"] is not None:
                report[line_id]["heard"] = take["heard"]
            if take["plain"]:
                report[line_id]["plain"] = True  # spoken without its cue and punctuation after every directed take failed
            if take["problem"]:
                report[line_id]["problem"] = take["problem"]
            done += 1
            ctx.progress(0.1 + 0.85 * done / len(script.lines), f"voiced {done}/{len(script.lines)} lines")

        tasks = [asyncio.ensure_future(speak(line)) for line in script.lines]
        try:
            await asyncio.gather(*tasks)
        finally:
            for task in tasks:
                task.cancel()
            await asyncio.gather(*tasks, return_exceptions=True)

        order = [line.id for line in script.lines]
        if len(rates) != 1:
            raise JobError(f"voice replicas spoke at different sample rates: {sorted(rates)}")
        audio = parse(AudioManifest, {"schema_version": 1, "chapter_id": chapter, "sample_rate": rates.pop(), "clips": [clips[i] for i in order]})
        manifest = {
            "chapter_id": chapter,
            "audio": audio.model_dump(mode="json", exclude_none=True),
            "files": files,
            "voices": created,
            "report": {"lines": [report[i] for i in order], "total_s": round(sum(c["duration"] for c in clips.values()), 2)},
        }
        ctx.progress(1.0, f"{len(order)} lines, {manifest['report']['total_s']} s of audio")
        return store.put(json.dumps(manifest).encode())
    finally:
        shutil.rmtree(work, ignore_errors=True)


def _energetic(emotion: str | None, tone: str | None) -> dict:
    """The recap delivery chosen in the voice A/B (2026-10-28): every line energetic and fast, a sad one at normal pace."""
    tone = "energetic" if tone in (None, "", "neutral") else f"energetic, {tone}"
    return {"tone": tone, "pace": "normal" if emotion == "sad" else "fast"}


def _pace(pcm, rate: int, speed: float):
    """A checked take at recap pace: pauses shortened, then sped up at the same pitch."""
    tight, removed = takes.tighten(pcm, rate)
    return takes.tempo(tight, rate, speed), removed


def _speaker(line) -> str:
    return (line.voice.speaker if line.voice else None) or "narrator"


async def _best_take(ctx: JobContext, work: Path, name: str, payload: dict, seed: int, hear: bool = False) -> dict:
    """Speaks a line until a take passes the checks (at most takes.ATTEMPTS), trimmed of its silent ends and, when `hear`,
    heard and cut to its line. When none passes, it is spoken takes.PLAIN_ATTEMPTS more times as plain text with no cue
    (takes.plain); if still none passes, the closest take is kept (most of the line heard, else nearest the text's
    expected length) and its problem reported."""
    best, tried = None, 0
    plain = {**payload, "text": takes.plain(payload["text"]), "emotion": None, "tone": None, "pace": None, "design": None}
    for attempt in range(1, takes.ATTEMPTS + takes.PLAIN_ATTEMPTS + 1):
        if attempt > takes.ATTEMPTS and not payload.get("voice"):
            break  # a voice still being invented needs its description
        spoken = payload if attempt <= takes.ATTEMPTS else plain
        tried = attempt
        raw = work / f"{name}.{attempt}.raw.wav"
        result = await ctx.call(MODEL, "speak", {**spoken, "out": str(raw), "seed": seed + attempt - 1})
        pcm, rate = await asyncio.to_thread(read_pcm, raw)
        trimmed, head, tail = takes.trim(pcm, rate)
        heard, cut, problem = None, 0.0, None
        if hear:
            await asyncio.to_thread(write_pcm, raw, trimmed, rate)
            words = (await ctx.call(EAR, "hear", {"audio": str(raw), "language": "hi"}))["words"]
            match = takes.hearing(spoken["text"], words)
            trimmed, cut = takes.cut_to_line(trimmed, rate, match)
            heard, problem = match["heard"], takes.heard_problem(match)
        problem = takes.problem(spoken["text"], trimmed, rate, result.get("stop")) or problem
        take = {"pcm": trimmed, "rate": rate, "seed": result.get("seed", seed + attempt - 1), "attempts": attempt, "plain": spoken is plain,
                "trimmed_s": round((head + tail) / rate, 3), "cut_s": cut, "heard": heard, "problem": problem}
        if best is None or take["problem"] is None or _closer(payload["text"], take, best):
            best = {**take, "attempts": attempt}
        if take["problem"] is None:
            break
        log.info("%s: take %d %s; speaking it again", name, attempt, take["problem"])
    best["attempts"] = tried
    best["path"] = str(work / f"{name}.wav")
    await asyncio.to_thread(write_pcm, best["path"], best["pcm"], best["rate"])
    return best


def _closer(text: str, take: dict, best: dict) -> bool:
    """Whether a failed take is better to keep than the best so far: more of the line heard, then nearer its length."""
    if take["heard"] is not None and best["heard"] is not None and take["heard"] != best["heard"]:
        return take["heard"] > best["heard"]
    return takes.distance(text, take["pcm"], take["rate"]) < takes.distance(text, best["pcm"], best["rate"])


async def _create_voice(ctx: JobContext, work: Path, voice: str, spec: dict, sample: str) -> Path:
    """Invents the voice from its description, then makes a voice file of that take so later lines match it."""
    prompt = work / f"{voice}.neutral.voice.gguf"
    take = await _best_take(ctx, work, f"{voice}.design", {"text": sample, "design": spec.get("design") or DEFAULT_DESIGN}, DEFAULT_SEED)
    await ctx.call(MODEL, "enroll", {"ref_audio": take["path"], "ref_text": sample, "out": str(prompt)})
    return prompt
