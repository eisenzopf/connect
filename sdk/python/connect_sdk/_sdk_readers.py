"""Bounded local waits over owned short polls; never ACK an abandoned reply."""
from __future__ import annotations

import asyncio
from dataclasses import dataclass
from typing import Any, Generic, Self, TypeVar
import uuid

from ._sdk_helper import _SdkHelper
from .sdk_types import Cursor, RequestOptions, SdkCancelledError, SdkError, cancelled

T = TypeVar("T")


@dataclass(frozen=True, repr=False)
class _Reply(Generic[T]):
    value: T | None = None
    acknowledged: Any = None
    ready: bool = False
    cursor: Cursor | None = None
    done: bool = False


class _Reader(Generic[T]):
    def __init__(self, helper: _SdkHelper, options: RequestOptions, prefix: str):
        if type(options) is not RequestOptions:
            raise SdkError("validation", "read-only")
        self._helper = helper
        self._options = options
        self._reader = prefix + uuid.uuid4().hex
        self._acknowledged = None
        self._flight: asyncio.Task[_Reply[T]] | None = None
        self._closing: asyncio.Task[Any] | None = None
        self._busy = False
        self._closed = False
        self._started = False
        self._detached = False

    def __repr__(self):
        return "<Connect SDK owned reader>"

    def _poll_action(self, ready: bool) -> dict:
        raise NotImplementedError()

    def _close_action(self) -> dict:
        raise NotImplementedError()

    def _decode(self, value: Any) -> _Reply[T]:
        raise NotImplementedError()

    def _decode_close(self, value: Any) -> None:
        raise NotImplementedError()

    def _delivered(self, value: T) -> None:
        pass

    async def _wait(self, task: asyncio.Task, expires: float, options: RequestOptions | None, *, closing: bool = False):
        loop = asyncio.get_running_loop()
        while True:
            if (not closing and self._closed) or (options is not None and cancelled(options.signal)):
                raise SdkError("cancelled", "read-only")
            remaining = expires - loop.time()
            if remaining <= 0:
                raise SdkError("deadline", "read-only")
            if task.done():
                return task.result()
            # asyncio.wait does not cancel the original task when this wait is
            # abandoned. Poll the thread-safe signal/close flag with a finite bound.
            await asyncio.wait({task}, timeout=min(remaining, .01))

    def _poll(self, ready: bool) -> asyncio.Task[_Reply[T]]:
        self._started = True
        task = self._helper._own(self._helper.request(self._poll_action(ready), RequestOptions(2000), self._decode))
        self._flight = task
        def failed(done):
            if (done.cancelled() or done.exception() is not None) and self._flight is done:
                self._flight = None
        task.add_done_callback(failed)
        return task

    async def _next(self, ready: bool = False, options: RequestOptions | None = None) -> _Reply[T]:
        selected = self._options if options is None else options
        if type(selected) is not RequestOptions:
            raise SdkError("validation", "read-only")
        if self._closed:
            raise StopAsyncIteration()
        if cancelled(selected.signal):
            raise SdkError("cancelled", "read-only")
        if self._busy:
            raise SdkError("conflict", "read-only")
        self._busy = True
        expires = asyncio.get_running_loop().time() + selected.timeout_ms / 1000
        try:
            abandoned = self._flight
            if abandoned is not None:
                # Never deliver a Python-cached reply after a local timeout or
                # cancellation. Re-poll the same Rust source under current health.
                await self._wait(abandoned, expires, selected)
                if self._flight is abandoned:
                    self._flight = None
            while True:
                if asyncio.get_running_loop().time() >= expires:
                    raise SdkError("deadline", "read-only")
                task = self._flight if self._flight is not None else self._poll(ready)
                reply = await self._wait(task, expires, selected)
                if self._closed or cancelled(selected.signal):
                    raise SdkError("cancelled", "read-only")
                if self._flight is task:
                    self._flight = None
                if reply.done:
                    await self.close()
                    raise StopAsyncIteration()
                if ready and reply.ready:
                    return reply
                if not ready and reply.value is not None:
                    self._delivered(reply.value)
                    self._acknowledged = reply.acknowledged
                    return reply
                await asyncio.sleep(.005)
        except asyncio.CancelledError:
            raise SdkCancelledError(SdkError("cancelled", "read-only")) from None
        finally:
            self._busy = False

    async def close(self) -> None:
        if self._detached:
            return
        self._closed = True
        if not self._started:
            self._detached = True
            return
        if self._closing is None:
            def decode(value):
                self._decode_close(value)
                self._detached = True
                self._flight = None
            task = self._helper._own(self._helper.request(self._close_action(), RequestOptions(2000), decode))
            self._closing = task
            def failed(done):
                if (done.cancelled() or done.exception() is not None) and self._closing is done:
                    self._closing = None
            task.add_done_callback(failed)
        try:
            await self._wait(self._closing, asyncio.get_running_loop().time() + 5, None, closing=True)
        except asyncio.CancelledError:
            raise SdkCancelledError(SdkError("cancelled", "read-only")) from None

    async def aclose(self) -> None:
        await self.close()

    def __aiter__(self) -> Self:
        return self

    async def __anext__(self) -> T:
        reply = await self._next()
        assert reply.value is not None
        return reply.value

    async def next(self, options: RequestOptions | None = None) -> T:
        reply = await self._next(options=options)
        assert reply.value is not None
        return reply.value

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(self, exc_type, exc, traceback):
        await self.close()
