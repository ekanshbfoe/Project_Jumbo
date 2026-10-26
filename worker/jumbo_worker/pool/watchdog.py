"""Layer 3 of the HBM ceiling: per-chip memory as the replicas report it, sampled every second.

- At or above the soft limit (ceiling - 4 points) a replica is throttled: it gets no new work until
  it drops back below.
- At or above the ceiling it is killed and its jobs are retried (jobs are idempotent).
Peaks are kept per chip for `pool.status` and the S0 soak report.
"""

from __future__ import annotations

from dataclasses import dataclass, field


@dataclass(frozen=True)
class Verdict:
    replica: str
    level: str  # "ok" | "soft" | "hard"
    chip: int | None
    fraction: float


@dataclass
class Watchdog:
    chip_hbm_gb: float
    soft: float
    hard: float
    peaks: dict[int, float] = field(default_factory=dict)

    def judge(self, replica: str, sample: dict[int, float]) -> Verdict:
        """Worst chip of one replica's sample (GB in use per chip)."""
        worst_chip, worst = None, 0.0
        for chip, used_gb in sample.items():
            fraction = used_gb / self.chip_hbm_gb
            self.peaks[chip] = max(self.peaks.get(chip, 0.0), fraction)
            if fraction > worst:
                worst_chip, worst = chip, fraction
        level = "hard" if worst >= self.hard else "soft" if worst >= self.soft else "ok"
        return Verdict(replica, level, worst_chip, worst)

    def peak(self) -> float:
        return max(self.peaks.values(), default=0.0)
