"""Synchronous entry into the same typed async SDK, on one owned caller loop."""
from __future__ import annotations

import asyncio
from dataclasses import dataclass
import inspect
import threading
from typing import Any, Awaitable, Callable, Generic, TypeVar
from weakref import WeakValueDictionary

from .client import Client
from .sdk_administration import AdministrativeClient
from .sdk_http import CanonicalHttpClient
from .sdk_types import RequestOptions, SdkError, cancelled

C = TypeVar("C", Client, AdministrativeClient, CanonicalHttpClient)
T = TypeVar("T")
_owners: WeakValueDictionary[int, Any] = WeakValueDictionary()
_registry_lock = threading.Lock()


@dataclass(frozen=True, init=False, repr=False)
class SynchronousSdk(Generic[C]):
    """Transfer a fresh SDK client to a single-thread synchronous owner.

    run() invokes a typed async callback on a persistent private event loop.
    Each SDK call retains its own RequestOptions deadline/cancellation. User
    callback code has no additional timeout or retry. close() has an explicit
    whole cleanup budget and retains the loop if cleanup is incomplete.
    """
    _client: C
    _thread: int
    _loop: asyncio.AbstractEventLoop | None
    _running: bool
    _closing: bool
    _closed: bool

    def __init__(self, client: C):
        if type(client) not in (Client, AdministrativeClient, CanonicalHttpClient):
            raise SdkError("configuration", "read-only")
        helper = getattr(client, "_helper", None)
        media = getattr(client, "media", None)
        if (getattr(client, "_closed", False) or getattr(client, "_loop", None) is not None
                or helper is not None and (helper._closed or helper._loop is not None)
                or media is not None and media._owner.loop is not None):
            raise SdkError("configuration", "read-only")
        with _registry_lock:
            if id(client) in _owners:
                raise SdkError("configuration", "read-only")
            _owners[id(client)] = self
        object.__setattr__(self, "_client", client)
        object.__setattr__(self, "_thread", threading.get_ident())
        object.__setattr__(self, "_loop", None)
        object.__setattr__(self, "_running", False)
        object.__setattr__(self, "_closing", False)
        object.__setattr__(self, "_closed", False)

    def _check(self):
        if threading.get_ident() != self._thread or self._running:
            raise SdkError("configuration", "read-only")
        try:
            asyncio.get_running_loop()
        except RuntimeError:
            return
        raise SdkError("configuration", "read-only")

    def _drive(self, action: Callable[[], Awaitable[T]]) -> T:
        if self._loop is None:
            object.__setattr__(self, "_loop", asyncio.new_event_loop())
        loop = self._loop
        assert loop is not None
        async def invoke():
            value = action()
            if not inspect.isawaitable(value):
                raise SdkError("validation", "read-only")
            return await value
        object.__setattr__(self, "_running", True)
        try:
            return loop.run_until_complete(invoke())
        finally:
            object.__setattr__(self, "_running", False)

    def run(self, action: Callable[[C], Awaitable[T]]) -> T:
        self._check()
        if self._closing or self._closed or not callable(action):
            raise SdkError("configuration", "read-only")
        return self._drive(lambda: action(self._client))

    def _helpers(self):
        helper = getattr(self._client, "_helper", None)
        return (helper,) if helper is not None else tuple(getattr(self._client, "_active", ()))

    def _managed(self):
        tasks = {task for helper in self._helpers() for task in helper._tasks}
        return tasks | set(getattr(self._client, "_cleanups", {}).values())

    async def _shutdown(self, options: RequestOptions):
        loop = asyncio.get_running_loop()
        expires = loop.time() + options.timeout_ms / 1000
        failure = None
        retained = asyncio.all_tasks(loop) - {asyncio.current_task()}
        try:
            await self._client.close(options)
        except SdkError as error:
            failure = error
        # Async SDK helpers retain their original process/reader tasks until
        # joined. Cancel callback-created tasks, never these cleanup owners.
        managed = self._managed()
        asyncgens = asyncio.create_task(loop.shutdown_asyncgens())
        pending = retained | (asyncio.all_tasks(loop) - {asyncio.current_task()})
        for task in pending - managed - {asyncgens}:
            task.cancel()
        while pending:
            remaining = expires - loop.time()
            if remaining <= 0 or cancelled(options.signal):
                raise SdkError("cleanup-incomplete", "read-only")
            done, pending = await asyncio.wait(pending, timeout=min(remaining, .01))
            for task in done:
                if not task.cancelled():
                    task.exception()
            pending |= asyncio.all_tasks(loop) - {asyncio.current_task()}
            managed = self._managed()
            for task in pending - managed - {asyncgens}:
                task.cancel()
        if any(helper._process is not None and not helper._joined for helper in self._helpers()):
            raise SdkError("cleanup-incomplete", "read-only")
        if failure is not None:
            raise failure

    def close(self, options: RequestOptions) -> None:
        self._check()
        if type(options) is not RequestOptions:
            raise SdkError("validation", "read-only")
        if self._closed:
            return
        object.__setattr__(self, "_closing", True)
        try:
            self._drive(lambda: self._shutdown(options))
        finally:
            # Never close a loop over live processes or pending cleanup work.
            if (self._loop is not None and not asyncio.all_tasks(self._loop)
                    and all(helper._process is None or helper._joined for helper in self._helpers())):
                self._loop.close()
                object.__setattr__(self, "_closed", True)
                with _registry_lock:
                    if _owners.get(id(self._client)) is self:
                        _owners.pop(id(self._client), None)

    def __enter__(self) -> SynchronousSdk[C]:
        self._check()
        if self._closing or self._closed:
            raise SdkError("configuration", "read-only")
        return self

    def __exit__(self, exc_type, exc, traceback) -> None:
        self.close(RequestOptions(5000))
