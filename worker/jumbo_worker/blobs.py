"""Content-addressed blob store with resumable chunked uploads.

Blobs are named by their lowercase-hex blake3. Uploads append in order to `partial/<hash>`; the chunk
that completes a blob is verified against its name before it moves into place, so a blob path either
holds exactly the bytes its name promises or does not exist.
"""

from __future__ import annotations

import os
import re
from dataclasses import dataclass
from pathlib import Path

import blake3

MAX_CHUNK = 32 * 1024 * 1024
_HASH = re.compile(r"^[0-9a-f]{64}$")


class BlobError(Exception):
    """A client error: bad hash, out-of-order chunk or corrupt data."""


@dataclass(frozen=True)
class BlobStatus:
    received: int
    complete: bool


def check_hash(name: str) -> str:
    if not _HASH.match(name):
        raise BlobError(f"{name!r} is not a blake3 hex digest")
    return name


def hash_bytes(data: bytes) -> str:
    return blake3.blake3(data).hexdigest()


def hash_file(path: Path) -> str:
    h = blake3.blake3()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


class BlobStore:
    def __init__(self, root: Path) -> None:
        self.root = root
        self._done = root / "blobs"
        self._partial = root / "partial"
        self._done.mkdir(parents=True, exist_ok=True)
        self._partial.mkdir(parents=True, exist_ok=True)

    def path(self, name: str) -> Path:
        return self._done / check_hash(name)

    def has(self, name: str) -> bool:
        return self.path(name).is_file()

    def missing(self, names: list[str]) -> list[str]:
        return [n for n in names if not self.has(n)]

    def status(self, name: str) -> BlobStatus:
        if self.has(name):
            return BlobStatus(self.path(name).stat().st_size, True)
        partial = self._partial / check_hash(name)
        return BlobStatus(partial.stat().st_size if partial.exists() else 0, False)

    def write_chunk(self, name: str, offset: int, total: int, data: bytes) -> BlobStatus:
        """Appends one chunk. `offset` must equal the bytes already received (resume from `status`)."""
        check_hash(name)
        if self.has(name):
            return self.status(name)
        if len(data) > MAX_CHUNK:
            raise BlobError(f"chunk of {len(data)} bytes exceeds {MAX_CHUNK}")
        if total < 0 or offset + len(data) > total:
            raise BlobError("chunk runs past the declared total size")
        partial = self._partial / name
        received = partial.stat().st_size if partial.exists() else 0
        if offset != received:
            raise BlobError(f"expected offset {received}, got {offset}")
        with partial.open("ab") as f:
            f.write(data)
            f.flush()
            os.fsync(f.fileno())
        received += len(data)
        if received < total:
            return BlobStatus(received, False)
        if hash_file(partial) != name:
            partial.unlink()
            raise BlobError("content does not match its blake3 name; upload discarded")
        os.replace(partial, self.path(name))
        return BlobStatus(received, True)

    def put(self, data: bytes) -> str:
        """Stores bytes produced on the worker (e.g. job outputs) and returns their name."""
        name = hash_bytes(data)
        if not self.has(name):
            tmp = self._partial / f"{name}.local"
            tmp.write_bytes(data)
            os.replace(tmp, self.path(name))
        return name
