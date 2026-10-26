"""Drama 3 plumbing without the model: cues, the speech.cpp worker protocol, take checks and silence trimming,
voice creation and reuse, and the whole voice job through the pool with a stand-in model."""

import asyncio
import base64
import json
import os
from pathlib import Path

import numpy as np

from jumbo_worker.blobs import BlobStore
from jumbo_worker.events import EventBus
from jumbo_worker.jobs import JobRunner
from jumbo_worker.models.drama3 import Drama3Runner, cue, speech_binary
from jumbo_worker.models.wav import read_pcm, write_pcm
from jumbo_worker.pipeline.voice import takes
from jumbo_worker.pipeline.voice.direction import pick_prompt
from jumbo_worker.pool.cpu import CpuPool
from jumbo_worker.pool.manager import ResourceManager
from jumbo_worker.pool.specs import ModelSpec, PoolConfig, load_specs

SCRIPT = json.loads((Path(__file__).parents[2] / "packages/schema/fixtures/script-doc.sample.json").read_text())
RATE = 44100


def speech(seconds: float, rate: int = RATE, lead: float = 0.0, trail: float = 0.0) -> np.ndarray:
    """A tone standing in for speech, with digital silence before and after."""
    t = np.arange(int(seconds * rate)) / rate
    tone = (np.sin(2 * np.pi * 220 * t) * 8000).astype("<i2")
    return np.concatenate([np.zeros(int(lead * rate), "<i2"), tone, np.zeros(int(trail * rate), "<i2")])


def test_direction_becomes_one_cue():
    assert cue("sad", "whispered", "slow") == "[sad, whispered, slowly] "
    assert cue("calm", "neutral", "normal") == "[calm] "
    assert cue(None, None, None) == ""
    assert cue(design="[deep, calm Indian male narrator voice]") == "[deep, calm Indian male narrator voice] "


def test_a_line_uses_its_emotion_recording_else_the_neutral_one():
    files = {"neutral": "n.voice.gguf", "sad": "s.voice.gguf"}
    assert pick_prompt(files, "sad") == ("sad", "s.voice.gguf")
    assert pick_prompt(files, "action") == ("neutral", "n.voice.gguf")
    assert pick_prompt({}, "sad") is None


def test_trim_cuts_only_digital_silence():
    pcm = speech(2.0, lead=0.5, trail=0.8)
    out, head, tail = takes.trim(pcm, RATE)
    keep = int(takes.KEEP_S * RATE)
    loud = np.flatnonzero(np.abs(pcm.astype(np.int32)) > 32768 * 10 ** (takes.TRIM_DBFS / 20))
    assert head == loud[0] - keep and tail == len(pcm) - loud[-1] - 1 - keep
    # Every sample from the first sound to the last survives, untouched.
    assert np.array_equal(out[keep : keep + loud[-1] - loud[0] + 1], pcm[loud[0] : loud[-1] + 1])


def test_take_checks():
    text = "सियोल की रात हमेशा शांत लगती थी, लेकिन आज कुछ अलग था।"  # ~4.6 s at the measured rate
    assert takes.problem(text, speech(4.8), RATE, "complete") is None
    assert takes.problem(text, speech(4.8), RATE, "max_seconds") == "stopped early (max_seconds)"
    assert takes.problem(text, np.zeros(RATE * 4, "<i2"), RATE, "complete") == "silent"
    assert takes.problem(text, speech(0.9), RATE, "complete").startswith("too short")
    assert takes.problem(text, speech(14.0), RATE, "complete").startswith("too long")
    # Cues and speaker tags are not spoken, so they do not lengthen the expectation.
    assert takes.expected_seconds("[sighs] <|speaker:0|> " + text) == takes.expected_seconds(text)


class FakeWorker:
    """speech.cpp's `worker` on the other end of the pipes: answers JSON lines with base64 PCM chunks."""

    def __init__(self, seconds_by_seed=None):
        self.sent = []
        self.out: asyncio.Queue = asyncio.Queue()
        self.seconds_by_seed = seconds_by_seed or {}
        self.pid = None
        worker = self

        class Stdin:
            def write(self, data):
                worker.answer(json.loads(data))

            async def drain(self):
                pass

        class Stdout:
            async def readline(self):
                return (json.dumps(await worker.out.get()) + "\n").encode()

        self.stdin, self.stdout = Stdin(), Stdout()

    def answer(self, msg):
        self.sent.append(msg)
        if msg["type"] == "add_voice":
            self.out.put_nowait({"type": "end", "id": msg["id"]})
            return
        pcm = speech(self.seconds_by_seed.get(msg.get("seed"), 1.0), trail=0.3).tobytes()
        self.out.put_nowait({"type": "progress", "id": msg["id"], "done": 0.5})
        for i in range(0, len(pcm), 32768):
            self.out.put_nowait({"type": "chunk", "id": msg["id"], "seq": i, "pcm": base64.b64encode(pcm[i : i + 32768]).decode()})
        self.out.put_nowait({"type": "end", "id": msg["id"], "seed": msg.get("seed", 1), "samples": len(pcm) // 2, "stop": "complete"})


async def test_runner_speaks_the_worker_protocol(tmp_path):
    worker = FakeWorker({7: 2.0})
    runner = Drama3Runner(ModelSpec(name="drama3", backend="fake"), worker=worker)
    payload = {"text": "जिनवू ने तलवार उठाई।", "out": str(tmp_path / "a.wav"), "voice": "/v/jinwoo.voice.gguf", "emotion": "sad", "pace": "slow", "seed": 7}
    result = await runner.run("speak", payload)
    assert result == {"duration": 2.3, "sample_rate": RATE, "stop": "complete", "seed": 7}
    pcm, rate = read_pcm(tmp_path / "a.wav")
    assert rate == RATE and np.array_equal(pcm, speech(2.0, trail=0.3))
    # The voice file is added once and reused; direction is one cue in front of the words.
    await runner.run("speak", {**payload, "seed": 8})
    assert [m["type"] for m in worker.sent] == ["add_voice", "synthesize", "synthesize"]
    assert worker.sent[1]["text"] == "[sad, slowly] जिनवू ने तलवार उठाई।" and worker.sent[1]["voice"] == "v0"
    # Without a voice file the model invents one from the description.
    await runner.run("speak", {"text": "नमस्ते।", "out": str(tmp_path / "b.wav"), "design": "deep male narrator"})
    assert worker.sent[-1]["voice"] == "none" and worker.sent[-1]["text"].startswith("[deep male narrator] ")


def test_engine_is_copied_out_of_read_only_dataset_input(tmp_path):
    unpacked = tmp_path / "input" / "jumbo-speechcpp-cuda75"
    unpacked.mkdir(parents=True)
    (unpacked / "speech").write_text("#!/bin/sh\n")
    (unpacked / "libspeech.so.3").write_text("lib")
    (unpacked / "speech").chmod(0o444)
    binary = speech_binary(str(tmp_path / "input"), str(tmp_path / "run"))
    assert binary == str(tmp_path / "run" / "speech") and os.access(binary, os.X_OK)
    assert (tmp_path / "run" / "libspeech.so.3").is_file()


def test_drama3_fits_a_t4_under_the_ceiling():
    spec = load_specs(chips=0, gpus=2).spec("drama3")
    assert spec.hbm_gb_per_chip <= 0.8 * 15 and spec.options["quant"] in ("q4_k", "q5_k", "q6_k", "q8_0")
    assert not spec.pip


class FakeVoice:
    """The drama3 replica: writes tone WAVs and voice files, records every request. Seeds in `bad` give a
    take that stalls (too short), so the job must speak those lines again."""

    def __init__(self, log, bad=()):
        self.log, self.bad = log, set(bad)

    async def run(self, kind, payload):
        self.log.append((kind, payload))
        if kind == "enroll":
            Path(payload["out"]).write_bytes(b"VOICE:" + Path(payload["ref_audio"]).read_bytes()[44:48])
            return {"out": payload["out"]}
        seconds = 0.2 if payload["seed"] in self.bad else takes.expected_seconds(payload["text"])
        duration = write_pcm(payload["out"], speech(seconds, lead=0.2, trail=0.5), RATE)
        return {"duration": duration, "sample_rate": RATE, "stop": "complete", "seed": payload["seed"]}

    async def memory(self):
        return {}

    async def stop(self):
        pass

    async def kill(self):
        pass


async def test_voice_job_creates_voices_once_retries_bad_takes_and_trims(tmp_path):
    cfg = PoolConfig.model_validate({"chips": 0, "tick_s": 0.02, "models": {"drama3": {"backend": "fake", "max_replicas": 2, "wake_s": 0}}})
    log = []

    async def spawn(spec, chips, cpu, weights=None, gpu=None):
        return FakeVoice(log, bad={7})

    pool = ResourceManager(cfg, cpu=CpuPool(4, 16), spawn=spawn)
    pool.start()
    store = BlobStore(tmp_path / "store")
    script = json.loads(json.dumps(SCRIPT))
    lines = script["lines"]
    for line in lines:
        line.setdefault("voice", {})["speaker"] = "narrator"
    lines[-1]["voice"]["speaker"] = "jinwoo"
    lines[-1]["emotion"] = "sad"
    sad = store.put(b"SAD VOICE")
    neutral = store.put(b"NEUTRAL VOICE")
    request = {"script": script, "voices": {"narrator": {"design": "deep, calm Indian male narrator voice"}, "jinwoo": {"prompts": {"neutral": neutral, "sad": sad}}}}
    job = JobRunner(store, EventBus(), pool=pool).submit("voice", store.put(json.dumps(request).encode()))
    await asyncio.wait_for(job.task, 30)
    await pool.close()
    assert job.status == "done", job.error
    manifest = json.loads(store.path(job.output).read_bytes())

    # The narrator had no voice file: invented once from its description, then cloned for every line.
    designs = [p for k, p in log if k == "speak" and p.get("design")]
    assert {p["design"] for p in designs} == {"deep, calm Indian male narrator voice"}
    assert [k for k, _ in log].count("enroll") == 1 and set(manifest["voices"]) == {"narrator"}
    spoken = [p for k, p in log if k == "speak" and p.get("voice")]
    narrator = {Path(p["voice"]).name for p in spoken if not Path(p["out"]).name.startswith(lines[-1]["id"])}
    assert narrator == {"narrator.neutral.voice.gguf"}
    # jinwoo's sad line used his sad recording, with the emotion passed on for the cue.
    last = [p for p in spoken if Path(p["out"]).name.startswith(lines[-1]["id"])]
    assert Path(last[0]["voice"]) == store.path(sad) and last[0]["emotion"] == "sad"

    # Seed 7 stalls, so every line took a second take with seed 8, which passed.
    report = {r["line_id"]: r for r in manifest["report"]["lines"]}
    assert all(r["attempts"] == 2 and r["seed"] == 8 and "problem" not in r for r in report.values())
    # Silence was cut off both ends (0.2 s and 0.5 s, less 40 ms kept on each side); the manifest says 44.1 kHz.
    assert all(abs(r["trimmed_s"] - 0.62) < 0.01 for r in report.values())
    audio = manifest["audio"]
    assert audio["sample_rate"] == RATE
    assert [c["line_id"] for c in audio["clips"]] == [line["id"] for line in lines]
    first = audio["clips"][0]
    assert first["src"] == f"chapters/{script['chapter_id']}/audio/{lines[0]['id']}.wav"
    assert abs(first["duration"] - (takes.expected_seconds(lines[0]["narration"]) + 2 * takes.KEEP_S)) < 0.01
    assert set(manifest["files"]) == {f"audio/{line['id']}.wav" for line in lines}
    assert all(store.has(h) for h in manifest["files"].values())


async def test_a_line_that_never_passes_keeps_its_closest_take(tmp_path):
    from jumbo_worker.pipeline.voice.stage import _best_take

    class Ctx:
        calls = 0

        async def call(self, model, kind, payload):
            self.calls += 1
            seconds = {1: 0.3, 2: 60.0, 3: 0.6}[self.calls]
            return {"duration": write_pcm(payload["out"], speech(seconds), RATE), "sample_rate": RATE, "stop": "complete", "seed": payload["seed"]}

    ctx = Ctx()
    take = await _best_take(ctx, tmp_path, "l_0001", {"text": "सियोल की रात हमेशा शांत लगती थी, लेकिन आज कुछ अलग था।"}, 7)
    assert ctx.calls == takes.ATTEMPTS and take["attempts"] == 3
    # 0.6 s is closest to the ~4.6 s the text should take (0.3 s and 60 s are further, on a log scale).
    assert take["seed"] == 9 and take["problem"].startswith("too short")
    assert read_pcm(take["path"])[0].size == len(take["pcm"])


async def test_voice_job_refuses_hindi_in_latin_letters(tmp_path):
    store = BlobStore(tmp_path / "store")
    script = json.loads(json.dumps(SCRIPT))
    script["lines"][0]["narration"] = "Seoul ki raat hamesha shaant lagti thi."
    job = JobRunner(store, EventBus()).submit("voice", store.put(json.dumps({"script": script}).encode()))
    await asyncio.wait_for(job.task, 10)
    assert job.status == "failed" and "lines.0.narration: narration has Hindi in Latin letters (ki, thi)" in job.error
