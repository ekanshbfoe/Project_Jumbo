import base64
import json
import time

import pytest
from fastapi.testclient import TestClient
from nacl.public import PrivateKey

from jumbo_worker.app import create_app
from jumbo_worker.blobs import BlobStore, hash_bytes
from jumbo_worker.crypto import derive_session_key, sign_request
from jumbo_worker.events import EventBus
from jumbo_worker.jobs import JobRunner
from jumbo_worker.session import Session, SessionTicket

SID = "0123456789abcdef0123456789abcdef"


class Signer:
    """What the desktop bridge does: derive K from its own key and the worker's public key, then sign."""

    def __init__(self, app_sk: PrivateKey, session: Session):
        self.key = derive_session_key(app_sk, session.worker_sk.public_key, SID)

    def headers(self, method: str, path: str, body: bytes = b"", ts: int | None = None) -> dict:
        ts = int(time.time()) if ts is None else ts
        return {"Authorization": f"Jumbo {SID}.{ts}.{sign_request(self.key, method, path, ts, body)}"}


@pytest.fixture
def world(tmp_path):
    app_sk = PrivateKey.generate()
    ticket = SessionTicket.from_dict({"proto": 1, "session_id": SID, "app_pk": base64.b64encode(bytes(app_sk.public_key)).decode()})
    session = Session(ticket)
    bus = EventBus()
    store = BlobStore(tmp_path)
    stops: list[str] = []
    app = create_app(session, store, bus, JobRunner(store, bus), on_stop=stops.append)
    with TestClient(app) as client:
        yield client, Signer(app_sk, session), session, stops


def test_health_is_open_but_everything_else_needs_a_signature(world):
    client, signer, _, _ = world
    assert client.get("/health").json() == {"ok": True, "proto": 1}
    assert client.get("/v1/session").status_code == 401
    stale = signer.headers("GET", "/v1/session", ts=int(time.time()) - 120)
    assert client.get("/v1/session", headers=stale).status_code == 401
    wrong_path = signer.headers("GET", "/v1/jobs/x")
    assert client.get("/v1/session", headers=wrong_path).status_code == 401
    assert client.get("/v1/session", headers=signer.headers("GET", "/v1/session")).json()["session_id"] == SID


def test_upload_echo_download_round_trip_with_events(world):
    client, signer, _, _ = world
    data = b"panel bytes " * 50_000
    name = hash_bytes(data)

    body = json.dumps({"hashes": [name]}).encode()
    missing = client.post("/v1/blobs/missing", content=body, headers=signer.headers("POST", "/v1/blobs/missing", body))
    assert missing.json() == {"missing": [name]}

    half = len(data) // 2
    for offset, chunk in ((0, data[:half]), (half, data[half:])):
        path = f"/v1/blobs/{name}?offset={offset}&total={len(data)}"
        r = client.put(path, content=chunk, headers=signer.headers("PUT", path, chunk))
        assert r.status_code == 200, r.text
    assert r.json() == {"received": len(data), "complete": True}

    with client.websocket_connect("/v1/events", headers=signer.headers("GET", "/v1/events")) as ws:
        assert ws.receive_json()["type"] == "hello"
        job_body = json.dumps({"kind": "echo", "input": name, "params": {"step_s": 0.01}}).encode()
        job_id = client.post("/v1/jobs", content=job_body, headers=signer.headers("POST", "/v1/jobs", job_body)).json()["job_id"]
        seen = []
        while not seen or seen[-1]["type"] != "job.done":
            seen.append(ws.receive_json())
    assert [e["type"] for e in seen].count("job.progress") == 5
    output = seen[-1]["output"]
    assert output == name

    # Resubmitting the same job returns the same id (idempotent).
    again = client.post("/v1/jobs", content=job_body, headers=signer.headers("POST", "/v1/jobs", job_body)).json()["job_id"]
    assert again == job_id

    path = f"/v1/blobs/{output}"
    assert client.get(path, headers=signer.headers("GET", path)).content == data
    ranged = client.get(path, headers={**signer.headers("GET", path), "Range": "bytes=10-19"})
    assert ranged.status_code == 206 and ranged.content == data[10:20]


def test_websocket_rejects_unsigned_clients(world):
    client, *_ = world
    with pytest.raises(Exception):
        with client.websocket_connect("/v1/events") as ws:
            ws.receive_json()


def test_stop_drains_and_refuses_new_jobs(world):
    client, signer, session, stops = world
    assert client.post("/v1/session/stop", headers=signer.headers("POST", "/v1/session/stop")).json() == {"state": "draining"}
    assert session.draining and stops == ["stop requested"]
    body = json.dumps({"kind": "echo", "input": "0" * 64}).encode()
    assert client.post("/v1/jobs", content=body, headers=signer.headers("POST", "/v1/jobs", body)).status_code == 409
