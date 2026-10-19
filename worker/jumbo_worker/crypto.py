"""Session keys, request signatures and sealed rendezvous payloads (docs/bridge-protocol.md)."""

from __future__ import annotations

import hashlib
import hmac

import blake3
from nacl.bindings import crypto_scalarmult
from nacl.public import PrivateKey, PublicKey, SealedBox

PROTO = 1
_SALT = b"jumbo-bridge-v1"


def _hkdf_sha256(ikm: bytes, salt: bytes, info: bytes, length: int = 32) -> bytes:
    """RFC 5869 HKDF with SHA-256."""
    prk = hmac.new(salt, ikm, hashlib.sha256).digest()
    out, block, counter = b"", b"", 1
    while len(out) < length:
        block = hmac.new(prk, block + info + bytes([counter]), hashlib.sha256).digest()
        out += block
        counter += 1
    return out[:length]


def derive_session_key(own: PrivateKey, peer: PublicKey, session_id: str) -> bytes:
    """32-byte key both sides derive from their X25519 exchange."""
    shared = crypto_scalarmult(bytes(own), bytes(peer))
    return _hkdf_sha256(shared, _SALT, session_id.encode())


def body_hash(body: bytes) -> str:
    return blake3.blake3(body).hexdigest()


def sign_request(key: bytes, method: str, path_and_query: str, ts: int, body: bytes) -> str:
    message = f"v1\n{method.upper()}\n{path_and_query}\n{ts}\n{body_hash(body)}".encode()
    return hmac.new(key, message, hashlib.sha256).hexdigest()


def verify_request(key: bytes, method: str, path_and_query: str, ts: int, body: bytes, sig: str) -> bool:
    return hmac.compare_digest(sign_request(key, method, path_and_query, ts, body), sig)


def seal_for(recipient: PublicKey, message: bytes) -> bytes:
    """libsodium sealed box: only the holder of the recipient's private key can open it."""
    return SealedBox(recipient).encrypt(message)
