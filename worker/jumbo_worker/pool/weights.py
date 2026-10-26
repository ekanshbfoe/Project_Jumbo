"""Weight cache in host RAM (architecture §4.4): a replica that sleeps (exits) leaves its weights in
`/dev/shm`, so waking it reloads from RAM instead of the Kaggle input mount. Entries in use by a
live replica are pinned; others are evicted least recently used when the RAM budget runs out.

The XLA compile caches live next to it on disk, so a re-woken replica skips most compilation; the
app saves that folder as a Kaggle dataset version between sessions.
"""

from __future__ import annotations

import asyncio
import shutil
import time
from pathlib import Path

DEFAULT_ROOT = Path("/dev/shm/jumbo-weights")
COMPLETE = ".complete"


def tree_size(path: Path) -> int:
    if path.is_file():
        return path.stat().st_size
    return sum(p.stat().st_size for p in path.rglob("*") if p.is_file())


def compile_cache_env(cache_dir: Path) -> dict[str, str]:
    """Persistent compilation caches for each TPU framework, all in one folder."""
    return {
        "JAX_COMPILATION_CACHE_DIR": str(cache_dir / "jax"),
        "JAX_PERSISTENT_CACHE_MIN_COMPILE_TIME_SECS": "0",
        "VLLM_XLA_CACHE_PATH": str(cache_dir / "vllm"),
        "JUMBO_TORCH_XLA_CACHE": str(cache_dir / "torch_xla"),
    }


class WeightCache:
    def __init__(self, sources: Path, root: Path = DEFAULT_ROOT, budget_gb: float | None = None) -> None:
        self.sources = sources
        self.root = root
        # Default: half of /dev/shm, leaving the rest to replicas and the OS.
        self.budget = int(budget_gb * 1e9) if budget_gb is not None else self._shm_bytes() // 2
        self.pins: dict[str, int] = {}
        self._locks: dict[str, asyncio.Lock] = {}

    def _shm_bytes(self) -> int:
        self.root.mkdir(parents=True, exist_ok=True)
        return shutil.disk_usage(self.root).total

    def cached(self) -> dict[str, int]:
        if not self.root.exists():
            return {}
        return {p.name: tree_size(p) - (p / COMPLETE).stat().st_size for p in self.root.iterdir() if (p / COMPLETE).exists()}

    async def ensure(self, name: str) -> Path:
        """Path to `name` in RAM, staging it from the sources first if needed."""
        lock = self._locks.setdefault(name, asyncio.Lock())
        async with lock:
            target = self.root / name
            if (target / COMPLETE).exists():
                (target / COMPLETE).touch()
                return target
            source = self.sources / name
            if not source.exists():
                raise FileNotFoundError(f"weights {name!r} not found under {self.sources}")
            size = await asyncio.to_thread(tree_size, source)
            if size > self.budget:
                raise MemoryError(f"weights {name!r} ({size / 1e9:.1f} GB) exceed the RAM cache budget ({self.budget / 1e9:.1f} GB)")
            self._evict_for(size)
            partial = self.root / f".{name}.partial"
            shutil.rmtree(partial, ignore_errors=True)
            if source.is_dir():
                await asyncio.to_thread(shutil.copytree, source, partial)
            else:
                partial.mkdir(parents=True)
                await asyncio.to_thread(shutil.copy2, source, partial / source.name)
            (partial / COMPLETE).write_text(str(time.time()))
            shutil.rmtree(target, ignore_errors=True)
            partial.rename(target)
            return target

    def _evict_for(self, size: int) -> None:
        entries = sorted(
            ((p.name, (p / COMPLETE).stat().st_mtime) for p in self.root.iterdir() if (p / COMPLETE).exists()),
            key=lambda e: e[1],
        ) if self.root.exists() else []
        used = sum(self.cached().values())
        for name, _ in entries:
            if used + size <= self.budget:
                return
            if self.pins.get(name):
                continue
            freed = tree_size(self.root / name)
            shutil.rmtree(self.root / name)
            used -= freed
        if used + size > self.budget:
            raise MemoryError(f"RAM cache is full of pinned weights; {size / 1e9:.1f} GB more does not fit")

    def pin(self, name: str) -> None:
        self.pins[name] = self.pins.get(name, 0) + 1

    def unpin(self, name: str) -> None:
        self.pins[name] = max(0, self.pins.get(name, 0) - 1)
