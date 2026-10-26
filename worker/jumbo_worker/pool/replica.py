"""Replica processes: one model instance bound to its chip lease (or CPU cores), spoken to over
JSON lines on its stdin/stdout. Killing the process is how the watchdog and sleep free its chips."""

from __future__ import annotations

import asyncio
import contextlib
import itertools
import json
import logging
import os
import sys
import time
from pathlib import Path

from .chips import ChipLease
from .cpu import CpuLease
from .specs import ModelSpec, PoolConfig

log = logging.getLogger(__name__)

# Per-process slice shape for a lease of n chips on a v5e-8 host (2 x 4 chips).
CHIP_BOUNDS = {1: "1,1,1", 2: "1,2,1", 4: "2,2,1", 8: "2,4,1"}
TPU_PORT_BASE = 8476


class ReplicaError(Exception):
    """The replica failed a request, or died."""


class ReplicaDied(ReplicaError):
    """The process is gone (crashed, killed by the watchdog, or put to sleep): the work can be retried."""


def replica_env(
    spec: ModelSpec, config: PoolConfig, chips: ChipLease | None, cpu: CpuLease, extra: dict[str, str] | None = None, gpu: int | None = None
) -> dict[str, str]:
    """Environment that binds a replica to its lease and caps its allocator at the HBM ceiling
    (layer 2 of the ceiling; admission is layer 1 and the watchdog layer 3)."""
    env = {
        "JUMBO_REPLICA_SPEC": spec.model_dump_json(),
        "JUMBO_HBM_CEILING": f"{config.hbm_ceiling:.2f}",
        "OMP_NUM_THREADS": str(max(1, cpu.cores)),
        "MKL_NUM_THREADS": str(max(1, cpu.cores)),
    }
    if chips is None:
        # CPU replicas (and CPU fallbacks) must not open the TPU: libtpu would lock every chip.
        env.update({"JAX_PLATFORMS": "cpu", "PJRT_DEVICE": "CPU", "TPU_VISIBLE_CHIPS": ""})
        # On a GPU host an accelerator model sees exactly its one card; everything else sees none.
        env["CUDA_VISIBLE_DEVICES"] = "" if gpu is None else str(gpu)
        if gpu is not None:
            env["JUMBO_DEVICE"] = "cuda"
    else:
        env.update(
            {
                "JUMBO_CHIPS": ",".join(map(str, chips.chips)),
                "XLA_PYTHON_CLIENT_MEM_FRACTION": f"{config.hbm_ceiling:.2f}",
                "XLA_PYTHON_CLIENT_PREALLOCATE": "false",
                "JUMBO_GPU_MEMORY_UTILIZATION": f"{config.hbm_ceiling:.2f}",
                "PJRT_DEVICE": "TPU",
            }
        )
        if len(chips.chips) < config.chips:
            port = TPU_PORT_BASE + chips.chips[0]
            env.update(
                {
                    "TPU_VISIBLE_CHIPS": env["JUMBO_CHIPS"],
                    "TPU_CHIPS_PER_PROCESS_BOUNDS": CHIP_BOUNDS[len(chips.chips)],
                    "TPU_PROCESS_BOUNDS": "1,1,1",
                    "TPU_PROCESS_PORT": str(port),
                    "TPU_PROCESS_ADDRESSES": f"localhost:{port}",
                    "CLOUD_TPU_TASK_ID": "0",
                }
            )
    return {**env, **(extra or {})}


class Replica:
    """A running model replica (subprocess). `call` is safe to use concurrently."""

    _ids = itertools.count(1)

    def __init__(self, spec: ModelSpec, chips: ChipLease | None, cpu: CpuLease, env: dict[str, str], log_dir: Path | None = None) -> None:
        self.id = f"{spec.name}#{next(self._ids)}"
        self.spec = spec
        self.chips = chips
        self.cpu = cpu
        self._env = env
        self._log_dir = log_dir
        self.state = "starting"
        self.inflight = 0
        self.last_used = time.monotonic()
        self.info: dict = {}
        self._proc: asyncio.subprocess.Process | None = None
        self._pending: dict[int, asyncio.Future] = {}
        self._ready: asyncio.Future | None = None
        self._seq = itertools.count(1)
        self._reader: asyncio.Task | None = None

    @property
    def on_cpu(self) -> bool:
        return self.chips is None

    async def start(self, timeout: float) -> None:
        stderr = None
        if self._log_dir is not None:
            self._log_dir.mkdir(parents=True, exist_ok=True)
            stderr = open(self._log_dir / f"{self.id.replace('#', '-')}.log", "ab")  # noqa: SIM115 - owned by the child
        self._proc = await asyncio.create_subprocess_exec(
            sys.executable,
            "-m",
            "jumbo_worker.pool.replica_main",
            stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE,
            stderr=stderr,
            env={**os.environ, **self._env},
            limit=16 * 1024 * 1024,
        )
        if stderr is not None:
            stderr.close()
        self._ready = asyncio.get_running_loop().create_future()
        self._reader = asyncio.create_task(self._read())
        try:
            self.info = await asyncio.wait_for(self._ready, timeout)
        except BaseException:
            await self.kill()
            raise
        self.state = "ready"

    async def _read(self) -> None:
        assert self._proc and self._proc.stdout
        try:
            async for line in self._proc.stdout:
                msg = json.loads(line)
                if "ready" in msg:
                    if self._ready and not self._ready.done():
                        if msg["ready"]:
                            self._ready.set_result(msg.get("info", {}))
                        else:
                            self._ready.set_exception(ReplicaError(f"{self.id} failed to load: {msg.get('error')}"))
                    continue
                future = self._pending.pop(msg.get("id"), None)
                if future and not future.done():
                    if msg.get("ok"):
                        future.set_result(msg.get("result"))
                    else:
                        future.set_exception(ReplicaError(msg.get("error", "replica error")))
        finally:
            code = await self._proc.wait()
            self.state = "dead"
            err = ReplicaDied(f"{self.id} exited with code {code}")
            if self._ready and not self._ready.done():
                self._ready.set_exception(err)
            for future in self._pending.values():
                if not future.done():
                    future.set_exception(err)
            self._pending.clear()

    async def call(self, op: str, **payload) -> object:
        if self.state == "dead" or not self._proc or not self._proc.stdin:
            raise ReplicaDied(f"{self.id} is not running")
        seq = next(self._seq)
        future = asyncio.get_running_loop().create_future()
        self._pending[seq] = future
        self._proc.stdin.write((json.dumps({"id": seq, "op": op, **payload}) + "\n").encode())
        await self._proc.stdin.drain()
        return await future

    async def run(self, kind: str, payload: dict) -> object:
        self.inflight += 1
        try:
            return await self.call("run", kind=kind, payload=payload)
        finally:
            self.inflight -= 1
            self.last_used = time.monotonic()

    async def memory(self) -> dict[int, float]:
        """GB in use on each leased chip, as the replica's framework reports it."""
        raw = await self.call("memory")
        return {int(k): float(v) for k, v in (raw or {}).items()}

    async def stop(self, grace: float = 10.0) -> None:
        self.state = "stopping"
        if self._proc and self._proc.returncode is None:
            with contextlib.suppress(Exception):
                await asyncio.wait_for(self.call("shutdown"), grace)
            with contextlib.suppress(asyncio.TimeoutError):
                await asyncio.wait_for(self._proc.wait(), grace)
        await self.kill()

    async def kill(self) -> None:
        if self._proc and self._proc.returncode is None:
            self._proc.kill()
            await self._proc.wait()
        if self._reader:
            with contextlib.suppress(Exception):
                await self._reader
        self.state = "dead"
