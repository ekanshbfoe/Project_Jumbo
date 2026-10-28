import pytest

from jumbo_worker.pool.chips import ChipLedger, LeaseError
from jumbo_worker.pool.cpu import CpuPool, host_ram_gb
from jumbo_worker.pool.specs import DEFAULT_SPECS, PoolConfig, load_specs


def test_the_shipped_registry_loads_at_both_ceilings():
    for ceiling in (0.80, 0.90):
        config = load_specs(DEFAULT_SPECS, hbm_ceiling=ceiling)
        assert config.chip_budget_gb == pytest.approx(16 * ceiling)
        assert config.soft_limit == pytest.approx(ceiling - 0.04)
        assert config.spec("qwen3vl_32b_tpu").chips == 8
        assert config.spec("remotion").on_tpu is False


def test_admission_rejects_a_model_over_the_per_chip_budget():
    with pytest.raises(ValueError, match="exceeds the 12.8 GB budget"):
        PoolConfig.model_validate(
            {"hbm_ceiling": 0.8, "models": {"big": {"backend": "jax", "chips": 1, "hbm_gb_per_chip": 13.0, "phase": "p"}}}
        )
    # The same model fits once spike 6 raises the ceiling to 90%.
    PoolConfig.model_validate(
        {"hbm_ceiling": 0.9, "models": {"big": {"backend": "jax", "chips": 1, "hbm_gb_per_chip": 13.0, "phase": "p"}}}
    )


@pytest.mark.parametrize(
    ("spec", "message"),
    [
        ({"backend": "jax", "chips": 3, "phase": "p"}, "chips must be one of"),
        ({"backend": "jax", "chips": 1}, "need a phase"),
        ({"backend": "cpu", "min_replicas": 2, "max_replicas": 1}, "min_replicas"),
    ],
)
def test_bad_specs_are_rejected(spec, message):
    with pytest.raises(ValueError, match=message):
        PoolConfig.model_validate({"models": {"m": spec}})


def test_leases_are_exclusive_aligned_slices():
    ledger = ChipLedger(8, 12.8)
    a = ledger.lease("a", 1, 5.0)
    b = ledger.lease("b", 2, 5.0)
    assert len(set(a.chips) | set(b.chips)) == 3
    assert b.chips[0] % 2 == 0
    with pytest.raises(LeaseError, match="no free aligned block of 8"):
        ledger.lease("q", 8, 12.5)
    ledger.release(a)
    ledger.release(b)
    assert ledger.lease("q", 8, 12.5).chips == tuple(range(8))


def test_best_fit_keeps_large_blocks_free():
    ledger = ChipLedger(8, 12.8)
    leases = [ledger.lease(f"m{i}", 1, 4.0) for i in range(3)]
    # Single-chip replicas pack into one half, so a 4-chip lease still fits in the other.
    assert max(c for l in leases for c in l.chips) < 4
    assert ledger.lease("big", 4, 10.0).chips == (4, 5, 6, 7)


def test_ledger_refuses_hbm_over_budget_and_unknown_sizes():
    ledger = ChipLedger(8, 12.8)
    with pytest.raises(LeaseError, match="over the 12.8 GB budget"):
        ledger.lease("x", 1, 13.0)
    with pytest.raises(LeaseError, match="cannot lease 3"):
        ledger.lease("x", 3, 1.0)


def test_cpu_pool_accounts_cores_and_ram():
    pool = CpuPool(cores=8, ram_gb=32)
    lease = pool.lease("render", 4, 8)
    assert pool.can_lease(4, 24) and not pool.can_lease(5, 1)
    with pytest.raises(LeaseError, match="already holds"):
        pool.lease("render", 1, 1)
    pool.release(lease)
    assert pool.used() == (0, 0)


def test_host_ram_is_read_from_meminfo(tmp_path):
    meminfo = tmp_path / "meminfo"
    meminfo.write_text("MemTotal:       346030080 kB\nMemFree: 1 kB\n")
    assert host_ram_gb(meminfo) == pytest.approx(330, abs=0.1)
    assert host_ram_gb(tmp_path / "missing") == 0.0
    assert CpuPool.detect().cores >= 1
