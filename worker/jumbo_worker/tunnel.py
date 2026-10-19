"""Keeps a public URL pointing at the local API: a supervised cloudflared quick tunnel on Kaggle, or
the local address itself in development."""

from __future__ import annotations

import asyncio
import logging
import re
from typing import Awaitable, Callable

import httpx

log = logging.getLogger("jumbo.tunnel")

URL_RE = re.compile(r"https://[a-z0-9-]+\.trycloudflare\.com")

OnUrl = Callable[[str], Awaitable[None]]


async def wait_healthy(url: str, timeout_s: float = 90, client: httpx.AsyncClient | None = None) -> bool:
    """Polls `<url>/health` until it answers; a fresh quick tunnel takes a few seconds to resolve."""
    own = client is None
    client = client or httpx.AsyncClient(timeout=10)
    deadline = asyncio.get_running_loop().time() + timeout_s
    try:
        while asyncio.get_running_loop().time() < deadline:
            try:
                r = await client.get(f"{url}/health")
                if r.status_code == 200 and r.json().get("ok"):
                    return True
            except (httpx.HTTPError, ValueError):
                pass
            await asyncio.sleep(2)
        return False
    finally:
        if own:
            await client.aclose()


class DirectTunnel:
    """Development mode: the local address is the public address."""

    def __init__(self, local_url: str, on_url: OnUrl) -> None:
        self.local_url = local_url
        self.on_url = on_url
        self._stopped = asyncio.Event()

    async def run(self) -> None:
        if await wait_healthy(self.local_url, timeout_s=30):
            await self.on_url(self.local_url)
        await self._stopped.wait()

    def stop(self) -> None:
        self._stopped.set()


class CloudflaredTunnel:
    """Runs a cloudflared quick tunnel and restarts it if it dies. Every new URL, once reachable,
    is reported through `on_url` so the caller can publish a new rendezvous epoch."""

    def __init__(
        self,
        local_url: str,
        on_url: OnUrl,
        command: list[str] | None = None,
        healthy: Callable[[str], Awaitable[bool]] = wait_healthy,
        max_restarts: int = 20,
    ) -> None:
        self.command = command or ["cloudflared", "tunnel", "--no-autoupdate", "--url", local_url]
        self.on_url = on_url
        self.healthy = healthy
        self.max_restarts = max_restarts
        self._proc: asyncio.subprocess.Process | None = None
        self._stopping = False

    async def _run_once(self) -> None:
        self._proc = await asyncio.create_subprocess_exec(
            *self.command, stdout=asyncio.subprocess.DEVNULL, stderr=asyncio.subprocess.PIPE
        )
        assert self._proc.stderr is not None
        announced = False
        async for raw in self._proc.stderr:
            line = raw.decode(errors="replace")
            match = URL_RE.search(line)
            if match and not announced:
                announced = True
                url = match.group(0)
                if await self.healthy(url):
                    await self.on_url(url)
                else:
                    log.warning("tunnel %s never became reachable; restarting", url)
                    self._proc.terminate()
        await self._proc.wait()

    async def run(self) -> None:
        delay = 2.0
        for _ in range(self.max_restarts + 1):
            await self._run_once()
            if self._stopping:
                return
            log.warning("cloudflared exited; restarting in %.0fs", delay)
            await asyncio.sleep(delay)
            delay = min(delay * 2, 30)
        raise RuntimeError("cloudflared kept exiting; giving up")

    def stop(self) -> None:
        self._stopping = True
        if self._proc and self._proc.returncode is None:
            self._proc.terminate()
