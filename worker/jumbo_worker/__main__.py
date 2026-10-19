"""Worker entry point: `python -m jumbo_worker --ticket ticket.json`.

Starts the API on localhost, exposes it (cloudflared on Kaggle, directly in development), publishes a
sealed rendezvous record for every new tunnel URL, and shuts down cleanly on a stop request, the idle
limit or the session deadline.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import logging
import os
from pathlib import Path

import uvicorn

from . import __version__
from .app import create_app
from .blobs import BlobStore
from .events import EventBus
from .jobs import JobRunner
from .rendezvous import FileRendezvous, GistRendezvous, Rendezvous, build_record
from .session import Session, SessionTicket
from .tunnel import CloudflaredTunnel, DirectTunnel

log = logging.getLogger("jumbo.worker")


def host_facts() -> dict:
    facts: dict = {"cpus": os.cpu_count()}
    try:
        meminfo = Path("/proc/meminfo").read_text()
        kb = int(next(line for line in meminfo.splitlines() if line.startswith("MemTotal")).split()[1])
        facts["ram_gb"] = round(kb / 1024 / 1024, 1)
    except (OSError, StopIteration, ValueError):
        pass
    facts["tpu"] = Path("/dev/accel0").exists() or bool(os.environ.get("TPU_NAME"))
    return facts


def gist_token() -> str:
    """The gist token comes from the environment, or from Kaggle Secrets when running on Kaggle."""
    token = os.environ.get("JUMBO_GIST_TOKEN")
    if token:
        return token
    try:
        from kaggle_secrets import UserSecretsClient  # type: ignore[import-not-found]

        return UserSecretsClient().get_secret("JUMBO_GIST_TOKEN")
    except Exception as e:  # noqa: BLE001
        raise SystemExit(
            "No JUMBO_GIST_TOKEN: attach the Kaggle Secret JUMBO_GIST_TOKEN to this notebook (Add-ons > Secrets)"
        ) from e


def make_rendezvous(ticket_raw: dict, override: str | None) -> Rendezvous:
    spec = override or ticket_raw.get("rendezvous", {}).get("kind", "gist")
    if spec.startswith("file:"):
        return FileRendezvous(Path(spec[len("file:"):]))
    gist_id = ticket_raw.get("rendezvous", {}).get("gist_id")
    if not gist_id:
        raise SystemExit("ticket has no rendezvous.gist_id")
    return GistRendezvous(gist_id, gist_token())


async def serve(args: argparse.Namespace) -> None:
    raw = json.loads(Path(args.ticket).read_text(encoding="utf-8"))
    session = Session(SessionTicket.from_dict(raw))
    bus = EventBus()
    store = BlobStore(Path(args.data))
    runner = JobRunner(store, bus)
    rendezvous = make_rendezvous(raw, args.rendezvous)
    stop = asyncio.Event()

    def request_stop(reason: str) -> None:
        log.info("stopping: %s", reason)
        stop.set()

    app = create_app(session, store, bus, runner, on_stop=request_stop)
    server = uvicorn.Server(uvicorn.Config(app, host="127.0.0.1", port=args.port, log_level="warning", ws="websockets-sansio", timeout_graceful_shutdown=10))
    local_url = f"http://127.0.0.1:{args.port}"

    async def publish(url: str) -> None:
        session.epoch += 1
        await rendezvous.publish(build_record(session, url, {"version": __version__, **host_facts()}))
        bus.publish({"type": "tunnel.epoch", "epoch": session.epoch})
        log.info("published rendezvous epoch %d", session.epoch)

    tunnel = DirectTunnel(local_url, publish) if args.tunnel == "direct" else CloudflaredTunnel(local_url, publish, command=args.cloudflared and [args.cloudflared, "tunnel", "--no-autoupdate", "--url", local_url])

    async def watchdog() -> None:
        while not stop.is_set():
            reason = session.should_stop()
            if reason:
                session.draining = True
                bus.publish({"type": "session.draining", "reason": reason})
                request_stop(reason)
            await asyncio.sleep(5)

    tasks = [asyncio.create_task(server.serve()), asyncio.create_task(tunnel.run()), asyncio.create_task(watchdog())]
    await stop.wait()
    await runner.drain()
    try:
        await rendezvous.remove(session.ticket.session_id)
    except Exception as e:  # noqa: BLE001 - shutting down regardless
        log.warning("could not remove rendezvous record: %s", e)
    bus.publish({"type": "session.stopped"})
    await asyncio.sleep(0.2)  # let event streams flush the stop notice and close
    tunnel.stop()
    server.should_exit = True
    await asyncio.gather(*tasks, return_exceptions=True)


def main() -> None:
    parser = argparse.ArgumentParser(prog="jumbo_worker")
    parser.add_argument("--ticket", required=True, help="session ticket JSON written by the launcher")
    parser.add_argument("--port", type=int, default=8000)
    parser.add_argument("--data", default="/kaggle/working/jumbo", help="blob store folder")
    parser.add_argument("--tunnel", choices=["cloudflared", "direct"], default="cloudflared")
    parser.add_argument("--cloudflared", help="path to the cloudflared binary (default: on PATH)")
    parser.add_argument("--rendezvous", help="override the ticket, e.g. file:/tmp/rv for local runs")
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(name)s %(levelname)s %(message)s")
    asyncio.run(serve(args))


if __name__ == "__main__":
    main()
