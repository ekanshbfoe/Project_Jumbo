import base64
import json
from pathlib import Path

from nacl.public import PrivateKey, SealedBox

from jumbo_worker.crypto import derive_session_key, seal_for, sign_request, verify_request

VECTORS = json.loads((Path(__file__).resolve().parents[2] / "protocol" / "bridge-v1.vectors.json").read_text())


def key_pair(name: str) -> PrivateKey:
    return PrivateKey(base64.b64decode(VECTORS[f"{name}_sk"]))


def test_both_sides_derive_the_same_session_key():
    app, worker = key_pair("app"), key_pair("worker")
    sid = VECTORS["session_id"]
    from_worker = derive_session_key(worker, app.public_key, sid)
    from_app = derive_session_key(app, worker.public_key, sid)
    assert from_worker == from_app
    assert from_worker.hex() == VECTORS["session_key_hex"]


def test_signatures_match_vectors_and_reject_tampering():
    key = bytes.fromhex(VECTORS["session_key_hex"])
    for v in VECTORS["signatures"]:
        body = v["body"].encode()
        assert sign_request(key, v["method"], v["path"], v["ts"], body) == v["sig"]
        assert verify_request(key, v["method"], v["path"], v["ts"], body, v["sig"])
        assert not verify_request(key, v["method"], v["path"], v["ts"] + 1, body, v["sig"])
        assert not verify_request(key, v["method"], v["path"] + "x", v["ts"], body, v["sig"])
        assert not verify_request(key, v["method"], v["path"], v["ts"], body + b" ", v["sig"])


def test_sealed_payload_opens_only_for_the_app():
    app = key_pair("app")
    box = base64.b64decode(VECTORS["sealed"]["box"])
    assert SealedBox(app).decrypt(box).decode() == VECTORS["sealed"]["message"]
    fresh = seal_for(app.public_key, b"hello")
    assert SealedBox(app).decrypt(fresh) == b"hello"


def test_hkdf_matches_rfc5869_case_1():
    from jumbo_worker.crypto import _hkdf_sha256

    okm = _hkdf_sha256(bytes([0x0B] * 22), bytes(range(13)), bytes(range(0xF0, 0xFA)), 42)
    assert okm.hex() == (
        "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
    )
