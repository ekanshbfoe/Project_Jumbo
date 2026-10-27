"""Fish Audio Drama 3 (35B MoE, 3B active) as a replica runner, through speech.cpp's worker protocol.

Requests (paths are on the worker host):

- `speak`  {"text", "out", "voice"?: a .voice.gguf, "design"?: words describing a voice to invent,
            "emotion"?, "tone"?, "pace"?, "seed"?}
           -> {"duration", "sample_rate", "stop", "seed"}; writes a 16-bit mono WAV to `out`.
           Without `voice` the model chooses one, steered by `design`.
- `enroll` {"ref_audio", "ref_text", "out"} -> {"out"}: makes a voice file from a recording and its words,
           so a voice is cloned once and reused for every line after.

There is no speed or duration control: the panel waits for the audio (packages/schema buildEpisodeSpec).
Emotion, tone and pace become one bracketed cue in front of the line, which is how Drama 3 takes direction.

The engine is speech.cpp built for CUDA (spike 4/5, with the dual-GPU and memory-cap patch); the weights are
sakasegawa/drama-3-GGUF, checked stage by stage against Fish Audio's implementation. Experts that do not fit
the leased card(s) run on the CPU.
"""

from __future__ import annotations

import asyncio
import base64
import glob
import json
import os
import shutil
from pathlib import Path

import numpy as np

from ..pool.specs import GIB, ModelSpec
from .wav import write_pcm

REPO = "sakasegawa/drama-3-GGUF"
# Pinned so the files cannot change under us.
REVISION = "9a01354d04ab6eb521bc9cdfc56ab6824c474af2"
FILES = {
    "q4_k": "Drama-35B-A3B-v3-Q4_K.gguf",
    "q5_k": "Drama-35B-A3B-v3-Q5_K.gguf",
    "q6_k": "Drama-35B-A3B-v3-Q6_K.gguf",
    "q8_0": "Drama-35B-A3B-v3-Q8_0.gguf",
}
# Room under the allocator cap for what speech.cpp's own estimate of its buffers leaves out.
GPU_MARGIN = 0.03

# ScriptDoc emotions as Drama 3 cues.
EMOTION_CUES = {
    "calm": "calm",
    "tense": "tense",
    "action": "urgent, intense",
    "shock": "shocked",
    "sad": "sad",
    "comedic": "playful",
    "romantic": "soft, tender",
    "mysterious": "mysterious, low voice",
    "triumphant": "triumphant",
}
PACE_CUES = {"slow": "slowly", "fast": "quickly"}


def cue(emotion: str | None = None, tone: str | None = None, pace: str | None = None, design: str | None = None) -> str:
    """One bracketed direction for the start of a line, e.g. "[sad, whispered, slowly] "; "" when neutral."""
    parts = [design, EMOTION_CUES.get(emotion or ""), None if tone in (None, "", "neutral") else tone, PACE_CUES.get(pace or "")]
    words = [p.strip().strip("[]") for p in parts if p and p.strip()]
    return f"[{', '.join(dict.fromkeys(words))}] " if words else ""


def speech_binary(inputs: str = "/kaggle/input", local: str = "/tmp/jumbo-speechcpp") -> str:
    """The CUDA build: JUMBO_SPEECH_BIN, the attached jumbo-speechcpp-cuda75 dataset, or `speech` on PATH.

    Kaggle unpacks the dataset's archive into read-only input, where the exec bit may be lost, so the
    binary and its libraries are copied out once and run from there."""
    if os.environ.get("JUMBO_SPEECH_BIN"):
        return os.environ["JUMBO_SPEECH_BIN"]
    found = [p for p in sorted(glob.glob(f"{inputs}/**/speech", recursive=True)) if os.path.isfile(p)]
    if found:
        binary = Path(local) / "speech"
        if not binary.is_file():
            shutil.copytree(Path(found[0]).parent, local, dirs_exist_ok=True)
            binary.chmod(0o755)
        return str(binary)
    if shutil.which("speech"):
        return shutil.which("speech")
    raise RuntimeError("speech.cpp not found: set JUMBO_SPEECH_BIN or attach the jumbo-speechcpp-cuda75 dataset")


class Drama3Runner:
    def __init__(self, spec: ModelSpec, worker=None) -> None:
        self.spec = spec
        self.quant = spec.options.get("quant", "q6_k")
        self.lock = asyncio.Lock()
        self.voices: dict[str, str] = {}  # voice file path -> name inside the worker
        self.sample_rate = 44100
        self._ids = 0
        self.proc = worker  # tests pass a stand-in with stdin/stdout

    async def start(self) -> None:
        if self.proc is not None:
            return
        self.binary = speech_binary()
        self.model = self._weights()
        env = dict(os.environ, SPEECH_NO_UPDATE_CHECK="1")
        env["LD_LIBRARY_PATH"] = str(Path(self.binary).parent) + ":" + env.get("LD_LIBRARY_PATH", "")
        fraction = float(os.environ.get("JUMBO_GPU_MEMORY_FRACTION", "0.8"))
        env["SPEECH_GPU_FRACTION"] = f"{max(0.1, fraction - GPU_MARGIN):.2f}"
        # The replica sees its leased card(s) as CUDA0 (and CUDA1 when the spec splits over two).
        cards = len([c for c in os.environ.get("CUDA_VISIBLE_DEVICES", "").split(",") if c])
        device = "CUDA0" if cards else "cpu"
        if cards > 1:
            env["SPEECH_DRAMA3_SECOND_DEVICE"] = "CUDA1"
        threads = os.environ.get("OMP_NUM_THREADS", "1")
        self.proc = await asyncio.create_subprocess_exec(
            self.binary, "worker", self.model, "--device", device, "--threads", threads,
            stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE, env=env, limit=64 << 20,
        )
        ready = await self._read()
        if ready.get("type") != "ready":
            raise RuntimeError(f"speech.cpp worker did not start: {ready}")
        self.sample_rate = int(ready.get("model", {}).get("sample_rate", self.sample_rate))

    def _weights(self) -> str:
        if os.environ.get("JUMBO_WEIGHTS"):
            return os.environ["JUMBO_WEIGHTS"]
        from huggingface_hub import hf_hub_download

        # Kaggle's /kaggle/tmp has the room (1.1 TB); /kaggle/working would save 30 GB as notebook output.
        local = "/kaggle/tmp/drama3" if os.path.isdir("/kaggle") else None
        return hf_hub_download(REPO, FILES[self.quant], revision=REVISION, local_dir=local)

    async def _read(self) -> dict:
        line = await self.proc.stdout.readline()
        if not line:
            raise RuntimeError("speech.cpp worker exited")
        return json.loads(line)

    async def _request(self, msg: dict) -> tuple[dict, list[bytes]]:
        """Sends one request and collects its audio chunks until its terminal message."""
        self._ids += 1
        msg["id"] = rid = f"r{self._ids}"
        self.proc.stdin.write((json.dumps(msg, ensure_ascii=False) + "\n").encode())
        await self.proc.stdin.drain()
        chunks: list[bytes] = []
        while True:
            out = await self._read()
            if out.get("id") is None and out.get("type") == "error":
                raise RuntimeError(f"speech.cpp refused {msg['type']}: {out.get('error')}")
            if out.get("id") != rid:
                continue
            kind = out.get("type")
            if kind == "chunk":
                chunks.append(base64.b64decode(out["pcm"]))
            elif kind in ("end", "error", "cancelled"):
                if kind != "end":
                    raise RuntimeError(f"speech.cpp {msg['type']}: {out.get('error') or kind}")
                return out, chunks

    async def _voice(self, path: str) -> str:
        if path not in self.voices:
            name = f"v{len(self.voices)}"
            await self._request({"type": "add_voice", "name": name, "path": path})
            self.voices[path] = name
        return self.voices[path]

    async def _speak(self, p: dict) -> dict:
        voice = await self._voice(p["voice"]) if p.get("voice") else "none"
        text = cue(p.get("emotion"), p.get("tone"), p.get("pace"), None if p.get("voice") else p.get("design")) + p["text"]
        msg = {"type": "synthesize", "text": text, "voice": voice}
        if p.get("seed") is not None:
            msg["seed"] = int(p["seed"])
        end, chunks = await self._request(msg)
        # The worker's samples are written as they come: no re-quantization.
        seconds = write_pcm(p["out"], np.frombuffer(b"".join(chunks), "<i2"), self.sample_rate)
        return {"duration": round(seconds, 4), "sample_rate": self.sample_rate, "stop": end.get("stop"), "seed": end.get("seed")}

    async def _enroll(self, p: dict) -> dict:
        Path(p["out"]).parent.mkdir(parents=True, exist_ok=True)
        # The codec encodes on the CPU, which speech.cpp says matches the official encoder most closely.
        proc = await asyncio.create_subprocess_exec(
            self.binary, "voice", self.model, p["ref_audio"], "--transcript", p["ref_text"], p["out"],
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.STDOUT,
        )
        out, _ = await proc.communicate()
        if proc.returncode:
            raise RuntimeError("speech voice failed: " + out.decode(errors="replace")[-800:])
        self.voices.pop(p["out"], None)
        return {"out": p["out"]}

    async def run(self, kind: str, payload: dict) -> object:
        handlers = {"speak": self._speak, "enroll": self._enroll}
        if kind not in handlers:
            raise ValueError(f"drama3 does not serve {kind!r}")
        async with self.lock:  # the worker speaks one request at a time
            await self.start()
            return await handlers[kind](payload)

    def memory(self) -> dict[int, float]:
        """GiB the speech.cpp process holds on each leased card (NVML), so the watchdog sees the engine."""
        if self.proc is None or not getattr(self.proc, "pid", None):
            return {}
        cards = [int(c) for c in os.environ.get("CUDA_VISIBLE_DEVICES", "").split(",") if c]
        try:
            import pynvml

            pynvml.nvmlInit()
            out = {}
            for card in cards:
                handle = pynvml.nvmlDeviceGetHandleByIndex(card)
                used = sum(p.usedGpuMemory or 0 for p in pynvml.nvmlDeviceGetComputeRunningProcesses(handle) if p.pid == self.proc.pid)
                out[card] = used / GIB
            return out
        except Exception:  # noqa: BLE001 - NVML missing or not permitted in this container
            return {}
