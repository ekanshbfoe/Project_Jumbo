"""The worker's HTTP + WebSocket API (docs/bridge-protocol.md)."""

from __future__ import annotations

import asyncio
from typing import Callable

from fastapi import FastAPI, HTTPException, Request, WebSocket, WebSocketDisconnect
from fastapi.responses import FileResponse
from pydantic import BaseModel

from .blobs import BlobError, BlobStore, check_hash
from .crypto import PROTO
from .events import EventBus
from .jobs import JobError, JobRunner
from .session import AuthError, Session


class MissingRequest(BaseModel):
    hashes: list[str]


class JobRequest(BaseModel):
    kind: str
    input: str
    params: dict = {}


def _path_and_query(scope: dict) -> str:
    """The exact path and query the client signed, before any decoding."""
    path = scope.get("raw_path", scope["path"].encode()).decode("latin-1")
    query = scope.get("query_string", b"").decode("latin-1")
    return f"{path}?{query}" if query else path


def create_app(
    session: Session,
    store: BlobStore,
    bus: EventBus,
    runner: JobRunner,
    on_stop: Callable[[str], None] = lambda reason: None,
) -> FastAPI:
    app = FastAPI(title="Jumbo worker", docs_url=None, redoc_url=None, openapi_url=None)

    async def authed(request: Request) -> bytes:
        body = await request.body()
        try:
            session.authenticate(
                request.headers.get("authorization"), request.method, _path_and_query(request.scope), body
            )
        except AuthError as e:
            raise HTTPException(status_code=401, detail=str(e)) from None
        return body

    @app.get("/health")
    async def health() -> dict:
        return {"ok": True, "proto": PROTO}

    @app.get("/v1/session")
    async def session_info(request: Request) -> dict:
        await authed(request)
        return session.info()

    @app.get("/v1/pool")
    async def pool_status(request: Request) -> dict:
        await authed(request)
        return {"enabled": True, **runner.pool.status()} if runner.pool else {"enabled": False}

    @app.post("/v1/session/stop")
    async def stop(request: Request) -> dict:
        await authed(request)
        if not session.draining:
            session.draining = True
            bus.publish({"type": "session.draining", "reason": "stop requested"})
            on_stop("stop requested")
        return {"state": "draining"}

    @app.post("/v1/blobs/missing")
    async def missing(request: Request) -> dict:
        body = await authed(request)
        req = MissingRequest.model_validate_json(body)
        try:
            return {"missing": store.missing([check_hash(h) for h in req.hashes])}
        except BlobError as e:
            raise HTTPException(status_code=422, detail=str(e)) from None

    @app.get("/v1/blobs/{name}/status")
    async def blob_status(name: str, request: Request) -> dict:
        await authed(request)
        try:
            status = store.status(name)
        except BlobError as e:
            raise HTTPException(status_code=422, detail=str(e)) from None
        return {"received": status.received, "complete": status.complete}

    @app.put("/v1/blobs/{name}")
    async def put_chunk(name: str, offset: int, total: int, request: Request) -> dict:
        body = await authed(request)
        try:
            status = store.write_chunk(name, offset, total, body)
        except BlobError as e:
            raise HTTPException(status_code=422, detail=str(e)) from None
        return {"received": status.received, "complete": status.complete}

    @app.get("/v1/blobs/{name}")
    async def get_blob(name: str, request: Request) -> FileResponse:
        await authed(request)
        try:
            path = store.path(name)
        except BlobError as e:
            raise HTTPException(status_code=422, detail=str(e)) from None
        if not path.is_file():
            raise HTTPException(status_code=404, detail="blob not found")
        return FileResponse(path, media_type="application/octet-stream")

    @app.post("/v1/jobs")
    async def submit_job(request: Request) -> dict:
        body = await authed(request)
        if session.draining:
            raise HTTPException(status_code=409, detail="worker is draining")
        req = JobRequest.model_validate_json(body)
        try:
            job = runner.submit(req.kind, req.input, req.params)
        except (JobError, BlobError) as e:
            raise HTTPException(status_code=400, detail=str(e)) from None
        return {"job_id": job.id}

    @app.get("/v1/jobs/{job_id}")
    async def job_status(job_id: str, request: Request) -> dict:
        await authed(request)
        job = runner.get(job_id)
        if job is None:
            raise HTTPException(status_code=404, detail="job not found")
        return job.public()

    @app.websocket("/v1/events")
    async def events(ws: WebSocket) -> None:
        try:
            session.authenticate(ws.headers.get("authorization"), "GET", _path_and_query(ws.scope), b"")
        except AuthError:
            await ws.close(code=4401)
            return
        await ws.accept()
        queue = bus.subscribe()
        await ws.send_json({"type": "hello", **session.info()})

        async def pump() -> None:
            while True:
                event = await queue.get()
                await ws.send_json(event)
                if event["type"] == "session.stopped":
                    # Close our side so server shutdown is not held open by connected clients.
                    await ws.close(code=1001)
                    return

        async def listen() -> None:
            # Keep-alive frames hold the stream open but are not activity: an app left open with nothing to do
            # must not keep the session alive until its deadline. Requests and running jobs count instead.
            while True:
                await ws.receive_text()

        tasks = [asyncio.create_task(pump()), asyncio.create_task(listen())]
        try:
            await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
        except WebSocketDisconnect:
            pass
        finally:
            for t in tasks:
                t.cancel()
            bus.unsubscribe(queue)

    return app
