"""The resource manager: owns the chip and CPU leases, starts and stops replicas as the autoscaler
plans, dispatches work to them and enforces the HBM watchdog. Everything runs on the worker's event
loop; jobs use `lease(model)` or `run(model, kind, payload)`."""

from __future__ import annotations

import asyncio
import contextlib
import itertools
import logging
import time
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path
from typing import AsyncIterator, Awaitable, Callable, Protocol

from .autoscaler import Plan, ReplicaView, plan
from .chips import ChipLease, ChipLedger, LeaseError
from .cpu import CpuLease, CpuPool
from .replica import Replica, ReplicaDied, ReplicaError, replica_env
from .specs import ModelSpec, PoolConfig
from .watchdog import Watchdog
from .weights import WeightCache, compile_cache_env

log = logging.getLogger(__name__)


class Handle(Protocol):
    async def run(self, kind: str, payload: dict) -> object: ...
    async def memory(self) -> dict[int, float]: ...
    async def stop(self) -> None: ...
    async def kill(self) -> None: ...


# (spec, chip lease or None for CPU, CPU lease, staged weights folder or None) -> running replica
Spawn = Callable[[ModelSpec, "ChipLease | None", CpuLease, "Path | None"], Awaitable[Handle]]


@dataclass
class Slot:
    """One replica as the manager tracks it, from placement to exit."""

    id: str
    spec: ModelSpec
    chips: ChipLease | None
    cpu: CpuLease
    started: float
    handle: Handle | None = None
    state: str = "starting"  # starting | ready | stopping
    busy: int = 0
    last_used: float = 0.0
    throttled: bool = False
    pinned: bool = False
    task: asyncio.Task | None = field(default=None, repr=False)

    @property
    def where(self) -> str:
        return "cpu" if self.chips is None else "tpu:" + ",".join(map(str, self.chips.chips))


class ResourceManager:
    def __init__(
        self,
        config: PoolConfig,
        *,
        ledger: ChipLedger | None = None,
        cpu: CpuPool | None = None,
        spawn: Spawn | None = None,
        publish: Callable[[dict], object] | None = None,
        clock: Callable[[], float] = time.monotonic,
        sample_s: float = 1.0,
        log_dir: Path | None = None,
        extra_env: dict[str, str] | None = None,
        weights: WeightCache | None = None,
        compile_cache: Path | None = None,
    ) -> None:
        self.config = config
        self.ledger = ledger or ChipLedger(config.chips, config.chip_budget_gb)
        self.cpu = cpu or CpuPool.detect()
        self._spawn = spawn or self._spawn_process
        self._publish = publish or (lambda event: None)
        self.clock = clock
        self.sample_s = sample_s
        self.log_dir = log_dir
        self.extra_env = {**(compile_cache_env(compile_cache) if compile_cache else {}), **(extra_env or {})}
        self.weights = weights
        self.watchdog = Watchdog(config.chip_hbm_gb, config.soft_limit, config.hbm_ceiling)
        self.slots: dict[str, Slot] = {}
        self.queued: dict[str, int] = defaultdict(int)
        self.broken: dict[str, str] = {}
        self.phase: str | None = None
        self.phase_since = clock()
        self.last_plan = Plan(None)
        self._ids = itertools.count(1)
        self._cond = asyncio.Condition()
        self._wake = asyncio.Event()
        self._tasks: list[asyncio.Task] = []
        self._last_status: dict | None = None
        self._closing = False
        self.kills = 0

    # --- lifecycle ---------------------------------------------------------------------------

    def start(self) -> None:
        self._tasks = [asyncio.create_task(self._scale_loop()), asyncio.create_task(self._watch_loop())]

    async def close(self) -> None:
        # The loops also stop on this flag: on Python 3.11, asyncio.wait_for can swallow a
        # cancellation that lands as its inner wait completes, so cancelling alone may not stop them.
        self._closing = True
        self._wake.set()
        for task in self._tasks:
            task.cancel()
        await asyncio.gather(*self._tasks, return_exceptions=True)
        await asyncio.gather(*(self._retire(slot, kill=False) for slot in list(self.slots.values())), return_exceptions=True)

    def kick(self) -> None:
        self._wake.set()

    # --- work --------------------------------------------------------------------------------

    @contextlib.asynccontextmanager
    async def lease(self, model: str) -> AsyncIterator[Handle]:
        """Waits for a ready replica of `model` with spare capacity and holds one slot of it."""
        spec = self.config.spec(model)
        self.queued[model] += 1
        self.kick()
        try:
            async with self._cond:
                await self._cond.wait_for(lambda: model in self.broken or self._pick(spec) is not None)
                if model in self.broken:
                    raise ReplicaError(f"{model} cannot start: {self.broken[model]}")
                slot = self._pick(spec)
                assert slot is not None and slot.handle is not None
                slot.busy += 1
        finally:
            self.queued[model] -= 1
        try:
            yield slot.handle
        except ReplicaDied:
            # Crashed on its own: retire it so retries go elsewhere and its chips come back.
            if slot.id in self.slots and slot.state != "stopping":
                slot.state = "stopping"
                slot.task = asyncio.create_task(self._retire(slot, kill=True))
            raise
        finally:
            slot.busy -= 1
            slot.last_used = self.clock()
            async with self._cond:
                self._cond.notify_all()
            self.kick()

    async def run(self, model: str, kind: str, payload: dict, attempts: int = 3) -> object:
        """Runs one request, retrying on another replica if the one serving it dies (watchdog kill,
        crash or sleep). Model errors are not retried."""
        for attempt in range(1, attempts + 1):
            try:
                async with self.lease(model) as replica:
                    return await replica.run(kind, payload)
            except ReplicaDied:
                if attempt == attempts:
                    raise
                log.warning("%s replica died during %s; retrying (%d/%d)", model, kind, attempt, attempts)
        raise AssertionError("unreachable")

    def _pick(self, spec: ModelSpec) -> Slot | None:
        candidates = [
            s
            for s in self.slots.values()
            if s.spec.name == spec.name
            and s.state == "ready"
            and not s.throttled
            and s.busy < spec.concurrency
            and (s.chips is None or spec.phase == self.phase)
        ]
        # Chips first (faster), then the least busy replica.
        return min(candidates, key=lambda s: (s.chips is None, s.busy), default=None)

    # --- scaling -----------------------------------------------------------------------------

    async def _scale_loop(self) -> None:
        while not self._closing:
            with contextlib.suppress(asyncio.TimeoutError):
                await asyncio.wait_for(self._wake.wait(), self.config.tick_s)
            self._wake.clear()
            if self._closing:
                return
            try:
                await self.reconcile()
            except Exception:  # noqa: BLE001 - the loop must survive any single bad round
                log.exception("autoscaler round failed")

    def views(self) -> list[ReplicaView]:
        now = self.clock()
        return [
            ReplicaView(s.id, s.spec.name, s.chips, s.cpu, inflight=s.busy if s.state == "ready" else 1, idle_s=now - s.last_used if s.state == "ready" else 0.0, stopping=s.state == "stopping")
            for s in self.slots.values()
        ]

    async def reconcile(self) -> None:
        queued = {m: n for m, n in self.queued.items() if n > 0 and m not in self.broken}
        decided = plan(self.config, queued, self.views(), self.phase, self.clock() - self.phase_since, self.ledger, self.cpu)
        self.last_plan = decided
        if decided.phase != self.phase:
            log.info("phase %s -> %s: %s", self.phase, decided.phase, decided.reason)
            self._publish({"type": "pool.phase", "from": self.phase, "to": decided.phase, "reason": decided.reason})
            self.phase, self.phase_since = decided.phase, self.clock()
            async with self._cond:
                self._cond.notify_all()
        for slot_id in decided.stop:
            slot = self.slots.get(slot_id)
            if slot and slot.state == "ready" and slot.busy == 0:
                slot.state = "stopping"
                slot.task = asyncio.create_task(self._retire(slot, kill=False))
        for start in decided.start:
            self._place(self.config.spec(start.model), start.on_cpu)
        self._publish_status()

    def _place(self, spec: ModelSpec, on_cpu: bool) -> None:
        if spec.name in self.broken:
            return
        slot_id = f"{spec.name}#{next(self._ids)}"
        try:
            chips = None if on_cpu else self.ledger.lease(slot_id, spec.chips, spec.hbm_gb_per_chip)
        except LeaseError:
            return
        try:
            cpu = self.cpu.lease(slot_id, spec.cpu_cores, spec.host_ram_gb)
        except LeaseError:
            if chips:
                self.ledger.release(chips)
            return
        now = self.clock()
        slot = Slot(slot_id, spec, chips, cpu, started=now, last_used=now)
        self.slots[slot_id] = slot
        slot.task = asyncio.create_task(self._boot(slot))

    async def _boot(self, slot: Slot) -> None:
        try:
            staged = None
            if slot.spec.weights and self.weights:
                staged = await self.weights.ensure(slot.spec.weights)
                self.weights.pin(slot.spec.weights)
                slot.pinned = True
            slot.handle = await self._spawn(slot.spec, slot.chips, slot.cpu, staged)
        except Exception as e:  # noqa: BLE001 - recorded; waiters for this model fail fast
            log.error("%s failed to start: %s", slot.id, e)
            self.broken[slot.spec.name] = str(e)
            self._publish({"type": "pool.replica_failed", "replica": slot.id, "error": str(e)})
            await self._retire(slot, kill=True)
            return
        slot.state, slot.last_used = "ready", self.clock()
        self._publish({"type": "pool.replica_ready", "replica": slot.id, "where": slot.where, "wake_s": round(self.clock() - slot.started, 2)})
        async with self._cond:
            self._cond.notify_all()
        self.kick()

    async def _retire(self, slot: Slot, kill: bool) -> None:
        slot.state = "stopping"
        if slot.handle is not None:
            with contextlib.suppress(Exception):
                await (slot.handle.kill() if kill else slot.handle.stop())
        if slot.chips:
            self.ledger.release(slot.chips)
        self.cpu.release(slot.cpu)
        if slot.pinned and self.weights and slot.spec.weights:
            self.weights.unpin(slot.spec.weights)
            slot.pinned = False
        self.slots.pop(slot.id, None)
        async with self._cond:
            self._cond.notify_all()
        self.kick()

    # --- watchdog ----------------------------------------------------------------------------

    async def _watch_loop(self) -> None:
        while not self._closing:
            await asyncio.sleep(self.sample_s)
            for slot in [s for s in self.slots.values() if s.state == "ready" and s.chips]:
                try:
                    sample = await asyncio.wait_for(slot.handle.memory(), 5)
                except Exception:  # noqa: BLE001 - a dead or hung replica is handled by the scaler
                    continue
                await self.judge(slot, sample)

    async def judge(self, slot: Slot, sample: dict[int, float]) -> None:
        verdict = self.watchdog.judge(slot.id, sample, slot.spec.hbm_gb_per_chip)
        if verdict.level == "hard":
            self.kills += 1
            log.error("%s hit %.0f%% HBM on chip %s; killing it", slot.id, verdict.fraction * 100, verdict.chip)
            self._publish({"type": "pool.hbm_kill", "replica": slot.id, "chip": verdict.chip, "fraction": round(verdict.fraction, 4)})
            await self._retire(slot, kill=True)
            return
        throttled = verdict.level == "soft"
        if throttled != slot.throttled:
            slot.throttled = throttled
            self._publish({"type": "pool.hbm_soft" if throttled else "pool.hbm_ok", "replica": slot.id, "chip": verdict.chip, "fraction": round(verdict.fraction, 4)})
            if not throttled:
                async with self._cond:
                    self._cond.notify_all()

    # --- status ------------------------------------------------------------------------------

    def status(self) -> dict:
        owners = {c: s.id for s in self.slots.values() if s.chips for c in s.chips.chips}
        return {
            "phase": self.phase,
            "hbm_ceiling": self.config.hbm_ceiling,
            "chips": [
                {"chip": c["chip"], "replica": owners.get(c["chip"]), "reserved_gb": c["reserved_gb"], "peak": round(self.watchdog.peaks.get(c["chip"], 0.0), 4)}
                for c in self.ledger.snapshot()
            ],
            "cpu": self.cpu.snapshot(),
            "replicas": [{"id": s.id, "model": s.spec.name, "where": s.where, "state": s.state, "busy": s.busy, "throttled": s.throttled} for s in self.slots.values()],
            "queued": {m: n for m, n in self.queued.items() if n > 0},
            "broken": dict(self.broken),
        }

    def _publish_status(self) -> None:
        status = self.status()
        if status != self._last_status:
            self._last_status = status
            self._publish({"type": "pool.status", **status})

    # --- default spawner ---------------------------------------------------------------------

    async def _spawn_process(self, spec: ModelSpec, chips: ChipLease | None, cpu: CpuLease, weights: Path | None) -> Handle:
        extra = {**self.extra_env, **({"JUMBO_WEIGHTS": str(weights)} if weights else {})}
        replica = Replica(spec, chips, cpu, replica_env(spec, self.config, chips, cpu, extra), log_dir=self.log_dir)
        await replica.start(timeout=spec.wake_s * 3 + 60)
        return replica
