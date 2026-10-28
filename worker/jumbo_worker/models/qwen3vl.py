"""Qwen3-VL-32B Instruct, the Director's model, as a replica runner through llama.cpp's server.

Requests:

- `chat` {"messages", ...sampling and request options} -> {"status", "content", "finish", "usage", "timings",
         "wall_s", "error"}: one OpenAI-style chat completion, the backend `pipeline/director/passes.py` calls.
         A refused request (e.g. a schema the grammar converter rejects) comes back with its status and error
         instead of raising, so the Director can fall back to a looser schema.

Setup from spikes 7b-7e on Kaggle's T4 x2: unsloth's Q6_K plus Qwen's F16 vision projector, both cards split by
layer (47/53), 16K context with a q8_0 KV cache and flash attention, one sequence. It loads in ~130 s and holds
~14.4 GiB on each card. vLLM is not an option on the T4s (no Turing attention backend for Qwen3-VL); on a TPU the
Director runs through vLLM instead (ADR 0004).

The engine is llama.cpp b11925 built for CUDA sm_75 (spike 7), from the jumbo-llamacpp-cuda75 dataset.
"""

from __future__ import annotations

import asyncio
import glob
import os
import shutil
import time
from pathlib import Path

from ..pool.specs import GIB, ModelSpec

TAG = "b11925"
# (repo, pinned revision, file).
LLM = ("unsloth/Qwen3-VL-32B-Instruct-GGUF", "b9262a359f54dead8e2609f6146e2fc3398fd0d9", "Qwen3-VL-32B-Instruct-Q6_K.gguf")
MMPROJ = ("Qwen/Qwen3-VL-32B-Instruct-GGUF", "e3e1fe0c76de7ee58ea65db420c643adfe2e457c", "mmproj-Qwen3VL-32B-Instruct-F16.gguf")
# Tried in order: 16K fits both cards next to the weights; 12K is the fallback if a card has less free.
CONTEXTS = (16384, 12288)
PORT = 8080
LOAD_TIMEOUT_S = 900
# Pass 1 of a long scene takes several minutes at ~6 tokens/s.
REQUEST_TIMEOUT_S = 1800


def server_binary(inputs: str = "/kaggle/input", local: str = "/tmp/jumbo-llamacpp") -> str:
    """The CUDA build: JUMBO_LLAMA_SERVER, the attached jumbo-llamacpp-cuda75 dataset, or llama-server on PATH.

    Read-only input may lose the exec bit, so the binary and its libraries are copied out once."""
    if os.environ.get("JUMBO_LLAMA_SERVER"):
        return os.environ["JUMBO_LLAMA_SERVER"]
    found = [p for p in sorted(glob.glob(f"{inputs}/**/llama-server", recursive=True)) if os.path.isfile(p)]
    if found:
        binary = Path(local) / "llama-server"
        if not binary.is_file():
            shutil.copytree(Path(found[0]).parent, local, dirs_exist_ok=True, symlinks=True)
            for f in Path(local).iterdir():
                f.chmod(0o755)
        return str(binary)
    if shutil.which("llama-server"):
        return shutil.which("llama-server")
    raise RuntimeError("llama.cpp not found: set JUMBO_LLAMA_SERVER or attach the jumbo-llamacpp-cuda75 dataset")


def server_command(binary: str, llm: str, mmproj: str, ctx: int, threads: str) -> list[str]:
    return [binary, "-m", llm, "--mmproj", mmproj, "-ngl", "999", "--split-mode", "layer", "--tensor-split", "47,53",
            "-c", str(ctx), "--flash-attn", "on", "-ctk", "q8_0", "-ctv", "q8_0", "-np", "1", "--jinja",
            "--reasoning-format", "deepseek", "--host", "127.0.0.1", "--port", str(PORT), "-t", threads, "--no-warmup"]


class Qwen3VLRunner:
    def __init__(self, spec: ModelSpec, client=None) -> None:
        self.spec = spec
        self.lock = asyncio.Lock()
        self.proc = None
        self.ctx = None
        self.client = client  # tests pass a stand-in for the server's HTTP API

    async def start(self) -> None:
        if self.client is not None:
            return
        import httpx

        binary = server_binary()
        llm, mmproj = await asyncio.to_thread(self._weights)
        env = dict(os.environ, LD_LIBRARY_PATH=str(Path(binary).parent) + ":" + os.environ.get("LD_LIBRARY_PATH", ""))
        threads = os.environ.get("OMP_NUM_THREADS", "4")
        log = open(os.environ.get("JUMBO_LLAMA_LOG", "/tmp/jumbo-llama-server.log"), "ab")
        client = httpx.AsyncClient(base_url=f"http://127.0.0.1:{PORT}", timeout=REQUEST_TIMEOUT_S)
        for ctx in CONTEXTS:
            self.proc = await asyncio.create_subprocess_exec(*server_command(binary, llm, mmproj, ctx, threads),
                                                             stdout=log, stderr=log, env=env)
            if await self._ready(client):
                self.ctx, self.client = ctx, client
                return
            await self.stop()
        await client.aclose()
        raise RuntimeError(f"llama-server did not come up at any of {CONTEXTS} context (log: {log.name})")

    async def _ready(self, client) -> bool:
        deadline = time.monotonic() + LOAD_TIMEOUT_S
        while time.monotonic() < deadline:
            if self.proc.returncode is not None:
                return False
            try:
                if (await client.get("/health", timeout=2)).status_code == 200:
                    return True
            except Exception:  # noqa: BLE001 - still loading
                pass
            await asyncio.sleep(2)
        return False

    def _weights(self) -> tuple[str, str]:
        if os.environ.get("JUMBO_WEIGHTS"):
            root = Path(os.environ["JUMBO_WEIGHTS"])
            return str(root / LLM[2]), str(root / MMPROJ[2])
        from huggingface_hub import hf_hub_download

        # /kaggle/tmp has the room; /kaggle/working would save 28 GB as notebook output.
        local = "/kaggle/tmp/qwen3vl" if os.path.isdir("/kaggle") else None
        return tuple(hf_hub_download(repo, name, revision=rev, local_dir=local) for repo, rev, name in (LLM, MMPROJ))

    async def stop(self) -> None:
        if self.proc is not None and self.proc.returncode is None:
            self.proc.terminate()
            try:
                await asyncio.wait_for(self.proc.wait(), 30)
            except asyncio.TimeoutError:
                self.proc.kill()
        self.proc = None

    async def _chat(self, p: dict) -> dict:
        t = time.monotonic()
        body = {"cache_prompt": True, **p}
        try:
            r = await self.client.post("/v1/chat/completions", json=body)
        except Exception as e:  # noqa: BLE001 - reported, the Director decides what to do
            return {"status": 0, "error": repr(e), "wall_s": round(time.monotonic() - t, 1)}
        wall = round(time.monotonic() - t, 1)
        if r.status_code != 200:
            return {"status": r.status_code, "error": r.text[:1500], "wall_s": wall}
        d = r.json()
        choice = d["choices"][0]
        return {"status": 200, "wall_s": wall, "content": choice["message"].get("content"), "finish": choice.get("finish_reason"),
                "usage": d.get("usage"), "timings": d.get("timings")}

    async def run(self, kind: str, payload: dict) -> object:
        if kind != "chat":
            raise ValueError(f"qwen3vl does not serve {kind!r}")
        async with self.lock:  # one sequence: requests take turns, which also keeps the prompt cache warm
            await self.start()
            return await self._chat(payload)

    def memory(self) -> dict[int, float]:
        """GiB llama-server holds on each leased card (NVML), so the watchdog sees the engine."""
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
