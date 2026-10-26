"""Isolated Python environments for models whose dependencies clash with the worker's own.

Magi's remote code needs transformers 4.46; a model that needs a newer one cannot share its
interpreter. Every replica is its own process anyway, so a model that lists `pip` requirements in its
spec runs in a virtualenv of its own. The env sees the host's packages (torch, CUDA, numpy are
preinstalled on Kaggle and reused, not downloaded again) and installs only its requirements on top,
which take precedence. It is built on the first wake and reused while the requirements match.
"""

from __future__ import annotations

import asyncio
import os
import sys
from pathlib import Path
from typing import Awaitable, Callable

Run = Callable[[list[str]], Awaitable[None]]

_locks: dict[str, asyncio.Lock] = {}


def envs_root() -> Path:
    return Path(os.environ.get("JUMBO_ENVS", Path.home() / ".cache" / "jumbo" / "envs"))


def env_python(env: Path) -> Path:
    return env / ("Scripts/python.exe" if sys.platform == "win32" else "bin/python")


async def _run(cmd: list[str]) -> None:
    proc = await asyncio.create_subprocess_exec(*cmd, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.STDOUT)
    out, _ = await proc.communicate()
    if proc.returncode:
        tail = out.decode(errors="replace").strip().splitlines()[-8:]
        raise RuntimeError(f"{' '.join(cmd[:4])} … failed: " + " | ".join(tail))


async def ensure_env(name: str, requirements: list[str], root: Path | None = None, run: Run = _run) -> str:
    """Path of the interpreter for `name`'s env, building it if it is missing or out of date."""
    env = (root or envs_root()) / name
    marker = env / ".jumbo-requirements"
    want = "\n".join(sorted(requirements))
    lock = _locks.setdefault(str(env), asyncio.Lock())
    async with lock:  # replicas of one model waking together build the env once
        if marker.is_file() and marker.read_text() == want:
            return str(env_python(env))
        # No pip inside the env (some hosts lack ensurepip): the host's pip installs into it.
        await run([sys.executable, "-m", "venv", "--system-site-packages", "--without-pip", "--clear", str(env)])
        await run([sys.executable, "-m", "pip", "--python", str(env_python(env)), "install", "-q", "--disable-pip-version-check", *requirements])
        env.mkdir(parents=True, exist_ok=True)
        marker.write_text(want)
        return str(env_python(env))
