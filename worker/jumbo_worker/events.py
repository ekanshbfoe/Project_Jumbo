"""In-process broadcast of worker events to WebSocket subscribers."""

from __future__ import annotations

import asyncio
import time
from collections import deque

QUEUE_SIZE = 1_000


class EventBus:
    def __init__(self, history: int = 200) -> None:
        self._subscribers: set[asyncio.Queue[dict]] = set()
        self.history: deque[dict] = deque(maxlen=history)

    def publish(self, event: dict) -> dict:
        event = {"ts": time.time(), **event}
        self.history.append(event)
        for queue in list(self._subscribers):
            try:
                queue.put_nowait(event)
            except asyncio.QueueFull:
                # A stalled client must not block the worker; it can resync from job status.
                pass
        return event

    def subscribe(self) -> asyncio.Queue[dict]:
        queue: asyncio.Queue[dict] = asyncio.Queue(maxsize=QUEUE_SIZE)
        self._subscribers.add(queue)
        return queue

    def unsubscribe(self, queue: asyncio.Queue[dict]) -> None:
        self._subscribers.discard(queue)
