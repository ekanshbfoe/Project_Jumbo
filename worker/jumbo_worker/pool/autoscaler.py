"""The scaling policy (architecture §4.5) as a pure function of the pool's state, so every decision
can be tested without processes. The manager runs it on every queue or replica event and on a tick.

- desired[model] = clamp(ceil(backlog / target_per_replica), min, max)
- TPU phases: models of one phase hold the chips at a time. The phase switches when the other phase's
  queued work beats the current one by the swap cost plus a hysteresis margin (or the current phase
  has nothing left), and never before the current phase has held the chips for the minimum dwell.
- CPU-capable models overflow to the CpuPool while their phase is out or the chips are full. On a
  GPU host (no chips) that path runs them on a CUDA card instead, and a replica only starts when a
  card still has its profiled footprint free under the ceiling (GpuLedger).
- Idle replicas go to sleep (exit) after their TTL, down to min_replicas.
"""

from __future__ import annotations

import copy
import math
from dataclasses import dataclass, field

from .chips import ChipLease, ChipLedger, GpuLedger
from .cpu import CpuLease, CpuPool
from .specs import ModelSpec, PoolConfig


@dataclass(frozen=True)
class ReplicaView:
    id: str
    model: str
    chips: ChipLease | None
    cpu: CpuLease
    inflight: int
    idle_s: float
    stopping: bool = False

    @property
    def on_cpu(self) -> bool:
        return self.chips is None


@dataclass(frozen=True)
class Start:
    model: str
    on_cpu: bool


@dataclass
class Plan:
    phase: str | None
    start: list[Start] = field(default_factory=list)
    stop: list[str] = field(default_factory=list)
    reason: str = ""


def desired(spec: ModelSpec, backlog: int) -> int:
    want = math.ceil(backlog / spec.target_queue_per_replica) if backlog else 0
    return max(spec.min_replicas, min(spec.max_replicas, want))


def phase_work_s(config: PoolConfig, phase: str, backlog: dict[str, int]) -> float:
    """Seconds of queued and running work waiting on a phase's models."""
    return sum(backlog.get(s.name, 0) * s.job_s for s in config.phases().get(phase, []))


def choose_phase(config: PoolConfig, current: str | None, held_s: float, backlog: dict[str, int]) -> tuple[str | None, str]:
    work = {p: phase_work_s(config, p, backlog) for p in config.phases()}
    busiest = max(work, key=lambda p: work[p], default=None)
    if busiest is None or work[busiest] <= 0:
        return current, "no TPU work"
    if current is None or work.get(current, 0) <= 0:
        return busiest, f"{busiest} has the only TPU work" if current is None else f"{current} is idle; {busiest} has work"
    if busiest == current:
        return current, f"{current} has the most work"
    if held_s < config.phase_min_dwell_s:
        return current, f"{current} is within its minimum dwell"
    swap_s = max(s.wake_s for s in config.phases()[busiest])
    if work[busiest] > work[current] + swap_s + config.phase_hysteresis_s:
        return busiest, f"{busiest} work {work[busiest]:.0f}s beats {current} {work[current]:.0f}s + swap {swap_s:.0f}s + margin"
    return current, f"{busiest} does not yet outweigh the swap"


def plan(
    config: PoolConfig,
    queued: dict[str, int],
    replicas: list[ReplicaView],
    phase: str | None,
    held_s: float,
    ledger: ChipLedger,
    cpu: CpuPool,
    gpus: GpuLedger | None = None,
) -> Plan:
    live = [r for r in replicas if not r.stopping]
    inflight: dict[str, int] = {}
    for r in live:
        inflight[r.model] = inflight.get(r.model, 0) + r.inflight
    backlog = {m: queued.get(m, 0) + inflight.get(m, 0) for m in config.models}

    new_phase, reason = choose_phase(config, phase, held_s, backlog)
    out = Plan(new_phase, reason=reason)

    # Placement is simulated on copies: leases of replicas being stopped come back first.
    chips, cores, cards = copy.deepcopy(ledger), copy.deepcopy(cpu), copy.deepcopy(gpus)

    def stop(r: ReplicaView) -> None:
        out.stop.append(r.id)
        if r.chips:
            chips.release(r.chips)
        cores.release(r.cpu)
        if cards:
            cards.release(r.id)

    # TPU replicas of other phases give their chips back once their work is done.
    for r in live:
        spec = config.spec(r.model)
        if r.chips and spec.phase != new_phase and r.inflight == 0:
            stop(r)

    tpu_count = {m: sum(1 for r in live if r.model == m and r.chips and r.id not in out.stop) for m in config.models}
    cpu_count = {m: sum(1 for r in live if r.model == m and r.on_cpu) for m in config.models}

    # Idle replicas sleep after their TTL, keeping min_replicas of the active models.
    for r in sorted(live, key=lambda r: -r.idle_s):
        spec = config.spec(r.model)
        if r.id in out.stop or r.inflight or r.idle_s < spec.idle_ttl_s or queued.get(r.model, 0):
            continue
        if r.on_cpu:
            # A CPU fallback is not kept around once the chips can serve the model.
            if spec.on_tpu or cpu_count[r.model] > spec.min_replicas:
                stop(r)
                cpu_count[r.model] -= 1
        elif tpu_count[r.model] > spec.min_replicas:
            stop(r)
            tpu_count[r.model] -= 1

    order = sorted(config.models.values(), key=lambda s: (-s.priority, -backlog[s.name]))
    for spec in order:
        want = desired(spec, backlog[spec.name])
        if spec.on_tpu and spec.phase == new_phase:
            while tpu_count[spec.name] + cpu_count[spec.name] < want and chips.can_place(spec.chips, spec.hbm_gb_per_chip) and cores.can_lease(config.replica_cores(spec, True), spec.host_ram_gb):
                chips.lease(f"plan:{spec.name}:{tpu_count[spec.name]}", spec.chips, spec.hbm_gb_per_chip)
                cores.lease(f"plan:{spec.name}:t{tpu_count[spec.name]}", config.replica_cores(spec, True), spec.host_ram_gb)
                out.start.append(Start(spec.name, on_cpu=False))
                tpu_count[spec.name] += 1
        # CPU replicas: CPU-only models, and fallbacks for TPU models the chips cannot serve right now.
        cpu_ok = not spec.on_tpu or (spec.cpu_fallback and queued.get(spec.name, 0) > 0 and tpu_count[spec.name] == 0)
        if cpu_ok:
            on_gpu = spec.on_tpu and config.gpus > 0

            def gpu_fits() -> bool:
                return not on_gpu or (cards is not None and cards.can_place(spec.hbm_gb_per_chip, spec.gpus, spec.exclusive))

            while tpu_count[spec.name] + cpu_count[spec.name] < want and cores.can_lease(config.replica_cores(spec, False), spec.host_ram_gb) and gpu_fits():
                cores.lease(f"plan:{spec.name}:c{cpu_count[spec.name]}", config.replica_cores(spec, False), spec.host_ram_gb)
                if on_gpu and cards is not None:
                    cards.lease(f"plan:{spec.name}:g{cpu_count[spec.name]}", spec.hbm_gb_per_chip, spec.gpus, spec.exclusive)
                out.start.append(Start(spec.name, on_cpu=True))
                cpu_count[spec.name] += 1
    return out
