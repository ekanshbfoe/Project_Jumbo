"""The model registry (`worker/model_specs.json`) and the HBM ceiling every limit is derived from.

The ceiling is configuration, not code: S0 spike 6 decides between 0.80 and 0.90, and the per-chip
budget, the framework allocator caps and the watchdog thresholds all follow from that one number.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Literal

from pydantic import BaseModel, Field, model_validator

DEFAULT_SPECS = Path(__file__).resolve().parents[2] / "model_specs.json"

# A TPU v5e-8 host: 8 chips, 16 GB HBM each, every chip owned by exactly one process (libtpu lock).
TPU_CHIPS = 8
CHIP_HBM_GB = 16.0
# A replica may lease 1, 2, 4 or 8 chips: the slice shapes libtpu can bind in one process.
LEASE_SIZES = (1, 2, 4, 8)
# Soft watchdog threshold sits this far below the hard ceiling.
SOFT_MARGIN = 0.04
# Usable memory of one Kaggle T4 (16 GB nominal, about 15 GB after the driver's reserve).
GPU_HBM_GB = 15.0

Backend = Literal["vllm-tpu", "jax", "torch-xla", "ggml", "cpu", "node", "fake"]


class ModelSpec(BaseModel):
    name: str
    backend: Backend
    # Chips per replica; 0 for CPU-only stages.
    chips: int = Field(0, ge=0, le=TPU_CHIPS)
    # Profiled peak HBM per leased chip, in GB.
    hbm_gb_per_chip: float = Field(0.0, ge=0)
    # CUDA cards one replica spans on a GPU host (hbm_gb_per_chip is then its peak on each card).
    gpus: int = Field(1, ge=1)
    # Host resources a replica holds while it runs (on TPU too: tokenizers, preprocessing).
    cpu_cores: int = Field(1, ge=0)
    host_ram_gb: float = Field(1.0, ge=0)
    # Runs on the CpuPool when no chips are free (slower, but keeps the stage moving).
    cpu_fallback: bool = False
    # Cores a replica takes when it runs on CPU instead of chips (default: cpu_cores).
    cpu_fallback_cores: int | None = Field(None, ge=1)
    min_replicas: int = Field(0, ge=0)
    max_replicas: int = Field(1, ge=0)
    # Queued jobs one replica is expected to keep up with.
    target_queue_per_replica: int = Field(4, ge=1)
    # Requests one replica works on at once (continuous batching for vLLM).
    concurrency: int = Field(1, ge=1)
    # Higher goes first: stages closer to a finished deliverable win ties for chips and cores.
    priority: int = 0
    idle_ttl_s: float = Field(120.0, ge=0)
    # Seconds to wake a replica (load weights, compile); the phase arbiter weighs this.
    wake_s: float = Field(30.0, ge=0)
    # Expected seconds per job, used to compare backlogs across phases.
    job_s: float = Field(10.0, gt=0)
    # TPU phase this model belongs to; models in different phases never hold chips together.
    phase: str | None = None
    # Weight files, relative to the weights root (a Kaggle input mount).
    weights: str | None = None
    # Extra pip requirements: the replica then runs in an isolated env of its own (pool/envs.py),
    # for models whose dependencies clash with the worker's (e.g. a newer transformers than Magi's).
    pip: list[str] = Field(default_factory=list)
    # Model-specific settings its runner reads, e.g. {"quant": "q6_k"} for Drama 3.
    options: dict[str, str] = Field(default_factory=dict)
    notes: str = ""

    @model_validator(mode="after")
    def _shape(self) -> ModelSpec:
        if self.chips and self.chips not in LEASE_SIZES:
            raise ValueError(f"{self.name}: chips must be one of {LEASE_SIZES}")
        if self.chips and self.phase is None:
            raise ValueError(f"{self.name}: TPU models need a phase")
        if self.min_replicas > self.max_replicas:
            raise ValueError(f"{self.name}: min_replicas is above max_replicas")
        return self

    @property
    def on_tpu(self) -> bool:
        return self.chips > 0

    def cores(self, on_cpu: bool) -> int:
        """Cores one replica leases: its CPU-fallback share when it runs without chips."""
        return (self.cpu_fallback_cores or self.cpu_cores) if on_cpu and self.on_tpu else self.cpu_cores


class PoolConfig(BaseModel):
    hbm_ceiling: float = Field(0.80, gt=0.5, le=0.95)
    chips: int = Field(TPU_CHIPS, ge=0)
    # CUDA GPUs on the host (Kaggle's T4 x2). Accelerator models without chips run on them, shared,
    # each replica reserving its profiled footprint (hbm_gb_per_chip) on each of its spec.gpus cards.
    gpus: int = Field(0, ge=0)
    gpu_hbm_gb: float = Field(GPU_HBM_GB, gt=0)
    chip_hbm_gb: float = Field(CHIP_HBM_GB, gt=0)
    # A phase switch must win by this many seconds of queued work beyond the swap cost.
    phase_hysteresis_s: float = Field(60.0, ge=0)
    # A phase holds the chips at least this long before it can be switched out.
    phase_min_dwell_s: float = Field(30.0, ge=0)
    tick_s: float = Field(2.0, gt=0)
    models: dict[str, ModelSpec]

    @model_validator(mode="before")
    @classmethod
    def _names(cls, data: dict) -> dict:
        # Each spec's name is its key in the file.
        models = data.get("models", {})
        if isinstance(models, dict):
            data = {**data, "models": {k: {**v, "name": k} if isinstance(v, dict) else v for k, v in models.items()}}
        return data

    @model_validator(mode="after")
    def _admission(self) -> PoolConfig:
        # Layer 1 of the ceiling: a model that cannot fit under it on every chip is never placed.
        for spec in self.models.values():
            if spec.on_tpu and spec.hbm_gb_per_chip > self.chip_budget_gb:
                raise ValueError(
                    f"{spec.name}: {spec.hbm_gb_per_chip} GB/chip exceeds the {self.chip_budget_gb:.1f} GB budget "
                    f"({self.hbm_ceiling:.0%} of {self.chip_hbm_gb} GB)"
                )
            # A host without a TPU (local worker, Kaggle CPU) still loads every model: TPU models
            # run on their CPU fallback there, or report that they cannot start.
            if 0 < self.chips < spec.chips:
                raise ValueError(f"{spec.name}: needs {spec.chips} chips, the host has {self.chips}")
            if spec.on_tpu and 0 < self.gpus < spec.gpus:
                raise ValueError(f"{spec.name}: needs {spec.gpus} GPUs, the host has {self.gpus}")
        return self

    @property
    def chip_budget_gb(self) -> float:
        return self.hbm_ceiling * self.chip_hbm_gb

    @property
    def gpu_budget_gb(self) -> float:
        return self.hbm_ceiling * self.gpu_hbm_gb

    @property
    def soft_limit(self) -> float:
        return self.hbm_ceiling - SOFT_MARGIN

    def replica_cores(self, spec: ModelSpec, on_chips: bool) -> int:
        """Cores a replica leases: its CPU-fallback share only when it computes on the CPU itself."""
        if on_chips or (spec.on_tpu and self.gpus):
            return spec.cpu_cores
        return spec.cores(True)

    def phases(self) -> dict[str, list[ModelSpec]]:
        out: dict[str, list[ModelSpec]] = {}
        for spec in self.models.values():
            if spec.phase:
                out.setdefault(spec.phase, []).append(spec)
        return out

    def spec(self, name: str) -> ModelSpec:
        try:
            return self.models[name]
        except KeyError:
            raise KeyError(f"unknown model {name!r}") from None


def detect_tpu_chips(dev: Path = Path("/dev")) -> int:
    """TPU chips this host exposes: /dev/accel* on older TPU VMs, /dev/vfio/<n> on v5e and later."""
    accel = list(dev.glob("accel[0-9]*"))
    if accel:
        return len(accel)
    vfio = dev / "vfio"
    return sum(1 for p in vfio.iterdir() if p.name.isdigit()) if vfio.is_dir() else 0


def detect_cuda_gpus(dev: Path = Path("/dev")) -> int:
    """NVIDIA GPUs this host exposes (/dev/nvidia0, /dev/nvidia1, ...)."""
    return sum(1 for p in dev.glob("nvidia*") if p.name[len("nvidia"):].isdigit())


def load_specs(path: Path = DEFAULT_SPECS, **overrides) -> PoolConfig:
    raw = json.loads(path.read_text(encoding="utf-8"))
    raw.pop("$comment", None)
    return PoolConfig.model_validate({**raw, **overrides})
