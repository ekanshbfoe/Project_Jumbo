"""Jobs reach models through the pool, and the worker sizes the pool to its host."""

import asyncio

import pytest

from jumbo_worker.blobs import BlobStore
from jumbo_worker.events import EventBus
from jumbo_worker.jobs import JobContext, JobError, JobRunner
from jumbo_worker.pool.cpu import CpuPool
from jumbo_worker.pool.manager import ResourceManager
from jumbo_worker.pool.specs import DEFAULT_SPECS, PoolConfig, detect_tpu_chips, load_specs

CONFIG = PoolConfig.model_validate(
    {"tick_s": 0.02, "models": {"det": {"backend": "fake", "chips": 1, "hbm_gb_per_chip": 3, "phase": "vision", "max_replicas": 2, "wake_s": 0}}}
)


async def test_a_job_handler_calls_a_model_and_pool_events_reach_the_bus(tmp_path):
    bus = EventBus()
    pool = ResourceManager(CONFIG, cpu=CpuPool(4, 16), publish=bus.publish, log_dir=tmp_path / "replicas")
    pool.start()

    async def detect(ctx: JobContext) -> str:
        results = await asyncio.gather(*(ctx.call("det", "page", {"echo": i}) for i in range(3)))
        async with ctx.lease("det") as replica:
            again = await replica.run("page", {"echo": "held"})
        return ctx.store.put(repr(([r["echo"] for r in results], again["echo"])).encode())

    store = BlobStore(tmp_path / "blobs")
    runner = JobRunner(store, bus, {"detect": detect}, pool=pool)
    job = runner.submit("detect", store.put(b"pdf"), {})
    await asyncio.wait_for(job.task, 30)
    await pool.close()
    assert job.status == "done", job.error
    assert store.path(job.output).read_bytes() == b"([0, 1, 2], 'held')"
    types = {e["type"] for e in bus.history}
    assert {"pool.phase", "pool.replica_ready", "pool.status"} <= types


async def test_jobs_without_a_pool_get_a_clear_error(tmp_path):
    store = BlobStore(tmp_path)
    ctx = JobContext("j", "x", {}, store, lambda f, m: None)
    with pytest.raises(JobError, match="no model pool"):
        await ctx.call("det", "page", {})


def test_tpu_chips_are_counted_from_device_nodes(tmp_path):
    assert detect_tpu_chips(tmp_path) == 0
    (tmp_path / "vfio").mkdir()
    for n in ["vfio", *map(str, range(8))]:
        (tmp_path / "vfio" / n).touch()
    assert detect_tpu_chips(tmp_path) == 8
    for n in range(4):
        (tmp_path / f"accel{n}").touch()
    assert detect_tpu_chips(tmp_path) == 4


def test_a_host_without_a_tpu_still_loads_every_model():
    config = load_specs(DEFAULT_SPECS, chips=0)
    assert config.spec("qwen25vl_72b_int8").chips == 8
    with pytest.raises(ValueError, match="needs 8 chips, the host has 4"):
        load_specs(DEFAULT_SPECS, chips=4)


def test_cuda_gpus_are_counted_from_device_nodes(tmp_path):
    from jumbo_worker.pool.specs import detect_cuda_gpus

    for name in ("nvidia0", "nvidia1", "nvidiactl", "nvidia-uvm"):
        (tmp_path / name).touch()
    assert detect_cuda_gpus(tmp_path) == 2


async def test_on_a_gpu_host_accelerator_models_share_the_cards_and_cpu_stages_get_none():
    from jumbo_worker.pool.replica import replica_env

    cfg = PoolConfig.model_validate(
        {
            "chips": 0,
            "gpus": 2,
            "tick_s": 0.02,
            "models": {
                "det": {"backend": "fake", "chips": 1, "hbm_gb_per_chip": 3, "phase": "vision", "cpu_fallback": True, "cpu_fallback_cores": 3, "max_replicas": 3, "target_queue_per_replica": 1, "wake_s": 0},
                "render": {"backend": "fake", "cpu_cores": 1, "max_replicas": 1, "wake_s": 0},
            },
        }
    )
    placed = []

    async def spawn(spec, chips, cpu, weights=None, gpu=None):
        placed.append((spec.name, gpu, cpu.cores, replica_env(spec, cfg, chips, cpu, gpu=gpu)["CUDA_VISIBLE_DEVICES"]))

        class Handle:
            async def run(self, kind, payload):
                await asyncio.sleep(0.05)
                return gpu

            async def memory(self):
                return {}

            async def stop(self):
                pass

            async def kill(self):
                pass

        return Handle()

    pool = ResourceManager(cfg, cpu=CpuPool(8, 32), spawn=spawn)
    pool.start()
    gpus = await asyncio.gather(*(pool.run("det", "x", {}) for _ in range(6)), pool.run("render", "x", {}))
    await pool.close()
    dets = [p for p in placed if p[0] == "det"]
    # Spread over both cards, each replica pinned to one; with a GPU they lease their normal cpu_cores, not the CPU-fallback 3.
    assert {g for _, g, _, _ in dets} == {0, 1} and all(env == str(g) and cores == 1 for _, g, cores, env in dets)
    assert ("render", None, 1, "") in placed
    assert set(gpus[:6]) <= {0, 1}
