"""One owned original SDK process, bounded FIFO and absolute caller deadlines.

Internal transport only; the typed facade supplies intent and result decoders.
No automatic reconnect, credential reload, mutation replay or alternate journal.
"""
from __future__ import annotations

import asyncio
from collections import deque
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
import json
import math
import os
from typing import Any, Callable, Coroutine, Generic, TypeVar
import uuid

from .sdk_types import (CERTAINTIES, ERROR_KINDS, CanonicalProblem, OperationKey,
                        RequestOptions, SdkError, SdkCancelledError, cancelled)

T = TypeVar("T")


def _pairs(values):
    result = {}
    for key, value in values:
        if key in result:
            raise ValueError()
        result[key] = value
    return result


def _normal(value, depth=0, *, maximum_depth=32, maximum_nodes=100_000, maximum_bytes=65_536):
    budget = [maximum_nodes, maximum_bytes]

    def charge(amount):
        if budget[1] is not None:
            budget[1] -= amount
            if budget[1] < 0:
                raise ValueError()

    def visit(item, level):
        budget[0] -= 1
        if budget[0] < 0 or level > maximum_depth:
            raise ValueError()
        if item is None or type(item) is bool:
            charge(4 if item is None or item is True else 5)
            result = item
        elif type(item) is int:
            if not -(2**63) <= item < 2**64:
                raise ValueError()
            charge(len(str(item)))
            result = item
        elif type(item) is float:
            if not math.isfinite(item):
                raise ValueError()
            charge(len(json.dumps(item, allow_nan=False)))
            result = item
        elif type(item) is str:
            charge(len(item.encode("utf-8")) + 2)
            result = item
        elif isinstance(item, Mapping):
            charge(2 + max(0, len(item) - 1) + len(item))
            result = {}
            for key, child in item.items():
                if type(key) is not str:
                    raise ValueError()
                visit(key, level + 1)
                result[key] = visit(child, level + 1)
        elif isinstance(item, Sequence) and not isinstance(item, (bytes, bytearray)):
            charge(2 + max(0, len(item) - 1))
            result = [visit(child, level + 1) for child in item]
        else:
            raise ValueError()
        return result

    return visit(value, depth)


def _encode(value, limit, large=False):
    value = _normal(value, maximum_depth=40 if large else 32,
                    maximum_nodes=1_100_000 if large else 100_000,
                    maximum_bytes=limit)
    raw = json.dumps(value, ensure_ascii=False, allow_nan=False, separators=(",", ":"), sort_keys=True).encode("utf-8")
    if len(raw) > limit:
        raise ValueError()
    return value, raw


def _parse(raw, large=False):
    if len(raw) > 2 * 1024 * 1024:
        raise ValueError()
    value = json.loads(raw.decode("utf-8", errors="strict"), object_pairs_hook=_pairs,
                       parse_constant=lambda _: (_ for _ in ()).throw(ValueError()))
    return _normal(value, maximum_depth=40 if large else 32,
                   maximum_nodes=1_100_000 if large else 100_000,
                   maximum_bytes=None)


def _object(value, fields):
    return type(value) is dict and set(value) == set(fields)


@dataclass(eq=False, repr=False)
class _Job(Generic[T]):
    id: str
    action: dict
    options: RequestOptions
    deadline: float
    decode: Callable[[Any], T]
    key: OperationKey | None
    mutation: bool
    future: asyncio.Future[T]
    sent: bool = False
    timer: asyncio.TimerHandle | None = None
    watcher: asyncio.Task | None = None


class _SdkHelper:
    def __init__(self, config: str | os.PathLike, executable: str | os.PathLike = "connect", *, administrative=False):
        try:
            self._config, self._executable = os.fspath(config), os.fspath(executable)
            if any(type(value) is not str or not value or len(value) > 4096 or "\0" in value for value in (self._config, self._executable)):
                raise ValueError()
        except (ValueError, TypeError):
            raise SdkError("configuration", "not-submitted") from None
        self._profile = "connect-sdk-administration/2" if administrative else "connect-sdk-helper/4"
        self._command = "sdk-admin-stdio" if administrative else "sdk-stdio"
        self._administrative = administrative
        self._loop: asyncio.AbstractEventLoop | None = None
        self._queue: deque[_Job[Any]] = deque()
        self._active = None
        self._worker = None
        self._spawn = None
        self._process = None
        self._tasks: set[asyncio.Task[Any]] = set()
        self._closed = False
        self._terminating = False
        self._termination = None
        self._force_killed = False
        self._joined = False

    def __repr__(self):
        return "<Connect SDK owned helper>"

    def _task(self, coroutine):
        task = self._loop.create_task(coroutine)
        self._tasks.add(task)
        task.add_done_callback(self._tasks.discard)
        task.add_done_callback(lambda done: done.exception() if not done.cancelled() else None)
        return task

    def _own(self, coroutine: Coroutine[Any, Any, T]) -> asyncio.Task[T]:
        """Keep a reader's bounded RPC owned when its local waiter is cancelled."""
        loop = asyncio.get_running_loop()
        if self._closed or self._loop is not None and self._loop is not loop:
            coroutine.close()
            raise SdkError("configuration", "read-only")
        self._loop = loop
        return self._task(coroutine)

    def _error(self, job, kind):
        return SdkError(kind, "uncertain" if job.mutation and job.sent else "not-submitted" if job.mutation else "read-only", job.key)

    def _release(self, job):
        if job.timer:
            job.timer.cancel()
        if job.watcher:
            job.watcher.cancel()

    def _cancel(self, job, kind):
        if job.future.done():
            return
        if job.sent:
            self._fence(kind)
        else:
            if job is self._active:
                self._active = None
            else:
                self._queue.remove(job)
            self._release(job)
            job.future.set_exception(self._error(job, kind))

    async def _watch(self, job):
        while not job.future.done():
            if cancelled(job.options.signal):
                self._cancel(job, "cancelled")
                return
            await asyncio.sleep(.01)

    async def _wait_future(self, future):
        # asyncio.wait leaves the original future owned on caller cancellation,
        # without creating a shield proxy that logs an expected inner refusal.
        await asyncio.wait({future}, return_when=asyncio.FIRST_COMPLETED)
        return future.result()

    async def request(self, action: dict, options: RequestOptions, decode: Callable[[Any], T],
                      key: OperationKey | None = None, mutation=False) -> T:
        loop = asyncio.get_running_loop()
        started = loop.time()
        fail = lambda kind: SdkError(kind, "not-submitted" if mutation else "read-only", key)
        try:
            if type(options) is not RequestOptions or type(action) is not dict or type(action.get("type")) is not str:
                raise ValueError()
            raw_action = action["type"] == "raw_request"
            captured, _ = _encode(action, 1024 * 1024 + 1024 if raw_action else 65_536 + 1024 if self._administrative else 63 * 1024, raw_action or self._administrative)
        except (ValueError, TypeError, UnicodeError, RecursionError):
            raise fail("validation") from None
        if cancelled(options.signal):
            raise fail("cancelled")
        if self._closed:
            raise fail("configuration")
        if self._loop is not None and loop is not self._loop:
            raise fail("configuration")
        self._loop = loop
        deadline = started + options.timeout_ms / 1000
        if loop.time() >= deadline:
            raise fail("deadline")
        if len(self._queue) + int(self._active is not None) >= 16:
            raise fail("application")
        future = loop.create_future()
        future.add_done_callback(lambda done: done.exception() if not done.cancelled() else None)
        job = _Job("sdk_" + uuid.uuid4().hex, captured, options, deadline,
                   decode, key, mutation, future)
        job.timer = loop.call_at(job.deadline, self._cancel, job, "deadline")
        self._queue.append(job)
        if options.signal is not None:
            job.watcher = self._task(self._watch(job))
        if self._worker is None or self._worker.done():
            self._worker = self._task(self._run())
        try:
            return await self._wait_future(future)
        except asyncio.CancelledError:
            self._cancel(job, "cancelled")
            raise SdkCancelledError(self._error(job, "cancelled")) from None

    async def _start(self):
        try:
            self._process = await asyncio.create_subprocess_exec(
                self._executable, self._command, "--config", self._config,
                stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE, limit=2 * 1024 * 1024 + 1)
        except OSError:
            self._fence("configuration")
            return
        self._task(self._read())
        self._task(self._stderr())
        self._task(self._exit())
        if self._closed:
            self._terminate()

    async def _run(self):
        try:
            while self._queue and not self._closed:
                job = self._queue.popleft()
                if job.future.done():
                    continue
                self._active = job
                if self._spawn is None:
                    self._spawn = self._task(self._start())
                await asyncio.shield(self._spawn)
                if self._closed or job.future.done():
                    continue
                remaining = int((job.deadline - self._loop.time()) * 1000)
                if remaining < 1 or cancelled(job.options.signal):
                    self._cancel(job, "cancelled" if cancelled(job.options.signal) else "deadline")
                    continue
                try:
                    raw_action = job.action["type"] == "raw_request"
                    _, line = _encode({"profile":self._profile,"id":job.id,"request":{"timeout_ms":remaining,"action":job.action}},
                                      2 * 1024 * 1024 if raw_action else 128 * 1024 if self._administrative else 65_536,
                                      raw_action or self._administrative)
                    job.sent = True
                    self._process.stdin.write(line + b"\n")
                    await self._process.stdin.drain()
                    await self._wait_future(job.future)
                except SdkError:
                    pass
                except (OSError, ConnectionError):
                    self._fence("transport-uncertain")
                except (ValueError, TypeError, UnicodeError, RecursionError):
                    self._fence("protocol")
        finally:
            self._worker = None

    def _response(self, value, job):
        if not _object(value, ("profile","id","phase","result")) or value["profile"] != self._profile or value["id"] != job.id or type(value["phase"]) is not str or value["phase"] not in ("startup","request"):
            raise ValueError()
        result = value["result"]
        if type(result) is not dict or len(result) != 1 or not set(result) <= {"Ok","Err"}:
            raise ValueError()
        if "Ok" in result:
            if value["phase"] != "request":
                raise ValueError()
            return job.decode(result["Ok"]), None
        error = result["Err"]
        if not _object(error, ("kind","certainty","operation","problem","observation")) or type(error["kind"]) is not str or error["kind"] not in ERROR_KINDS or type(error["certainty"]) is not str or error["certainty"] not in CERTAINTIES:
            raise ValueError()
        if error["operation"] is not None and error["operation"] != (job.key._wire() if job.key else None):
            raise ValueError()
        problem = error["problem"]
        if problem is not None and (not _object(problem,("code","category","reason")) or type(problem["code"]) is not int or problem["code"] < 0 or type(problem["category"]) is not str or type(problem["reason"]) is not str):
            raise ValueError()
        startup = value["phase"] == "startup"
        if startup and (error["operation"] is not None or problem is not None or error["observation"] is not None or error["certainty"] not in ("not-submitted","read-only")):
            raise ValueError()
        certainty = ("not-submitted" if job.mutation else "read-only") if startup else error["certainty"]
        return None, SdkError(error["kind"], certainty, job.key,
                              CanonicalProblem(**problem) if problem is not None else None, error["observation"])

    async def _read(self):
        pending = bytearray()
        try:
            while chunk := await self._process.stdout.read(65_536):
                job = self._active
                if job is None or len(pending) + len(chunk) > 2 * 1024 * 1024 + 1:
                    self._fence("protocol")
                    return
                if self._loop.time() >= job.deadline:
                    self._fence("deadline")
                    return
                pending.extend(chunk)
                if b"\n" not in pending:
                    continue
                raw, extra = pending.split(b"\n",1)
                if extra:
                    raise ValueError()
                pending.clear()
                value = _parse(raw, job.action["type"] in ("raw_request","events_poll") or self._administrative)
                result, error = self._response(value, job)
                if self._loop.time() >= job.deadline:
                    self._fence("deadline")
                    return
                self._active = None
                self._release(job)
                if error is not None:
                    job.future.set_exception(error)
                    if error.kind in ("protocol","transport-uncertain","cleanup-incomplete") and not (error.kind == "protocol" and job.action["type"] == "events_poll" and value["phase"] == "request"):
                        self._fence(error.kind)
                        return
                else:
                    job.future.set_result(result)
        except (OSError, ConnectionError):
            self._fence("transport-uncertain")
        except Exception:
            self._fence("protocol")
        finally:
            self._fence("transport-uncertain")

    async def _stderr(self):
        while await self._process.stderr.read(65_536):
            pass

    async def _exit(self):
        await self._process.wait()
        self._joined = True
        if self._termination is not None:
            self._termination.cancel()
        self._fence("transport-uncertain")

    def _fence(self, kind):
        self._closed = True
        jobs = ([self._active] if self._active is not None else []) + list(self._queue)
        self._active = None
        self._queue.clear()
        for job in jobs:
            self._release(job)
            if not job.future.done():
                job.future.set_exception(self._error(job, kind))
        self._terminate()

    def _terminate(self, force=False):
        process = self._process
        if process is None or process.returncode is not None:
            return
        try:
            if force:
                self._force_killed = True
                process.kill()
            elif not self._terminating:
                self._terminating = True
                process.stdin.close()
                process.terminate()
                self._termination = self._task(self._fallback())
        except ProcessLookupError:
            pass

    async def _fallback(self):
        await asyncio.sleep(5)
        self._terminate(True)

    async def close(self, options: RequestOptions):
        if type(options) is not RequestOptions:
            raise SdkError("validation","read-only")
        if self._loop is not None and self._loop is not asyncio.get_running_loop():
            raise SdkError("configuration","read-only")
        self._fence("cancelled")
        if self._loop is None:
            return
        deadline = self._loop.time() + options.timeout_ms / 1000
        try:
            while pending := {task for task in self._tasks if not task.done()}:
                remaining = deadline - self._loop.time()
                if remaining <= 0 or cancelled(options.signal):
                    self._terminate(True)
                    raise SdkError("cleanup-incomplete","read-only")
                await asyncio.wait(pending, timeout=min(remaining,.01) if options.signal is not None else remaining)
        except asyncio.CancelledError:
            self._terminate(True)
            raise SdkCancelledError(SdkError("cleanup-incomplete", "read-only")) from None
        if self._force_killed:
            raise SdkError("cleanup-incomplete","read-only")
