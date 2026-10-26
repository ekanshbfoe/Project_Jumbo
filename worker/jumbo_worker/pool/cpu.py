"""Host cores and RAM. CPU stages (Remotion, polygon refinement, CPU-fallback models) lease from here,
and so do TPU replicas for their host-side share. Real counts are read at boot, never assumed."""

from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from .chips import LeaseError

# Kept back for the control plane, the tunnel and the OS.
RESERVED_CORES = 1
RESERVED_RAM_GB = 4.0


@dataclass(frozen=True)
class CpuLease:
    owner: str
    cores: int
    ram_gb: float


def host_ram_gb(meminfo: Path = Path("/proc/meminfo")) -> float:
    try:
        for line in meminfo.read_text().splitlines():
            if line.startswith("MemTotal:"):
                return int(line.split()[1]) / 1024**2
    except OSError:
        pass
    return 0.0


class CpuPool:
    def __init__(self, cores: int, ram_gb: float) -> None:
        self.cores = cores
        self.ram_gb = ram_gb
        self._leases: dict[str, CpuLease] = {}

    @classmethod
    def detect(cls) -> CpuPool:
        cores = len(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else os.cpu_count() or 1
        return cls(max(1, cores - RESERVED_CORES), max(1.0, host_ram_gb() - RESERVED_RAM_GB))

    def used(self) -> tuple[int, float]:
        return sum(l.cores for l in self._leases.values()), sum(l.ram_gb for l in self._leases.values())

    def can_lease(self, cores: int, ram_gb: float) -> bool:
        used_cores, used_ram = self.used()
        return used_cores + cores <= self.cores and used_ram + ram_gb <= self.ram_gb

    def lease(self, owner: str, cores: int, ram_gb: float) -> CpuLease:
        if owner in self._leases:
            raise LeaseError(f"{owner} already holds a CPU lease")
        if not self.can_lease(cores, ram_gb):
            used_cores, used_ram = self.used()
            raise LeaseError(f"{cores} cores / {ram_gb} GB requested; {self.cores - used_cores} cores / {self.ram_gb - used_ram:.1f} GB free")
        lease = CpuLease(owner, cores, ram_gb)
        self._leases[owner] = lease
        return lease

    def release(self, lease: CpuLease) -> None:
        if self._leases.get(lease.owner) == lease:
            del self._leases[lease.owner]

    def snapshot(self) -> dict:
        used_cores, used_ram = self.used()
        return {"cores": self.cores, "ram_gb": round(self.ram_gb, 1), "used_cores": used_cores, "used_ram_gb": round(used_ram, 1)}
