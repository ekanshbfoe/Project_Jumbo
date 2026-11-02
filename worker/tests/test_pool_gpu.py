"""GPU hosts: replicas share the CUDA cards under the ceiling, and the card watchdog enforces it."""

import asyncio

from jumbo_worker.pool.chips import GpuLedger, LeaseError
from jumbo_worker.pool.cpu import CpuPool
from jumbo_worker.pool.manager import ResourceManager
from jumbo_worker.pool.specs import PoolConfig, load_specs
from jumbo_worker.pool.watchdog import Watchdog


def gpu_config(**det) -> PoolConfig:
    spec = {"backend": "fake", "chips": 1, "hbm_gb_per_chip": 5, "phase": "vision", "cpu_fallback": True, "cpu_cores": 1, "max_replicas": 8, "target_queue_per_replica": 1, "wake_s": 0, **det}
    return PoolConfig.model_validate({"chips": 0, "gpus": 2, "gpu_hbm_gb": 15, "tick_s": 0.02, "models": {"det": spec}})


def test_gpu_ledger_spreads_replicas_and_never_reserves_past_the_budget():
    cards = GpuLedger(2, 12.0)
    assert [cards.lease(f"r{i}", 5) for i in range(4)] == [(1,), (0,), (1,), (0,)]
    assert not cards.can_place(5) and cards.can_place(2)
    try:
        cards.lease("r4", 5)
        raise AssertionError("over budget")
    except LeaseError:
        pass
    cards.release("r1")
    assert cards.lease("r5", 5) == (0,) and cards.reserved() == [10.0, 10.0]


def test_card_watchdog_kills_the_replica_furthest_past_its_footprint():
    dog = Watchdog(15.0, soft=0.76, hard=0.80)
    ok = dog.judge_card(0, {"a": 5.0, "b": 5.0}, {"a": 5.0, "b": 5.0})
    assert ok.kill is None and not ok.throttle
    soft = dog.judge_card(0, {"a": 5.0, "b": 6.5}, {"a": 5.0, "b": 5.0})
    assert soft.kill is None and soft.throttle == {"b"}
    hard = dog.judge_card(0, {"a": 5.5, "b": 7.0}, {"a": 5.0, "b": 5.0})
    assert hard.kill == "b" and dog.peaks[0] > 0.8


class FakeGpu:
    """A replica handle that reports memory on its card and records when it is busy."""

    def __init__(self, gpus, log, grow=0.0):
        self.gpu, self.log, self.grow, self.used = gpus[0], log, grow, 5.0

    async def run(self, kind, payload):
        self.log.append(("start", self.gpu))
        self.used += self.grow
        try:
            await asyncio.sleep(payload.get("sleep_s", 0.1))
        finally:
            self.used -= self.grow
        self.log.append(("end", self.gpu))
        return self.gpu

    async def memory(self):
        return {self.gpu: self.used}

    async def stop(self):
        pass

    async def kill(self):
        pass


async def test_parallel_requests_run_on_both_cards_at_once_within_the_budget():
    cfg = gpu_config()
    log, placed = [], []

    async def spawn(spec, chips, cpu, weights=None, gpus=()):
        placed.append(gpus)
        return FakeGpu(gpus, log)

    pool = ResourceManager(cfg, cpu=CpuPool(8, 64), spawn=spawn, sample_s=0.01)
    pool.start()
    cards = await asyncio.gather(*(pool.run("det", "x", {"sleep_s": 0.2}) for _ in range(12)))
    peak_reserved = max(pool.gpus.reserved())
    await pool.close()
    # 12 GB budget per card (80% of 15) fits two 5 GB replicas: four in all, two per card.
    assert sorted(placed) == [(0,), (0,), (1,), (1,)] and set(cards) == {0, 1} and peak_reserved <= cfg.gpu_budget_gb
    # Both cards were busy at the same moment.
    busy, both = set(), False
    for event, gpu in log:
        busy.add(gpu) if event == "start" else busy.discard(gpu)
        both = both or busy == {0, 1}
    assert both


async def test_a_replica_growing_past_the_card_ceiling_is_killed_and_its_request_retried():
    cfg = gpu_config(max_replicas=2)
    log, handles = [], []

    async def spawn(spec, chips, cpu, weights=None, gpus=()):
        # The first replica leaks 6 GB while it works: 5 + 6 = 11 GB is 73% alone, but with the
        # other replica's 5 GB on the same card the sum (16 GB) is over 80% of 15 GB.
        handles.append(FakeGpu(gpus, log, grow=6.0 if not handles else 0.0))
        return handles[-1]

    events = []
    pool = ResourceManager(cfg.model_copy(update={"gpus": 1}), cpu=CpuPool(8, 64), spawn=spawn, sample_s=0.01, publish=events.append)
    pool.start()
    results = await asyncio.gather(*(pool.run("det", "x", {"sleep_s": 0.3}) for _ in range(2)))
    await pool.close()
    assert results == [0, 0] and pool.kills >= 1
    assert any(e["type"] == "pool.hbm_kill" and e.get("gpu") == 0 for e in events)


def test_magi_fits_one_replica_per_t4_on_the_kaggle_gpu_host():
    # Kaggle T4 x2: 4 vCPUs, 1 kept for the worker; Magi must leave room for a replica on each card.
    cfg = load_specs(chips=0, gpus=2)
    magi = cfg.spec("magi_v2")
    cores = CpuPool(4 - 1, 31.3 - 4)
    for _ in range(2):
        cores.lease(f"m{_}", cfg.replica_cores(magi, False), magi.host_ram_gb)
    cards = GpuLedger(2, cfg.gpu_budget_gb)
    assert {cards.lease("a", magi.hbm_gb_per_chip), cards.lease("b", magi.hbm_gb_per_chip)} == {(0,), (1,)}
    assert magi.target_queue_per_replica <= 2


def test_a_two_card_replica_reserves_both_cards_and_waits_for_both_to_be_free():
    cards = GpuLedger(2, 12.0)
    assert cards.lease("magi", 5) == (1,)
    # 11.8 GB on each card does not fit beside Magi's 5 GB.
    assert not cards.can_place(11.8, 2)
    cards.release("magi")
    assert cards.lease("drama3", 11.8, 2) == (0, 1) and cards.reserved() == [11.8, 11.8]
    assert cards.cards_of("drama3") == (0, 1) and not cards.can_place(1, 1)
    assert [c["replicas"] for c in cards.snapshot()] == [["drama3"], ["drama3"]]


async def test_a_two_card_replica_sees_both_cards_and_is_judged_on_each():
    cfg = gpu_config(gpus=2, hbm_gb_per_chip=11.0, max_replicas=1)
    placed, events = [], []

    class TwoCards(FakeGpu):
        async def memory(self):
            return {0: 11.0, 1: self.used}

    async def spawn(spec, chips, cpu, weights=None, gpus=()):
        placed.append(gpus)
        handle = TwoCards(gpus, [])
        handle.used = 13.0  # card 1 is past 80% of 15 GB
        return handle

    pool = ResourceManager(cfg, cpu=CpuPool(8, 64), spawn=spawn, sample_s=0.01, publish=events.append)
    pool.start()
    await pool.run("det", "x", {"sleep_s": 0.2})
    await pool.close()
    assert placed[0] == (0, 1)
    assert any(e["type"] == "pool.hbm_kill" and e.get("gpu") == 1 for e in events)


def test_a_model_needing_more_cards_than_the_host_has_is_refused():
    try:
        gpu_config(gpus=3)
        raise AssertionError("accepted")
    except ValueError as e:
        assert "needs 3 GPUs" in str(e)


def test_gpu_memory_is_reported_in_the_gib_the_budget_uses(monkeypatch):
    import os
    import sys
    import types

    from jumbo_worker.pool import replica_main

    # Spike 5's Drama 3 peak on CUDA0: 11771 MiB, under the 12 GiB hard line (80% of 15360 MiB).
    proc = types.SimpleNamespace(pid=os.getpid(), usedGpuMemory=11771 * 2**20)
    nvml = types.SimpleNamespace(nvmlInit=lambda: None, nvmlDeviceGetHandleByIndex=lambda i: i, nvmlDeviceGetComputeRunningProcesses=lambda h: [proc])
    monkeypatch.setitem(sys.modules, "pynvml", nvml)
    used = replica_main.gpu_memory_gb([0, 1])
    assert round(used[0], 2) == 11.5 and used[0] < load_specs(chips=0, gpus=2).gpu_budget_gb


def test_drama3_takes_both_t4s_of_the_kaggle_gpu_host():
    # Setup (c): one Q5_K replica split over both cards, with the RAM and cores the host has left.
    cfg = load_specs(chips=0, gpus=2)
    drama = cfg.spec("drama3")
    assert drama.gpus == 2 and drama.options["quant"] == "q5_k"
    cards = GpuLedger(2, cfg.gpu_budget_gb)
    assert cards.lease("voice", drama.hbm_gb_per_chip, drama.gpus) == (0, 1)
    cores = CpuPool(4 - 1, 31.3 - 4)
    cores.lease("voice", cfg.replica_cores(drama, False), drama.host_ram_gb)
    # Whisper hears its takes beside it, on one card, under the ceiling, with the RAM left and no cores of its own.
    ear = cfg.spec("whisper")
    assert ear.phase == drama.phase and ear.pip
    assert len(cards.lease("ear", ear.hbm_gb_per_chip, ear.gpus)) == 1 and max(cards.reserved()) <= cfg.gpu_budget_gb
    cores.lease("ear", cfg.replica_cores(ear, False), ear.host_ram_gb)


def test_a_replica_running_alone_takes_empty_cards_past_the_budget_and_keeps_others_off():
    cards = GpuLedger(2, 12.0, card_gb=15.0)
    assert cards.lease("magi", 5) == (1,)
    # The Director's 14.4 GB per card needs both cards empty.
    assert not cards.can_place(14.4, 2, alone=True)
    cards.release("magi")
    assert cards.lease("director", 14.4, 2, alone=True) == (0, 1)
    assert cards.alone_on(1) == "director" and not cards.can_place(1)
    assert not cards.can_place(15.5, 1, alone=True)
    cards.release("director")
    assert cards.alone_on(0) is None and cards.can_place(5)


def test_a_card_held_alone_is_judged_against_its_own_footprint():
    dog = Watchdog(15.0, soft=0.76, hard=0.80)
    ok = dog.judge_card(0, {"d": 14.4}, {"d": 14.4}, hard=0.98)
    assert ok.kill is None and not ok.throttle
    assert dog.judge_card(0, {"d": 14.9}, {"d": 14.4}, hard=0.98).kill == "d"


def test_only_a_model_marked_exclusive_may_pass_the_budget():
    try:
        gpu_config(gpus=2, hbm_gb_per_chip=14.4)
        raise AssertionError("accepted")
    except ValueError as e:
        assert "exceeds the 12.8 GB budget" in str(e)
    gpu_config(gpus=2, hbm_gb_per_chip=14.4, exclusive=True)


async def test_an_exclusive_replica_is_placed_alone_and_survives_past_the_ceiling():
    cfg = gpu_config(gpus=2, hbm_gb_per_chip=14.4, max_replicas=1, exclusive=True)
    placed, events = [], []

    class Full(FakeGpu):
        async def memory(self):
            return {0: 14.4, 1: 14.4}

    async def spawn(spec, chips, cpu, weights=None, gpus=()):
        placed.append(gpus)
        return Full(gpus, [])

    pool = ResourceManager(cfg, cpu=CpuPool(8, 64), spawn=spawn, sample_s=0.01, publish=events.append)
    pool.start()
    await pool.run("det", "x", {"sleep_s": 0.2})
    await pool.close()
    assert placed == [(0, 1)] and pool.kills == 0
    assert not any(e["type"] in ("pool.hbm_kill", "pool.hbm_soft") for e in events)


def test_the_director_runs_alone_on_both_t4s():
    cfg = load_specs(chips=0, gpus=2)
    director = cfg.spec("qwen3vl_32b")
    assert director.exclusive and director.gpus == 2 and director.hbm_gb_per_chip > cfg.gpu_budget_gb
    cards = GpuLedger(2, cfg.gpu_budget_gb, cfg.gpu_hbm_gb)
    assert cards.lease("director", director.hbm_gb_per_chip, director.gpus, alone=True) == (0, 1)
    magi = cfg.spec("magi_v2")
    assert not cards.can_place(magi.hbm_gb_per_chip)
    cores = CpuPool(4 - 1, 31.3 - 4)
    cores.lease("director", cfg.replica_cores(director, False), director.host_ram_gb)
