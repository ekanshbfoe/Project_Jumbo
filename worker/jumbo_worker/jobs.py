"""Idempotent jobs over blobs. Each job's id is derived from its inputs, so a client that lost its
connection can resubmit and get the same job (and its result) back instead of duplicate work."""

from __future__ import annotations

import asyncio
import json
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, AsyncContextManager, Awaitable, Callable

import blake3

from .blobs import BlobStore, check_hash
from .events import EventBus

if TYPE_CHECKING:
    from .pool.manager import Handle, ResourceManager


class JobError(Exception):
    """A client error: unknown kind or missing input."""


@dataclass
class JobContext:
    job_id: str
    input: str
    params: dict
    store: BlobStore
    _report: Callable[[float, str], None]
    pool: ResourceManager | None = None

    def progress(self, fraction: float, message: str = "") -> None:
        self._report(fraction, message)

    def lease(self, model: str) -> AsyncContextManager[Handle]:
        """Holds a replica of `model` (waiting for the pool to place one) for several requests."""
        return self._pool().lease(model)

    async def call(self, model: str, kind: str, payload: dict) -> object:
        """One request to `model`, retried on another replica if the serving one dies."""
        return await self._pool().run(model, kind, payload)

    def _pool(self) -> ResourceManager:
        if self.pool is None:
            raise JobError("this worker has no model pool")
        return self.pool


JobHandler = Callable[[JobContext], Awaitable[str]]


@dataclass
class Job:
    id: str
    kind: str
    input: str
    params: dict
    status: str = "queued"
    progress: float = 0.0
    output: str | None = None
    error: str | None = None
    task: asyncio.Task | None = field(default=None, repr=False)

    def public(self) -> dict:
        return {k: getattr(self, k) for k in ("id", "kind", "input", "status", "progress", "output", "error")}


async def echo(ctx: JobContext) -> str:
    """Bridge self-test: reads the input blob, reports progress, and returns the same bytes."""
    data = ctx.store.path(ctx.input).read_bytes()
    steps = 5
    for i in range(steps):
        await asyncio.sleep(float(ctx.params.get("step_s", 0.05)))
        ctx.progress((i + 1) / steps, f"echo step {i + 1}/{steps}")
    return ctx.store.put(data)


def default_handlers() -> dict[str, JobHandler]:
    """Every job kind the worker serves. Stages import lazily so the bridge core stays light."""
    from .pipeline.vision import stage as vision

    return {"echo": echo, "vision": vision.run}


class JobRunner:
    def __init__(self, store: BlobStore, bus: EventBus, handlers: dict[str, JobHandler] | None = None, pool: ResourceManager | None = None) -> None:
        self.store = store
        self.pool = pool
        self.bus = bus
        self.handlers = handlers or default_handlers()
        self.jobs: dict[str, Job] = {}

    @staticmethod
    def job_id(kind: str, input_hash: str, params: dict) -> str:
        canonical = json.dumps([kind, input_hash, params], sort_keys=True, separators=(",", ":"))
        return blake3.blake3(canonical.encode()).hexdigest()[:32]

    def submit(self, kind: str, input_hash: str, params: dict | None = None) -> Job:
        params = params or {}
        if kind not in self.handlers:
            raise JobError(f"unknown job kind {kind!r}")
        check_hash(input_hash)
        if not self.store.has(input_hash):
            raise JobError(f"input blob {input_hash} has not been uploaded")
        job_id = self.job_id(kind, input_hash, params)
        existing = self.jobs.get(job_id)
        if existing and existing.status != "failed":
            return existing
        job = Job(job_id, kind, input_hash, params)
        self.jobs[job_id] = job
        job.task = asyncio.get_running_loop().create_task(self._run(job))
        return job

    async def _run(self, job: Job) -> None:
        def report(fraction: float, message: str) -> None:
            job.progress = max(0.0, min(1.0, fraction))
            self.bus.publish({"type": "job.progress", "job_id": job.id, "progress": job.progress, "message": message})

        job.status = "running"
        self.bus.publish({"type": "job.started", "job_id": job.id, "kind": job.kind})
        try:
            ctx = JobContext(job.id, job.input, job.params, self.store, report, self.pool)
            job.output = await self.handlers[job.kind](ctx)
            job.status, job.progress = "done", 1.0
            self.bus.publish({"type": "job.done", "job_id": job.id, "output": job.output})
        except Exception as e:  # noqa: BLE001 - any handler failure is reported to the client
            job.status, job.error = "failed", f"{type(e).__name__}: {e}"
            self.bus.publish({"type": "job.failed", "job_id": job.id, "error": job.error})

    def get(self, job_id: str) -> Job | None:
        return self.jobs.get(job_id)

    async def drain(self) -> None:
        """Waits for every running job to finish."""
        tasks = [j.task for j in self.jobs.values() if j.task and not j.task.done()]
        if tasks:
            await asyncio.gather(*tasks, return_exceptions=True)
