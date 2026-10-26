"""The `voice` job: script lines in, one WAV per line out, spoken in the project's voices.

Input blob (JSON):

    {"script": ScriptDoc,
     "voices": {"narrator": {"design": "male, young adult, low pitch",     # used once, to create it
                             "language": "hi",                             # optional language hint
                             "prompts": {"neutral": "<blob>", "sad": "<blob>"}}},
     "durations": {"l_0001": 4.5}}                                         # optional, seconds

A voice with no prompts yet is created on first use: the model speaks the voice's first line in the
designed voice, and that take becomes the voice's neutral reference. The new prompt comes back in
the manifest for the app to keep in the project's voice library, so the voice sounds the same in
every later chapter. Emotion clips (`sad`, `tense`, ...) are added to a voice from reference audio;
until then lines fall back to neutral.

Output manifest:

    {"chapter_id", "audio": AudioManifest, "files": {"audio/l_0001.wav": "<blob>"},
     "voices": {"narrator": {"neutral": "<blob>"}},  # prompts created by this run
     "report": {"lines": [{"line_id", "voice", "emotion_clip", "duration", "speed"}]}}
"""

from __future__ import annotations

import asyncio
import json
import shutil
import tempfile
from pathlib import Path

from ...contracts import AudioManifest, ScriptDoc
from ...contracts.rules import parse
from ...jobs import JobContext, JobError
from .direction import NEUTRAL, pick_prompt, speed_for

MODEL = "omnivoice"
SAMPLE_RATE = 24000
PARALLEL_LINES = 8
DEFAULT_DESIGN = "male, young adult, moderate pitch"


async def run(ctx: JobContext) -> str:
    try:
        request = json.loads(ctx.store.path(ctx.input).read_bytes())
    except (OSError, ValueError) as e:
        raise JobError(f"voice needs a JSON request blob: {e}") from e
    script = parse(ScriptDoc, request.get("script"))
    chapter = script.chapter_id
    voices: dict[str, dict] = request.get("voices") or {}
    durations: dict[str, float] = request.get("durations") or {}
    store = ctx.store
    work = Path(tempfile.mkdtemp(prefix="voice-", dir=store.root))
    try:
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

        files: dict[str, str] = {}
        clips: dict[str, dict] = {}
        report: dict[str, dict] = {}
        gate = asyncio.Semaphore(PARALLEL_LINES)
        done = 0

        async def speak(line) -> None:
            nonlocal done
            voice, line_id = _speaker(line), line.id
            used, prompt = pick_prompt(prompts[voice], line.emotion) or (NEUTRAL, prompts[voice][NEUTRAL])
            pace = (line.voice.pace if line.voice else None) or "normal"
            out = work / f"{line_id}.wav"
            payload = {"text": line.narration, "out": str(out), "voice": prompt, "speed": speed_for(pace), "duration": durations.get(line_id)}
            if voices.get(voice, {}).get("language"):
                payload["language"] = voices[voice]["language"]
            async with gate:
                result = await ctx.call(MODEL, "speak", payload)
            rel = f"audio/{line_id}.wav"
            files[rel] = await asyncio.to_thread(store.put, out.read_bytes())
            clips[line_id] = {"line_id": line_id, "src": f"chapters/{chapter}/{rel}", "duration": result["duration"]}
            report[line_id] = {"line_id": line_id, "voice": voice, "emotion_clip": used, "duration": result["duration"], "speed": payload["speed"]}
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
        audio = parse(AudioManifest, {"schema_version": 1, "chapter_id": chapter, "sample_rate": SAMPLE_RATE, "clips": [clips[i] for i in order]})
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


def _speaker(line) -> str:
    return (line.voice.speaker if line.voice else None) or "narrator"


async def _create_voice(ctx: JobContext, work: Path, voice: str, spec: dict, sample: str) -> Path:
    """Designs the voice from its description, then clones that take so later lines match it."""
    take, prompt = work / f"{voice}.design.wav", work / f"{voice}.neutral.pt"
    payload = {"text": sample, "out": str(take), "instruct": spec.get("design") or DEFAULT_DESIGN}
    if spec.get("language"):
        payload["language"] = spec["language"]
    await ctx.call(MODEL, "speak", payload)
    await ctx.call(MODEL, "enroll", {"ref_audio": str(take), "ref_text": sample, "out": str(prompt)})
    return prompt
