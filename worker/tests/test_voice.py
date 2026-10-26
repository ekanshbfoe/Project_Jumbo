"""OmniVoice plumbing without the model: text cleaning, WAV output, line direction, voice creation and
reuse, and the whole voice job through the pool with a stand-in model."""

import asyncio
import json
import wave
from pathlib import Path

import numpy as np

from jumbo_worker.blobs import BlobStore
from jumbo_worker.events import EventBus
from jumbo_worker.jobs import JobRunner
from jumbo_worker.models.omnivoice import OmniVoiceRunner, clean_text, write_wav
from jumbo_worker.pipeline.voice.direction import pick_prompt, speed_for
from jumbo_worker.pool.cpu import CpuPool
from jumbo_worker.pool.manager import ResourceManager
from jumbo_worker.pool.specs import ModelSpec, PoolConfig, load_specs

SCRIPT = json.loads((Path(__file__).parents[2] / "packages/schema/fixtures/script-doc.sample.json").read_text())


def test_only_trained_tags_and_phonemes_survive_cleaning():
    text = "[Laughter] Arre yaar,  [pause for effect] he plays the [B EY1 S] guitar [sigh]"
    assert clean_text(text) == "[laughter] Arre yaar, he plays the [B EY1 S] guitar [sigh]"


def test_wav_is_16_bit_mono_24k_and_reports_its_length(tmp_path):
    seconds = write_wav(tmp_path / "a.wav", np.sin(np.linspace(0, 100, 36000)) * 2.0)
    with wave.open(str(tmp_path / "a.wav")) as f:
        assert (f.getnchannels(), f.getsampwidth(), f.getframerate(), f.getnframes()) == (1, 2, 24000, 36000)
    assert seconds == 1.5


def test_a_line_uses_its_emotion_clip_else_the_neutral_one():
    prompts = {"neutral": "n.pt", "sad": "s.pt"}
    assert pick_prompt(prompts, "sad") == ("sad", "s.pt")
    assert pick_prompt(prompts, "action") == ("neutral", "n.pt")
    assert pick_prompt({}, "sad") is None
    assert speed_for("slow") < speed_for("normal") == 1.0 < speed_for("fast")


class StandInModel:
    def __init__(self):
        self.calls = []

    def generate(self, **kwargs):
        self.calls.append(kwargs)
        seconds = kwargs.get("duration") or 0.1 * len(kwargs["text"].split())
        return [np.zeros(int(24000 * seconds), np.float32)]


def test_runner_passes_voice_speed_and_a_fitted_duration(tmp_path):
    model = StandInModel()
    runner = OmniVoiceRunner(ModelSpec(name="omnivoice", backend="fake"), model=model)
    runner._prompts["n.pt"] = "PROMPT"
    out = asyncio.run(runner.run("speak", {"text": "Ek [note] do teen", "out": str(tmp_path / "x.wav"), "voice": "n.pt", "speed": 1.12, "duration": 2.0}))
    assert out == {"duration": 2.0, "sample_rate": 24000}
    call = model.calls[0]
    assert call["text"] == "Ek do teen" and call["voice_clone_prompt"] == "PROMPT" and call["speed"] == 1.12
    # Fitted to a panel: silence trimming would make the clip shorter than asked.
    assert call["duration"] == 2.0 and call["postprocess_output"] is False


def test_omnivoice_gets_its_own_env_and_fits_a_t4():
    spec = load_specs(chips=0, gpus=2).spec("omnivoice")
    assert any(r.startswith("omnivoice==") for r in spec.pip)
    assert spec.hbm_gb_per_chip <= 0.8 * 15 / 2  # two replicas per T4 under the ceiling


class FakeVoice:
    """The omnivoice replica: writes silent WAVs and prompt files, and records every request."""

    def __init__(self, log):
        self.log = log

    async def run(self, kind, payload):
        self.log.append((kind, payload))
        if kind == "enroll":
            Path(payload["out"]).write_bytes(b"PROMPT:" + Path(payload["ref_audio"]).read_bytes()[:4])
            return {"out": payload["out"]}
        seconds = payload.get("duration") or 1.25
        return {"duration": write_wav(payload["out"], np.zeros(int(24000 * seconds))), "sample_rate": 24000}

    async def memory(self):
        return {}

    async def stop(self):
        pass

    async def kill(self):
        pass


async def test_voice_job_creates_missing_voices_once_and_reuses_emotion_clips(tmp_path):
    cfg = PoolConfig.model_validate({"chips": 0, "tick_s": 0.02, "models": {"omnivoice": {"backend": "fake", "max_replicas": 2, "wake_s": 0}}})
    log = []

    async def spawn(spec, chips, cpu, weights=None, gpu=None):
        return FakeVoice(log)

    pool = ResourceManager(cfg, cpu=CpuPool(4, 16), spawn=spawn)
    pool.start()
    store = BlobStore(tmp_path / "store")
    script = json.loads(json.dumps(SCRIPT))
    lines = script["lines"]
    for line in lines:
        line.setdefault("voice", {})["speaker"] = "narrator"
    lines[-1]["voice"]["speaker"] = "jinwoo"
    lines[-1]["emotion"] = "sad"
    sad = store.put(b"SAD PROMPT")
    neutral = store.put(b"NEUTRAL PROMPT")
    request = {
        "script": script,
        "voices": {"narrator": {"design": "male, middle-aged, low pitch", "language": "hi"}, "jinwoo": {"prompts": {"neutral": neutral, "sad": sad}}},
        "durations": {lines[0]["id"]: 3.0},
    }
    job = JobRunner(store, EventBus(), pool=pool).submit("voice", store.put(json.dumps(request).encode()))
    await asyncio.wait_for(job.task, 30)
    await pool.close()
    assert job.status == "done", job.error
    manifest = json.loads(store.path(job.output).read_bytes())

    # The narrator had no prompt: designed once from its description, then cloned for every line.
    designs = [p for k, p in log if k == "speak" and "instruct" in p]
    assert len(designs) == 1 and designs[0]["instruct"] == "male, middle-aged, low pitch" and designs[0]["language"] == "hi"
    assert [k for k, _ in log].count("enroll") == 1 and set(manifest["voices"]) == {"narrator"}
    spoken = {Path(p["out"]).stem: p for k, p in log if k == "speak" and "voice" in p}
    narrator_prompts = {p["voice"] for i, p in spoken.items() if i != lines[-1]["id"]}
    assert len(narrator_prompts) == 1 and narrator_prompts.pop().endswith("narrator.neutral.pt")
    # jinwoo's sad line used his sad clip.
    assert Path(spoken[lines[-1]["id"]]["voice"]) == store.path(sad)

    audio = manifest["audio"]
    assert [c["line_id"] for c in audio["clips"]] == [line["id"] for line in lines]
    assert audio["clips"][0]["duration"] == 3.0 and audio["clips"][0]["src"] == f"chapters/{script['chapter_id']}/audio/{lines[0]['id']}.wav"
    assert set(manifest["files"]) == {f"audio/{line['id']}.wav" for line in lines}
    assert all(store.has(h) for h in manifest["files"].values())
    report = {r["line_id"]: r for r in manifest["report"]["lines"]}
    assert report[lines[-1]["id"]]["emotion_clip"] == "sad"
