"""Chip leases on the TPU host. A chip belongs to exactly one replica process at a time (R1), so the
ledger hands out whole chips; the HBM ceiling is then a per-chip property checked at admission."""

from __future__ import annotations

from dataclasses import dataclass

from .specs import LEASE_SIZES


class LeaseError(Exception):
    """No placement is possible right now (or the request can never fit)."""


@dataclass(frozen=True)
class ChipLease:
    owner: str
    chips: tuple[int, ...]
    hbm_gb_per_chip: float


class ChipLedger:
    def __init__(self, chips: int, budget_gb: float) -> None:
        self.size = chips
        self.budget_gb = budget_gb
        self._owner: list[str | None] = [None] * chips
        self._hbm: list[float] = [0.0] * chips

    def free(self) -> list[int]:
        return [i for i, o in enumerate(self._owner) if o is None]

    def can_place(self, n: int, hbm_gb_per_chip: float) -> bool:
        return self._place(n, hbm_gb_per_chip) is not None

    def lease(self, owner: str, n: int, hbm_gb_per_chip: float) -> ChipLease:
        if n not in LEASE_SIZES or n > self.size:
            raise LeaseError(f"cannot lease {n} chips on a {self.size}-chip host")
        if hbm_gb_per_chip > self.budget_gb:
            raise LeaseError(f"{hbm_gb_per_chip} GB/chip is over the {self.budget_gb:.1f} GB budget")
        start = self._place(n, hbm_gb_per_chip)
        if start is None:
            raise LeaseError(f"no free aligned block of {n} chips")
        chips = tuple(range(start, start + n))
        for c in chips:
            self._owner[c] = owner
            self._hbm[c] = hbm_gb_per_chip
        return ChipLease(owner, chips, hbm_gb_per_chip)

    def release(self, lease: ChipLease) -> None:
        for c in lease.chips:
            if self._owner[c] == lease.owner:
                self._owner[c] = None
                self._hbm[c] = 0.0

    def _place(self, n: int, hbm: float) -> int | None:
        """Best fit: among free aligned blocks of `n`, pick the one inside the busiest enclosing
        block, so fragments stay together and an 8-chip lease becomes possible as soon as it can."""
        if n not in LEASE_SIZES or n > self.size or hbm > self.budget_gb:
            return None
        best: tuple[int, int] | None = None
        for start in range(0, self.size - n + 1, n):
            if any(self._owner[c] is not None for c in range(start, start + n)):
                continue
            parent = start - start % (2 * n)
            busy = sum(self._owner[c] is not None for c in range(parent, min(parent + 2 * n, self.size)))
            if best is None or busy > best[0]:
                best = (busy, start)
        return None if best is None else best[1]

    def reserved_gb(self) -> list[float]:
        """Admitted HBM per chip; by construction never above the budget."""
        return list(self._hbm)

    def snapshot(self) -> list[dict]:
        return [{"chip": i, "owner": o, "reserved_gb": h} for i, (o, h) in enumerate(zip(self._owner, self._hbm))]


class GpuLedger:
    """CUDA cards on a GPU host. Unlike TPU chips they are shared: every replica on a card holds a
    reservation of its profiled footprint, and a card never has more reserved than its budget (the
    ceiling's share of its memory). New replicas go to the card with the most room left."""

    def __init__(self, gpus: int, budget_gb: float) -> None:
        self.size = gpus
        self.budget_gb = budget_gb
        self._held: dict[str, tuple[int, float]] = {}

    def reserved(self) -> list[float]:
        out = [0.0] * self.size
        for card, gb in self._held.values():
            out[card] += gb
        return out

    def _choose(self, gb: float) -> int | None:
        reserved = self.reserved()
        fits = [c for c in range(self.size) if reserved[c] + gb <= self.budget_gb + 1e-9]
        return min(fits, key=lambda c: (reserved[c], c), default=None)

    def can_place(self, gb: float) -> bool:
        return self._choose(gb) is not None

    def lease(self, owner: str, gb: float) -> int:
        card = self._choose(gb)
        if card is None:
            raise LeaseError(f"no GPU has {gb} GB left under its {self.budget_gb:.1f} GB budget")
        self._held[owner] = (card, gb)
        return card

    def release(self, owner: str) -> None:
        self._held.pop(owner, None)

    def card_of(self, owner: str) -> int | None:
        held = self._held.get(owner)
        return held[0] if held else None

    def snapshot(self) -> list[dict]:
        reserved = self.reserved()
        return [{"gpu": c, "reserved_gb": round(reserved[c], 3), "replicas": sorted(o for o, (card, _) in self._held.items() if card == c)} for c in range(self.size)]
