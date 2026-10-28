"""Layer 3 of the HBM ceiling: per-chip memory as the replicas report it, sampled every second.

- At or above the soft limit (ceiling - 4 points) *and* beyond its own profiled footprint, a replica
  is throttled: it gets no new work until it drops back. The footprint test matters for vLLM, which
  pre-allocates every chip up to the ceiling by design and must not be throttled for doing so.
- Above the ceiling it is killed and its jobs are retried (jobs are idempotent). Exactly at the
  ceiling is allowed: that is where the allocator caps put a fully used replica.
Peaks are kept per chip for `pool.status` and the S0 soak report.

GPU cards are shared by several replicas, so they are judged per card on the sum of what every
replica on it reports: above the ceiling the replica furthest beyond its profiled footprint is
killed; at or above the soft limit the replicas beyond their footprint are throttled.
"""

from __future__ import annotations

from dataclasses import dataclass, field


@dataclass(frozen=True)
class Verdict:
    replica: str
    level: str  # "ok" | "soft" | "hard"
    chip: int | None
    fraction: float


@dataclass(frozen=True)
class CardVerdict:
    card: int
    fraction: float
    kill: str | None
    throttle: frozenset[str]


@dataclass
class Watchdog:
    chip_hbm_gb: float
    soft: float
    hard: float
    peaks: dict[int, float] = field(default_factory=dict)

    def judge(self, replica: str, sample: dict[int, float], profile_gb: float = 0.0) -> Verdict:
        """Worst chip of one replica's sample (GB in use per chip); `profile_gb` is its admitted footprint."""
        worst_chip, worst = None, 0.0
        for chip, used_gb in sample.items():
            fraction = used_gb / self.chip_hbm_gb
            self.peaks[chip] = max(self.peaks.get(chip, 0.0), fraction)
            if fraction > worst:
                worst_chip, worst = chip, fraction
        if worst > self.hard + 1e-9:
            level = "hard"
        elif worst >= self.soft and worst * self.chip_hbm_gb > profile_gb + 1e-9:
            level = "soft"
        else:
            level = "ok"
        return Verdict(replica, level, worst_chip, worst)

    def judge_card(self, card: int, used_gb: dict[str, float], profile_gb: dict[str, float], hard: float | None = None) -> CardVerdict:
        """One shared card: `used_gb` is each replica's own usage on it, `profile_gb` its reservation.
        `hard` replaces the ceiling on a card held by a replica that runs alone; such a card is never throttled."""
        fraction = sum(used_gb.values()) / self.chip_hbm_gb
        self.peaks[card] = max(self.peaks.get(card, 0.0), fraction)
        over = {r: used - profile_gb.get(r, 0.0) for r, used in used_gb.items()}
        if used_gb and fraction > (hard or self.hard) + 1e-9:
            return CardVerdict(card, fraction, kill=max(used_gb, key=lambda r: (over[r], used_gb[r])), throttle=frozenset())
        if hard is None and fraction >= self.soft:
            return CardVerdict(card, fraction, kill=None, throttle=frozenset(r for r, o in over.items() if o > 1e-9))
        return CardVerdict(card, fraction, kill=None, throttle=frozenset())

    def peak(self) -> float:
        return max(self.peaks.values(), default=0.0)
