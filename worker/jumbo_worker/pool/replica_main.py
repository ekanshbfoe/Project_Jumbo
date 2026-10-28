"""Entry point of a replica process (`python -m jumbo_worker.pool.replica_main`).

Reads its ModelSpec and lease from the environment set by `replica_env`, loads the model, prints
`{"ready": true}`, then serves JSON-line requests on stdin. The original stdout is kept for the
protocol and fd 1 is pointed at stderr, so library logging cannot corrupt it.
"""

from __future__ import annotations

import asyncio
import json
import os
import sys
import traceback
from typing import Callable, Protocol

from .specs import GIB, ModelSpec


class Runner(Protocol):
    async def run(self, kind: str, payload: dict) -> object: ...

    def memory(self) -> dict[int, float]: ...


def leased_chips() -> list[int]:
    raw = os.environ.get("JUMBO_CHIPS", "")
    return [int(c) for c in raw.split(",") if c]


def leased_gpus() -> list[int]:
    raw = os.environ.get("JUMBO_GPU", "")
    return [int(c) for c in raw.split(",") if c]


# CUDA context and kernels a process holds outside torch's allocator, when NVML cannot say.
CUDA_CONTEXT_GB = 0.4


def gpu_memory_gb(cards: list[int]) -> dict[int, float]:
    """GiB this process uses on each of its cards: NVML's per-process figure (includes the CUDA
    context), or torch's reserved memory plus a context estimate."""
    try:
        import pynvml

        pynvml.nvmlInit()
        out = {}
        for card in cards:
            handle = pynvml.nvmlDeviceGetHandleByIndex(card)
            used = [p.usedGpuMemory for p in pynvml.nvmlDeviceGetComputeRunningProcesses(handle) if p.pid == os.getpid() and p.usedGpuMemory]
            if used:
                out[card] = used[0] / GIB
        if out:
            return out
    except Exception:  # noqa: BLE001 - NVML missing or not permitted in this container
        pass
    if "torch" in sys.modules:
        import torch

        if torch.cuda.is_available() and torch.cuda.is_initialized():
            # Inside the replica its cards are cuda:0, cuda:1... in lease order.
            return {card: torch.cuda.memory_reserved(i) / GIB + CUDA_CONTEXT_GB for i, card in enumerate(cards)}
    return {}


def device_memory_gb() -> dict[int, float]:
    """GB in use per leased chip, or on the leased GPU card, from whichever framework this replica loaded."""
    gpus = leased_gpus()
    if gpus:
        return gpu_memory_gb(gpus)
    chips = leased_chips()
    if "jax" in sys.modules:
        import jax

        return {chips[i] if i < len(chips) else d.id: d.memory_stats().get("bytes_in_use", 0) / 1e9 for i, d in enumerate(jax.local_devices())}
    if "torch_xla" in sys.modules:
        import torch_xla.core.xla_model as xm

        out = {}
        for i, device in enumerate(xm.get_xla_supported_devices()):
            info = xm.get_memory_info(device)
            out[chips[i] if i < len(chips) else i] = info.get("bytes_used", 0) / 1e9
        return out
    return {}


class FakeRunner:
    """A model that declares its footprint instead of loading weights: for tests and simulations.

    Payload: `sleep_s` (work time), `grow_gb` (HBM it adds while running, to trigger the watchdog),
    `fail` (raise), `print` (written to stdout, as noisy libraries do), `echo` (returned as the result).
    """

    def __init__(self, spec: ModelSpec) -> None:
        self.spec = spec
        self.chips = leased_chips()
        self.extra = 0.0

    async def run(self, kind: str, payload: dict) -> object:
        if payload.get("print"):
            print(payload["print"], flush=True)
        if payload.get("fail"):
            raise RuntimeError(str(payload["fail"]))
        self.extra += float(payload.get("grow_gb", 0))
        try:
            await asyncio.sleep(float(payload.get("sleep_s", 0)))
        finally:
            if not payload.get("keep"):
                self.extra -= float(payload.get("grow_gb", 0))
        return {"kind": kind, "echo": payload.get("echo"), "pid": os.getpid(), "chips": self.chips}

    def memory(self) -> dict[int, float]:
        places = self.chips or leased_gpus()
        return {c: self.spec.hbm_gb_per_chip + self.extra for c in places}


def _not_yet(backend: str) -> Callable[[ModelSpec], Runner]:
    def make(spec: ModelSpec) -> Runner:
        raise NotImplementedError(f"{spec.name}: the {backend} runner is added with its pipeline stage")

    return make


# Real backends register here as their stages land (vision in Step 5, director/librarian in Step 6, voice/render in Step 7).
RUNNERS: dict[str, Callable[[ModelSpec], Runner]] = {
    "fake": FakeRunner,
    "vllm-tpu": _not_yet("vllm-tpu"),
    "jax": _not_yet("jax"),
    "torch-xla": _not_yet("torch-xla"),
    "ggml": _not_yet("ggml"),
    "cpu": _not_yet("cpu"),
    "node": _not_yet("node"),
}


def _magi(spec: ModelSpec) -> Runner:
    from ..models.magi import MagiRunner

    return MagiRunner(spec)


# Runners for specific models, imported only inside the replica that serves them.
def _drama3(spec: ModelSpec) -> Runner:
    from ..models.drama3 import Drama3Runner

    return Drama3Runner(spec)


def _qwen3vl(spec: ModelSpec) -> Runner:
    from ..models.qwen3vl import Qwen3VLRunner

    return Qwen3VLRunner(spec)


MODEL_RUNNERS: dict[str, Callable[[ModelSpec], Runner]] = {"magi_v2": _magi, "drama3": _drama3, "qwen3vl_32b": _qwen3vl}


def make_runner(spec: ModelSpec) -> Runner:
    return (MODEL_RUNNERS.get(spec.name) or RUNNERS[spec.backend])(spec)


async def serve(out) -> None:
    def send(msg: dict) -> None:
        out.write(json.dumps(msg) + "\n")
        out.flush()

    try:
        spec = ModelSpec.model_validate_json(os.environ["JUMBO_REPLICA_SPEC"])
        runner = make_runner(spec)
    except Exception as e:  # noqa: BLE001 - reported to the parent, which gives up on this replica
        # The full traceback goes to the replica log; the parent gets its tail, which usually names the cause.
        trace = traceback.format_exc()
        print(trace, file=sys.stderr, flush=True)
        tail = " | ".join(line.strip() for line in trace.strip().splitlines()[-6:-1])
        send({"ready": False, "error": f"{type(e).__name__}: {e} (at {tail})"[:2000]})
        return
    send({"ready": True, "info": {"pid": os.getpid(), "model": spec.name, "chips": leased_chips()}})

    loop = asyncio.get_running_loop()
    reader = asyncio.StreamReader(limit=16 * 1024 * 1024)
    await loop.connect_read_pipe(lambda: asyncio.StreamReaderProtocol(reader), sys.stdin)
    tasks: set[asyncio.Task] = set()

    async def handle(msg: dict) -> None:
        try:
            if msg["op"] == "run":
                result = await runner.run(msg["kind"], msg.get("payload", {}))
            elif msg["op"] == "memory":
                result = runner.memory() or device_memory_gb()
            else:
                raise ValueError(f"unknown op {msg['op']!r}")
            send({"id": msg["id"], "ok": True, "result": result})
        except Exception as e:  # noqa: BLE001 - the parent decides whether to retry
            send({"id": msg["id"], "ok": False, "error": f"{type(e).__name__}: {e}"})

    while line := await reader.readline():
        msg = json.loads(line)
        if msg.get("op") == "shutdown":
            for task in tasks:
                task.cancel()
            send({"id": msg["id"], "ok": True, "result": None})
            return
        task = asyncio.create_task(handle(msg))
        tasks.add(task)
        task.add_done_callback(tasks.discard)


def main() -> None:
    out = os.fdopen(os.dup(1), "w", encoding="utf-8")
    os.dup2(2, 1)
    asyncio.run(serve(out))


if __name__ == "__main__":
    main()
