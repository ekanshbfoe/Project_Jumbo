import asyncio

import pytest

from jumbo_worker.pool.cpu import CpuPool
from jumbo_worker.pool.manager import ResourceManager
from jumbo_worker.pool.specs import PoolConfig
from jumbo_worker.pool.weights import WeightCache, compile_cache_env


def weights(root, name: str, mb: int) -> None:
    (root / name).mkdir(parents=True)
    (root / name / "model.safetensors").write_bytes(b"\0" * mb * 1_000_000)


async def test_weights_are_staged_once_and_evicted_lru_unless_pinned(tmp_path):
    src, shm = tmp_path / "input", tmp_path / "shm"
    for name in ("a", "b", "c"):
        weights(src, name, 4)
    cache = WeightCache(src, shm, budget_gb=0.009)
    staged = await cache.ensure("a")
    assert (staged / "model.safetensors").stat().st_size == 4_000_000
    assert await cache.ensure("a") == staged  # second call is a hit
    cache.pin("a")
    await cache.ensure("b")
    # c needs room: b goes (a is pinned even though it is older).
    await cache.ensure("c")
    assert sorted(cache.cached()) == ["a", "c"]
    cache.pin("c")
    with pytest.raises(MemoryError, match="full of pinned"):
        await cache.ensure("b")
    with pytest.raises(FileNotFoundError):
        await cache.ensure("missing")
    with pytest.raises(MemoryError, match="exceed the RAM cache budget"):
        weights(src, "huge", 10)
        await cache.ensure("huge")


async def test_concurrent_wakes_stage_weights_once(tmp_path):
    weights(tmp_path / "input", "m", 2)
    cache = WeightCache(tmp_path / "input", tmp_path / "shm", budget_gb=1)
    paths = await asyncio.gather(*(cache.ensure("m") for _ in range(5)))
    assert len(set(paths)) == 1 and list((tmp_path / "shm").iterdir()) == [paths[0]]


def test_compile_caches_share_one_folder(tmp_path):
    env = compile_cache_env(tmp_path)
    assert env["JAX_COMPILATION_CACHE_DIR"] == str(tmp_path / "jax")
    assert env["VLLM_XLA_CACHE_PATH"] == str(tmp_path / "vllm")


async def test_replicas_get_their_weights_from_ram_and_unpin_on_sleep(tmp_path):
    weights(tmp_path / "input", "w", 1)
    cache = WeightCache(tmp_path / "input", tmp_path / "shm", budget_gb=1)
    cfg = PoolConfig.model_validate(
        {"tick_s": 0.02, "models": {"m": {"backend": "fake", "chips": 1, "hbm_gb_per_chip": 2, "phase": "p", "weights": "w", "idle_ttl_s": 0.1, "wake_s": 0}}}
    )
    seen = []

    async def spawn(spec, chips, cpu, staged, gpu=None):
        seen.append(staged)

        class Handle:
            async def run(self, kind, payload):
                return "ok"

            async def memory(self):
                return {}

            async def stop(self):
                pass

            async def kill(self):
                pass

        return Handle()

    manager = ResourceManager(cfg, cpu=CpuPool(4, 8), spawn=spawn, weights=cache, compile_cache=tmp_path / "xla")
    manager.start()
    assert await manager.run("m", "x", {}) == "ok"
    assert seen == [tmp_path / "shm" / "w"] and cache.pins["w"] == 1
    assert manager.extra_env["JAX_COMPILATION_CACHE_DIR"] == str(tmp_path / "xla" / "jax")
    for _ in range(50):
        if not manager.slots:
            break
        await asyncio.sleep(0.02)
    assert manager.slots == {} and cache.pins["w"] == 0
    assert "w" in cache.cached()  # asleep, but still in RAM for the next wake
    await manager.close()
