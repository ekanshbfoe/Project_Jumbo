"""Publishing the sealed rendezvous record so the desktop app can find the tunnel."""

from __future__ import annotations

import asyncio
import base64
import json
import os
import time
from pathlib import Path
from typing import Protocol

import httpx

from .crypto import PROTO, seal_for
from .session import Session


def build_record(session: Session, url: str, boot: dict) -> dict:
    """The record for the current epoch. Only the app's private key can open `sealed`."""
    sealed = seal_for(session.ticket.app_pk, json.dumps({"url": url, "boot": boot}).encode())
    return {
        "proto": PROTO,
        "session_id": session.ticket.session_id,
        "epoch": session.epoch,
        "worker_pk": session.worker_pk_b64,
        "sealed": base64.b64encode(sealed).decode(),
        "ts": int(time.time()),
    }


def record_name(session_id: str) -> str:
    return f"session-{session_id}.json"


class Rendezvous(Protocol):
    async def publish(self, record: dict) -> None: ...

    async def remove(self, session_id: str) -> None: ...


class GistRendezvous:
    """Writes the record as a file in a private GitHub gist (token needs only the `gist` scope)."""

    def __init__(self, gist_id: str, token: str, client: httpx.AsyncClient | None = None, retries: int = 4) -> None:
        self.url = f"https://api.github.com/gists/{gist_id}"
        self.headers = {
            "Authorization": f"Bearer {token}",
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
        }
        self.client = client or httpx.AsyncClient(timeout=20)
        self.retries = retries

    async def _patch(self, files: dict) -> None:
        delay = 1.0
        for attempt in range(self.retries):
            try:
                r = await self.client.patch(self.url, headers=self.headers, json={"files": files})
                if r.status_code < 500:
                    r.raise_for_status()
                    return
            except httpx.TransportError:
                if attempt == self.retries - 1:
                    raise
            await asyncio.sleep(delay)
            delay *= 2
        raise RuntimeError("GitHub kept failing to update the rendezvous gist")

    async def publish(self, record: dict) -> None:
        await self._patch({record_name(record["session_id"]): {"content": json.dumps(record)}})

    async def remove(self, session_id: str) -> None:
        # Setting a gist file to null deletes it.
        await self._patch({record_name(session_id): None})


class FileRendezvous:
    """Local stand-in for development and tests: records are files in a folder."""

    def __init__(self, folder: Path) -> None:
        self.folder = folder
        folder.mkdir(parents=True, exist_ok=True)

    async def publish(self, record: dict) -> None:
        target = self.folder / record_name(record["session_id"])
        tmp = target.with_suffix(".tmp")
        tmp.write_text(json.dumps(record), encoding="utf-8")
        os.replace(tmp, target)

    async def remove(self, session_id: str) -> None:
        (self.folder / record_name(session_id)).unlink(missing_ok=True)
