import asyncio

import pytest

from jumbo_worker.pool.chips import ChipLedger
from jumbo_worker.pool.cpu import CpuLease
from jumbo_worker.pool.replica import Replica, ReplicaError, replica_env
from jumbo_worker.pool.specs import PoolConfig

CONFIG = PoolConfig.model_validate(
    {
        "hbm_ceiling": 0.9,
        "models": {
            "detector": {"backend": "fake", "chips": 2, "hbm_gb_per_chip": 5.0, "phase": "vision"},
            "renderer": {"backend": "fake", "cpu_cores": 3},
            "unbuilt": {"backend": "jax", "chips": 1, "hbm_gb_per_chip": 1.0, "phase": "vision"},
        },
    }
)


def test_env_binds_a_partial_lease_and_caps_the_allocator():
    lease = ChipLedger(8, CONFIG.chip_budget_gb).lease("d", 2, 5.0)
    env = replica_env(CONFIG.spec("detector"), CONFIG, lease, CpuLease("d", 2, 4))
    assert env["TPU_VISIBLE_CHIPS"] == "0,1"
    assert env["TPU_CHIPS_PER_PROCESS_BOUNDS"] == "1,2,1"
    assert env["TPU_PROCESS_PORT"] == "8476"
    assert env["XLA_PYTHON_CLIENT_MEM_FRACTION"] == "0.90"
    assert env["JUMBO_GPU_MEMORY_UTILIZATION"] == "0.90"
    assert env["OMP_NUM_THREADS"] == "2"


def test_env_for_a_whole_host_lease_leaves_libtpu_defaults():
    lease = ChipLedger(8, CONFIG.chip_budget_gb).lease("q", 8, 12.0)
    env = replica_env(CONFIG.spec("detector"), CONFIG, lease, CpuLease("q", 8, 90))
    assert "TPU_VISIBLE_CHIPS" not in env and env["JUMBO_CHIPS"] == "0,1,2,3,4,5,6,7"


def test_cpu_replicas_are_kept_off_the_tpu():
    env = replica_env(CONFIG.spec("renderer"), CONFIG, None, CpuLease("r", 3, 4))
    assert env["JAX_PLATFORMS"] == "cpu" and env["TPU_VISIBLE_CHIPS"] == ""


async def start(name: str, tmp_path, chips=None) -> Replica:
    spec = CONFIG.spec(name)
    cpu = CpuLease(name, 1, 1)
    replica = Replica(spec, chips, cpu, replica_env(spec, CONFIG, chips, cpu), log_dir=tmp_path)
    await replica.start(timeout=20)
    return replica


async def test_a_replica_serves_concurrent_requests_and_reports_memory(tmp_path):
    chips = ChipLedger(8, CONFIG.chip_budget_gb).lease("d", 2, 5.0)
    replica = await start("detector", tmp_path, chips)
    try:
        results = await asyncio.gather(*(replica.run("detect", {"echo": i, "sleep_s": 0.2}) for i in range(5)))
        assert [r["echo"] for r in results] == list(range(5))
        assert results[0]["chips"] == [0, 1]
        assert await replica.memory() == {0: 5.0, 1: 5.0}
        # Library output on stdout goes to the log, not into the protocol.
        assert (await replica.run("detect", {"print": "{noise", "echo": "ok"}))["echo"] == "ok"
        with pytest.raises(ReplicaError, match="RuntimeError: boom"):
            await replica.run("detect", {"fail": "boom"})
        assert replica.state == "ready"
    finally:
        await replica.stop()
    assert replica.state == "dead"
    assert "{noise" in next(tmp_path.glob("detector-*.log")).read_text()


async def test_killing_a_replica_fails_its_inflight_work(tmp_path):
    replica = await start("renderer", tmp_path)
    job = asyncio.create_task(replica.run("render", {"sleep_s": 30}))
    await asyncio.sleep(0.2)
    await replica.kill()
    with pytest.raises(ReplicaError, match="exited"):
        await job
    with pytest.raises(ReplicaError, match="not running"):
        await replica.run("render", {})


async def test_a_model_that_cannot_load_fails_start(tmp_path):
    with pytest.raises(ReplicaError, match="jax runner is added with its pipeline stage"):
        await start("unbuilt", tmp_path)
