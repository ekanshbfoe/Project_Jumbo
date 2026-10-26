import pytest

from jumbo_worker import __main__ as worker


def test_reads_the_token_from_either_mount_layout(tmp_path):
    assert worker.mounted_secret("gist_token", tmp_path) is None
    nested = tmp_path / "datasets" / "alice" / "jumbo-secrets"
    nested.mkdir(parents=True)
    (nested / "gist_token").write_text("ghp_nested\n")
    assert worker.mounted_secret("gist_token", tmp_path) == "ghp_nested"
    flat = tmp_path / "jumbo-secrets"
    flat.mkdir()
    (flat / "gist_token").write_text("ghp_flat")
    assert worker.mounted_secret("gist_token", tmp_path) == "ghp_flat"


def test_environment_wins_over_the_mounted_dataset(monkeypatch):
    monkeypatch.setattr(worker, "mounted_secret", lambda name: "ghp_mounted")
    monkeypatch.setenv("JUMBO_GIST_TOKEN", "ghp_env")
    assert worker.gist_token() == "ghp_env"
    monkeypatch.delenv("JUMBO_GIST_TOKEN")
    assert worker.gist_token() == "ghp_mounted"


def test_missing_everywhere_fails_with_a_clear_message(monkeypatch):
    monkeypatch.delenv("JUMBO_GIST_TOKEN", raising=False)
    monkeypatch.setattr(worker, "mounted_secret", lambda name: None)
    with pytest.raises(SystemExit, match="jumbo-secrets dataset is not attached"):
        worker.gist_token()
