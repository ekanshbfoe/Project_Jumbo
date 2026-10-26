"""Models whose dependencies clash with the worker's run in isolated envs, built once per requirement set."""

from jumbo_worker.pool.envs import ensure_env


async def test_a_model_env_is_built_once_and_rebuilt_when_its_requirements_change(tmp_path):
    commands = []

    async def run(cmd):
        commands.append(cmd)

    py = await ensure_env("tts", ["somepkg==0.2.1"], tmp_path, run)
    assert py.endswith("bin/python") and len(commands) == 2
    assert "--system-site-packages" in commands[0] and commands[1][-1] == "somepkg==0.2.1"
    await ensure_env("tts", ["somepkg==0.2.1"], tmp_path, run)
    assert len(commands) == 2
    await ensure_env("tts", ["somepkg==0.2.2"], tmp_path, run)
    assert len(commands) == 4
