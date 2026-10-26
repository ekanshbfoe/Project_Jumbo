"""Session ticket, derived key, request authentication and the idle/deadline clocks."""

from __future__ import annotations

import base64
import re
import time
from dataclasses import dataclass, field
from typing import Callable

from nacl.public import PrivateKey, PublicKey

from .crypto import PROTO, derive_session_key, verify_request

MAX_SKEW_S = 60
_AUTH = re.compile(r"^Jumbo ([0-9a-f]{32})\.(\d{1,12})\.([0-9a-f]{64})$")


class AuthError(Exception):
    pass


@dataclass(frozen=True)
class SessionTicket:
    session_id: str
    app_pk: PublicKey
    deadline_s: int = 30_600
    idle_s: int = 1_800

    @classmethod
    def from_dict(cls, raw: dict) -> "SessionTicket":
        if raw.get("proto") != PROTO:
            raise ValueError(f"unsupported bridge protocol {raw.get('proto')!r}")
        session_id = raw["session_id"]
        if not re.fullmatch(r"[0-9a-f]{32}", session_id):
            raise ValueError("session_id must be 32 lowercase hex characters")
        return cls(
            session_id=session_id,
            app_pk=PublicKey(base64.b64decode(raw["app_pk"])),
            deadline_s=int(raw.get("deadline_s", 30_600)),
            idle_s=int(raw.get("idle_s", 1_800)),
        )


@dataclass
class Session:
    ticket: SessionTicket
    clock: Callable[[], float] = time.time
    worker_sk: PrivateKey = field(default_factory=PrivateKey.generate)
    epoch: int = 0
    draining: bool = False
    # True while models load; the app shows "Warming" and waits until it clears.
    warming: bool = False

    def __post_init__(self) -> None:
        self.key = derive_session_key(self.worker_sk, self.ticket.app_pk, self.ticket.session_id)
        self.started_at = self.clock()
        self.last_seen = self.started_at

    @property
    def worker_pk_b64(self) -> str:
        return base64.b64encode(bytes(self.worker_sk.public_key)).decode()

    def authenticate(self, header: str | None, method: str, path_and_query: str, body: bytes) -> None:
        """Raises AuthError unless the request is signed with this session's key and is fresh."""
        match = _AUTH.match(header or "")
        if not match:
            raise AuthError("missing or malformed Authorization header")
        session_id, ts_text, sig = match.groups()
        if session_id != self.ticket.session_id:
            raise AuthError("unknown session")
        ts = int(ts_text)
        if abs(self.clock() - ts) > MAX_SKEW_S:
            raise AuthError("stale or future timestamp")
        if not verify_request(self.key, method, path_and_query, ts, body, sig):
            raise AuthError("bad signature")
        self.last_seen = self.clock()

    def idle_for(self) -> float:
        return self.clock() - self.last_seen

    def should_stop(self) -> str | None:
        """Why the worker should shut itself down now, if it should."""
        now = self.clock()
        if now - self.started_at >= self.ticket.deadline_s:
            return "deadline reached"
        if now - self.last_seen >= self.ticket.idle_s:
            return "no authenticated requests within the idle limit"
        return None

    def info(self) -> dict:
        return {
            "proto": PROTO,
            "session_id": self.ticket.session_id,
            "epoch": self.epoch,
            "started_at": self.started_at,
            "deadline_at": self.started_at + self.ticket.deadline_s,
            "idle_s": self.ticket.idle_s,
            "draining": self.draining,
            "warming": self.warming,
        }
