import asyncio
import base64
import json
import sys

import httpx
from nacl.public import PrivateKey, SealedBox

from jumbo_worker.rendezvous import FileRendezvous, GistRendezvous, build_record
from jumbo_worker.session import Session, SessionTicket
from jumbo_worker.tunnel import CloudflaredTunnel

SID = "0123456789abcdef0123456789abcdef"


def make_session() -> tuple[PrivateKey, Session]:
    app_sk = PrivateKey.generate()
    ticket = SessionTicket.from_dict({"proto": 1, "session_id": SID, "app_pk": base64.b64encode(bytes(app_sk.public_key)).decode()})
    return app_sk, Session(ticket, epoch=3)


def test_record_reveals_the_url_only_to_the_app():
    app_sk, session = make_session()
    record = build_record(session, "https://a-b.trycloudflare.com", {"cpus": 8})
    assert record["epoch"] == 3 and record["worker_pk"] == session.worker_pk_b64
    assert "trycloudflare" not in json.dumps(record)
    opened = json.loads(SealedBox(app_sk).decrypt(base64.b64decode(record["sealed"])))
    assert opened == {"url": "https://a-b.trycloudflare.com", "boot": {"cpus": 8}}


async def test_gist_publish_and_remove_use_the_gist_api():
    calls = []

    def handler(request: httpx.Request) -> httpx.Response:
        calls.append((request.method, str(request.url), request.headers["authorization"], json.loads(request.content)))
        return httpx.Response(200, json={})

    gist = GistRendezvous("abc123", "tok", client=httpx.AsyncClient(transport=httpx.MockTransport(handler)))
    _, session = make_session()
    await gist.publish(build_record(session, "https://x.trycloudflare.com", {}))
    await gist.remove(SID)
    assert [c[:3] for c in calls] == [("PATCH", "https://api.github.com/gists/abc123", "Bearer tok")] * 2
    assert f"session-{SID}.json" in calls[0][3]["files"]
    assert calls[1][3] == {"files": {f"session-{SID}.json": None}}


async def test_gist_retries_server_errors():
    attempts = []

    def handler(request: httpx.Request) -> httpx.Response:
        attempts.append(1)
        return httpx.Response(502 if len(attempts) < 2 else 200, json={})

    gist = GistRendezvous("g", "t", client=httpx.AsyncClient(transport=httpx.MockTransport(handler)))
    await gist.remove(SID)
    assert len(attempts) == 2


async def test_file_rendezvous_round_trip(tmp_path):
    rv = FileRendezvous(tmp_path)
    await rv.publish({"session_id": SID, "epoch": 1})
    assert json.loads((tmp_path / f"session-{SID}.json").read_text())["epoch"] == 1
    await rv.remove(SID)
    assert not list(tmp_path.iterdir())


async def test_cloudflared_supervisor_announces_each_new_tunnel_and_restarts():
    # A stand-in for cloudflared that logs a URL to stderr and exits, like a dropped tunnel.
    fake = [sys.executable, "-c", "import sys; print('INF |  https://quiet-river-12.trycloudflare.com  |', file=sys.stderr)"]
    announced: list[str] = []

    async def healthy(url: str) -> bool:
        return True

    tunnel = CloudflaredTunnel("http://127.0.0.1:1", lambda url: _record(announced, url), command=fake, healthy=healthy, max_restarts=1)
    tunnel_task = asyncio.create_task(tunnel.run())
    await asyncio.wait_for(_until(lambda: len(announced) == 2), timeout=20)
    tunnel.stop()
    tunnel_task.cancel()
    assert announced == ["https://quiet-river-12.trycloudflare.com"] * 2


async def _record(sink: list, url: str) -> None:
    sink.append(url)


async def _until(cond) -> None:
    while not cond():
        await asyncio.sleep(0.05)
