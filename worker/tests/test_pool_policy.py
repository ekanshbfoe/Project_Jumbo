import pytest

from jumbo_worker.pool.autoscaler import ReplicaView, choose_phase, desired, plan
from jumbo_worker.pool.chips import ChipLedger
from jumbo_worker.pool.cpu import CpuPool
from jumbo_worker.pool.specs import PoolConfig
from jumbo_worker.pool.watchdog import Watchdog

CONFIG = PoolConfig.model_validate(
    {
        "hbm_ceiling": 0.8,
        "phase_hysteresis_s": 60,
        "phase_min_dwell_s": 30,
        "models": {
            "magi": {"backend": "fake", "chips": 1, "hbm_gb_per_chip": 5, "cpu_fallback": True, "max_replicas": 8,
                     "target_queue_per_replica": 4, "job_s": 2, "wake_s": 20, "phase": "vision", "idle_ttl_s": 120, "priority": 1},
            "qwen": {"backend": "fake", "chips": 8, "hbm_gb_per_chip": 12.5, "cpu_cores": 8, "host_ram_gb": 90, "max_replicas": 1,
                     "target_queue_per_replica": 64, "job_s": 90, "wake_s": 600, "phase": "director", "priority": 2},
            "render": {"backend": "fake", "cpu_cores": 4, "host_ram_gb": 4, "max_replicas": 4, "target_queue_per_replica": 1, "priority": 5},
        },
    }
)


def pools():
    return ChipLedger(8, CONFIG.chip_budget_gb), CpuPool(cores=32, ram_gb=300)


def test_desired_replicas_follow_the_backlog_within_bounds():
    magi = CONFIG.spec("magi")
    assert [desired(magi, n) for n in (0, 1, 4, 5, 100)] == [0, 1, 1, 2, 8]


def test_phase_choice_has_dwell_and_hysteresis():
    assert choose_phase(CONFIG, None, 0, {"magi": 10})[0] == "vision"
    # Director work (2 x 90 s) does not beat vision (100 x 2 s) + swap (600 s) + margin.
    assert choose_phase(CONFIG, "vision", 300, {"magi": 100, "qwen": 2})[0] == "vision"
    # Enough chapters waiting on the Director and it wins, but only after the minimum dwell.
    assert choose_phase(CONFIG, "vision", 10, {"magi": 100, "qwen": 10})[0] == "vision"
    assert choose_phase(CONFIG, "vision", 300, {"magi": 100, "qwen": 10})[0] == "director"
    # An idle phase hands over at once.
    assert choose_phase(CONFIG, "vision", 1, {"qwen": 1})[0] == "director"


def test_scale_up_packs_single_chip_replicas_and_runs_cpu_stages_alongside():
    ledger, cpu = pools()
    p = plan(CONFIG, {"magi": 13, "render": 2}, [], None, 0, ledger, cpu)
    assert p.phase == "vision"
    assert sum(1 for s in p.start if s.model == "magi" and not s.on_cpu) == 4
    assert sum(1 for s in p.start if s.model == "render") == 2


def test_cpu_fallback_runs_while_the_phase_is_out():
    ledger, cpu = pools()
    qwen = ReplicaView("qwen#1", "qwen", ledger.lease("qwen#1", 8, 12.5), cpu.lease("qwen#1", 8, 90), inflight=3, idle_s=0)
    p = plan(CONFIG, {"magi": 4, "qwen": 5}, [qwen], "director", 5, ledger, cpu)
    assert p.phase == "director" and p.stop == []
    assert [(s.model, s.on_cpu) for s in p.start] == [("magi", True)]


def test_a_phase_switch_waits_for_busy_replicas_then_takes_the_chips():
    ledger, cpu = pools()
    magis = [ReplicaView(f"magi#{i}", "magi", ledger.lease(f"magi#{i}", 1, 5), cpu.lease(f"magi#{i}", 1, 1), inflight=i % 2, idle_s=0) for i in range(8)]
    p = plan(CONFIG, {"qwen": 10}, magis, "vision", 1000, ledger, cpu)
    assert p.phase == "director"
    # Idle ones stop now; busy ones finish first, so Qwen cannot be placed yet.
    assert sorted(p.stop) == [f"magi#{i}" for i in range(0, 8, 2)]
    assert p.start == []
    for r in magis:
        ledger.release(r.chips)
        cpu.release(r.cpu)
    p = plan(CONFIG, {"qwen": 10}, [], "director", 0, ledger, cpu)
    assert [(s.model, s.on_cpu) for s in p.start] == [("qwen", False)]


def test_idle_replicas_sleep_after_their_ttl():
    ledger, cpu = pools()
    fresh = ReplicaView("magi#1", "magi", ledger.lease("magi#1", 1, 5), cpu.lease("magi#1", 1, 1), inflight=0, idle_s=10)
    stale = ReplicaView("magi#2", "magi", ledger.lease("magi#2", 1, 5), cpu.lease("magi#2", 1, 1), inflight=0, idle_s=500)
    p = plan(CONFIG, {}, [fresh, stale], "vision", 1000, ledger, cpu)
    assert p.stop == ["magi#2"]


def test_watchdog_levels_and_peaks():
    dog = Watchdog(chip_hbm_gb=16, soft=0.76, hard=0.80)
    assert dog.judge("a", {0: 8.0, 1: 9.0}).level == "ok"
    soft = dog.judge("a", {0: 12.3})
    assert (soft.level, soft.chip) == ("soft", 0)
    # Exactly at the ceiling is where the allocator caps put a full replica; above it is a kill.
    assert dog.judge("b", {3: 12.8}).level == "soft"
    assert dog.judge("b", {3: 12.81}).level == "hard"
    # vLLM sits above the soft line by design; only growth beyond its profile throttles it.
    assert dog.judge("q", {4: 12.5}, profile_gb=12.5).level == "ok"
    assert dog.judge("q", {4: 12.7}, profile_gb=12.5).level == "soft"
    assert dog.peaks[0] == pytest.approx(12.3 / 16) and dog.peak() == pytest.approx(12.81 / 16)
