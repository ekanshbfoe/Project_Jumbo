"""Regenerates protocol/bridge-v1.vectors.json from fixed keys.

Both implementations test against this file: the Rust bridge must reproduce the shared key and
signatures exactly, and open the sealed record that Python produced.
"""

import base64
import json
from pathlib import Path

from nacl.public import PrivateKey

from jumbo_worker.crypto import derive_session_key, seal_for, sign_request

OUT = Path(__file__).resolve().parents[2] / "protocol" / "bridge-v1.vectors.json"


def b64(data: bytes) -> str:
    return base64.b64encode(data).decode()


def main() -> None:
    app_sk = PrivateKey(bytes(range(32)))
    worker_sk = PrivateKey(bytes(range(32, 64)))
    session_id = "0123456789abcdef0123456789abcdef"
    key = derive_session_key(worker_sk, app_sk.public_key, session_id)
    body = b'{"hashes":["abc"]}'
    sealed_message = json.dumps({"url": "https://example.trycloudflare.com", "boot": {"epoch": 1}}).encode()
    vectors = {
        "proto": 1,
        "session_id": session_id,
        "app_sk": b64(bytes(app_sk)),
        "app_pk": b64(bytes(app_sk.public_key)),
        "worker_sk": b64(bytes(worker_sk)),
        "worker_pk": b64(bytes(worker_sk.public_key)),
        "session_key_hex": key.hex(),
        "signatures": [
            {"method": "POST", "path": "/v1/blobs/missing", "ts": 1760860800, "body": body.decode(),
             "sig": sign_request(key, "POST", "/v1/blobs/missing", 1760860800, body)},
            {"method": "GET", "path": "/v1/blobs/abc?x=1", "ts": 1760860861, "body": "",
             "sig": sign_request(key, "GET", "/v1/blobs/abc?x=1", 1760860861, b"")},
        ],
        "sealed": {"message": sealed_message.decode(), "box": b64(seal_for(app_sk.public_key, sealed_message))},
    }
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(vectors, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(f"wrote {OUT}")


if __name__ == "__main__":
    main()
