"""The manager under load, with in-process fake replicas that declare their HBM footprint."""

import asyncio
import random

import pytest

from jumbo_worker.pool.cpu import CpuPool
from jumbo_worker.pool.manager import ResourceManager
from jumbo_worker.pool.replica import ReplicaDied, ReplicaError
from jumbo_worker.pool.specs import PoolConfig


def config(ceiling: float) -> PoolConfig:
    budget = 16 * ceiling
    fast = {"backend": "fake", "idle_ttl_s": 0.2, "wake_s": 0.05}
    return PoolConfig.model_validate(
        {
            "hbm_ceiling": ceiling,
            "phase_hysteresis_s": 0.05,
            "phase_min_dwell_s": 0.1,
            "tick_s": 0.02,
            "models": {
                "magi": {**fast, "chips": 1, "hbm_gb_per_chip": 5, "cpu_fallback": True, "max_replicas": 8, "concurrency": 2, "job_s": 0.01, "phase": "vision"},
                "voice": {**fast, "chips": 1, "hbm_gb_per_chip": 6, "cpu_fallback": True, "max_replicas": 8, "job_s": 0.02, "phase": "vision", "priority": 4},
                # The Director fills each chip right up to the budget, whatever the ceiling is.
                "qwen": {**fast, "chips": 8, "hbm_gb_per_chip": budget - 0.3, "cpu_cores": 8, "host_ram_gb": 90, "max_replicas": 1, "concurrency": 16, "job_s": 0.05, "wake_s": 0.1, "phase": "director", "priority": 2},
                "librarian": {**fast, "chips": 2, "hbm_gb_per_chip": 10, "cpu_cores": 4, "host_ram_gb": 20, "max_replicas": 4, "concurrency": 4, "job_s": 0.03, "phase": "librarian", "priority": 3},
                "render": {**fast, "cpu_cores": 4, "host_ram_gb": 4, "max_replicas": 4, "target_queue_per_replica": 1, "job_s": 0.03, "priority": 5},
            },
        }
    )


class FakeHandle:
    def __init__(self, spec, chips, spike_gb: float = 0.0) -> None:
        self.spec, self.chips, self.spike_gb = spec, chips, spike_gb
        self.extra = 0.0
        self.dead = False

    async def run(self, kind, payload):
        if self.dead:
            raise ReplicaDied("gone")
        self.extra += payload.get("grow_gb", 0.0)
        try:
            await asyncio.sleep(payload.get("sleep_s", 0.0))
        finally:
            self.extra -= payload.get("grow_gb", 0.0)
        if self.dead:
            raise ReplicaDied("killed")
        return {"model": self.spec.name, "chips": self.chips.chips if self.chips else None}

    async def memory(self):
        return {c: self.spec.hbm_gb_per_chip + self.extra for c in (self.chips.chips if self.chips else ())}

    async def stop(self):
        self.dead = True

    async def kill(self):
        self.dead = True


def fake_spawn(handles: list):
    async def spawn(spec, chips, cpu):
        await asyncio.sleep(spec.wake_s)
        handle = FakeHandle(spec, chips)
        handles.append(handle)
        return handle

    return spawn


@pytest.mark.parametrize("ceiling", [0.80, 0.90])
async def test_random_load_never_breaks_the_ceiling(ceiling):
    cfg = config(ceiling)
    events: list[dict] = []
    handles: list[FakeHandle] = []
    manager = ResourceManager(cfg, cpu=CpuPool(cores=32, ram_gb=300), spawn=fake_spawn(handles), publish=events.append, sample_s=0.005)
    manager.start()
    violations: list[str] = []
    done = asyncio.Event()

    async def check():
        while not done.is_set():
            reserved = manager.ledger.reserved_gb()
            if any(gb > cfg.chip_budget_gb + 1e-9 for gb in reserved):
                violations.append(f"reserved {reserved}")
            owners: dict[int, str] = {}
            for slot in manager.slots.values():
                for chip in slot.chips.chips if slot.chips else ():
                    if chip in owners:
                        violations.append(f"chip {chip} held by {owners[chip]} and {slot.id}")
                    owners[chip] = slot.id
            used_cores, used_ram = manager.cpu.used()
            if used_cores > manager.cpu.cores or used_ram > manager.cpu.ram_gb:
                violations.append("cpu oversubscribed")
            await asyncio.sleep(0.002)

    rng = random.Random(int(ceiling * 100))
    weights = {"magi": 40, "voice": 25, "librarian": 15, "qwen": 8, "render": 12}

    async def job(i: int):
        await asyncio.sleep(rng.uniform(0, 1.5))
        model = rng.choices(list(weights), list(weights.values()))[0]
        result = await manager.run(model, "work", {"sleep_s": rng.uniform(0.005, 0.05)})
        assert result["model"] == model
        return model

    checker = asyncio.create_task(check())
    models = await asyncio.wait_for(asyncio.gather(*(job(i) for i in range(300))), 60)
    done.set()
    await checker
    await manager.close()

    assert violations == []
    assert len(models) == 300 and set(models) == set(weights)
    # Every phase got the chips at some point, and Qwen's replica took all 8.
    phases = {e["to"] for e in events if e["type"] == "pool.phase"}
    assert {"vision", "director", "librarian"} <= phases
    assert any(h.spec.name == "qwen" and len(h.chips.chips) == 8 for h in handles)
    assert manager.watchdog.peak() <= ceiling and manager.kills == 0
    assert manager.ledger.free() == list(range(8)) and manager.cpu.used() == (0, 0)


async def test_hard_ceiling_kills_the_replica_and_retries_the_job():
    cfg = config(0.80)
    events: list[dict] = []
    handles: list[FakeHandle] = []
    spawn = fake_spawn(handles)

    async def spawn_leaky_first(spec, chips, cpu):
        handle = await spawn(spec, chips, cpu)
        if len(handles) == 1:
            # The first replica balloons to 13 GB (81%) while it works; replacements behave.
            handle.extra = 8.0
        return handle

    manager = ResourceManager(cfg, cpu=CpuPool(cores=32, ram_gb=300), spawn=spawn_leaky_first, publish=events.append, sample_s=0.005)
    manager.start()
    result = await asyncio.wait_for(manager.run("magi", "work", {"sleep_s": 0.3}), 5)
    await manager.close()
    kill = next(e for e in events if e["type"] == "pool.hbm_kill")
    assert kill["replica"] == "magi#1" and kill["fraction"] > 0.80
    assert manager.kills == 1 and len(handles) == 2 and handles[0].dead
    assert result["model"] == "magi"


async def test_soft_limit_throttles_until_memory_drops():
    cfg = PoolConfig.model_validate({**config(0.80).model_dump(), "models": {"m": {"backend": "fake", "chips": 1, "hbm_gb_per_chip": 5, "max_replicas": 1, "concurrency": 4, "phase": "p", "wake_s": 0}}})
    handles: list[FakeHandle] = []
    manager = ResourceManager(cfg, cpu=CpuPool(cores=8, ram_gb=32), spawn=fake_spawn(handles), sample_s=0.005)
    manager.start()
    await manager.run("m", "warm", {})
    slot = next(iter(manager.slots.values()))
    handles[0].extra = 7.4  # 12.4 GB: 77.5%, above the 76% soft limit
    await asyncio.sleep(0.05)
    assert slot.throttled
    waiting = asyncio.create_task(manager.run("m", "work", {}))
    await asyncio.sleep(0.1)
    assert not waiting.done()
    handles[0].extra = 0.0
    assert (await asyncio.wait_for(waiting, 2))["model"] == "m"
    assert not slot.throttled
    await manager.close()


async def test_a_model_that_cannot_start_fails_its_jobs_fast():
    async def broken(spec, chips, cpu):
        raise ReplicaError("weights missing")

    manager = ResourceManager(config(0.80), cpu=CpuPool(cores=32, ram_gb=300), spawn=broken)
    manager.start()
    with pytest.raises(ReplicaError, match="magi cannot start: weights missing"):
        await asyncio.wait_for(manager.run("magi", "work", {}), 2)
    await manager.close()
    assert manager.ledger.free() == list(range(8))


async def test_real_replica_processes_through_the_manager(tmp_path):
    cfg = config(0.80)
    manager = ResourceManager(cfg, cpu=CpuPool(cores=32, ram_gb=300), log_dir=tmp_path, sample_s=0.05)
    manager.start()
    results = await asyncio.wait_for(asyncio.gather(*(manager.run("librarian", "ops", {"sleep_s": 0.05}) for _ in range(6))), 30)
    assert all(len(r["chips"]) == 2 for r in results)
    status = manager.status()
    assert status["phase"] == "librarian" and any(c["replica"] for c in status["chips"])
    await manager.close()
    assert manager.ledger.free() == list(range(8)) and manager.slots == {}
