"""Owned, bounded local media socket; no media bytes enter JSON control RPCs."""
from __future__ import annotations

import asyncio
from dataclasses import dataclass
import secrets
import socket
import re
import struct
from typing import Any

from ._sdk_helper import _SdkHelper, _encode, _parse
from .sdk_types import (CanonicalProblem, CERTAINTIES, ERROR_KINDS, ErrorKind, RequestOptions,
                        SdkError, SdkCancelledError, cancelled)

PROFILE = 'connect-sdk-media/1'
MAX_METADATA = 16384
MAX_PAYLOAD = 16396
WINDOW = 16
HEADER = struct.Struct('!4sBIII')


def error(kind: ErrorKind, mutating: bool = False, sent: bool = False) -> SdkError:
    return SdkError(kind, 'uncertain' if mutating and sent else 'not-submitted' if mutating else 'read-only')


async def frame(reader: asyncio.StreamReader) -> tuple[int, int, dict, bytes]:
    header = await reader.readexactly(HEADER.size)
    magic, kind, id, metadata_size, payload_size = HEADER.unpack(header)
    if (magic != b'C6M1' or kind not in (1, 0x81, 0x82, 0x83, 0xff) or id == 0
            or metadata_size > MAX_METADATA or payload_size > MAX_PAYLOAD):
        raise ValueError()
    metadata = _parse(await reader.readexactly(metadata_size), maximum_bytes=MAX_METADATA)
    if type(metadata) is not dict:
        raise ValueError()
    return kind, id, metadata, await reader.readexactly(payload_size)


def encoded(kind: int, id: int, metadata: dict, payload: bytes = b'') -> bytes:
    _, raw = _encode(metadata, MAX_METADATA)
    if not 0 < id < 2**32 or len(payload) > MAX_PAYLOAD:
        raise ValueError()
    return HEADER.pack(b'C6M1', kind, id, len(raw), len(payload)) + raw + payload


def owned(task: asyncio.Future):
    task.add_done_callback(lambda done: done.exception() if not done.cancelled() else None)
    return task


async def wait(task: asyncio.Future, options: RequestOptions, *, mutating=False,
               sent=lambda: False, closing=False):
    if type(options) is not RequestOptions:
        raise error('validation', mutating)
    until = asyncio.get_running_loop().time() + options.timeout_ms / 1000
    try:
        while True:
            if cancelled(options.signal):
                raise error('cleanup-incomplete' if closing else 'cancelled', mutating, sent())
            if task.done():
                return task.result()
            remaining = until - asyncio.get_running_loop().time()
            if remaining <= 0:
                raise error('cleanup-incomplete' if closing else 'deadline', mutating, sent())
            # Caller cancellation never cancels an original socket read/write.
            await asyncio.wait({task}, timeout=min(.01, remaining))
    except asyncio.CancelledError:
        raise SdkCancelledError(error('cleanup-incomplete' if closing else 'cancelled', mutating, sent())) from None


@dataclass(eq=False, repr=False)
class Job:
    id: int
    command: dict
    payload: bytes
    options: RequestOptions
    deadline: float
    mutating: bool
    future: asyncio.Future
    sent: bool = False


class Channel:
    def __init__(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
        self.reader, self.writer = reader, writer
        writer.transport.set_write_buffer_limits(high=2*MAX_PAYLOAD,low=MAX_PAYLOAD)
        self.sequence = 1
        self.closed = False
        self.closing = False
        self.pending: dict[int, Job] = {}
        self.outbound: asyncio.Queue[Job] = asyncio.Queue(WINDOW)
        self.tasks = [owned(asyncio.create_task(self._read())), owned(asyncio.create_task(self._write()))]

    async def exchange(self, command: dict, payload: bytes, options: RequestOptions, mutating=False):
        if type(options) is not RequestOptions or type(payload) is not bytes:
            raise error('validation', mutating)
        if cancelled(options.signal):
            raise error('cancelled', mutating)
        if self.closed or self.closing:
            raise error('configuration', mutating)
        if len(self.pending) >= WINDOW or self.outbound.full() or self.sequence >= 2**32 - 1:
            raise error('application', mutating)
        try:
            captured, _ = _encode(command, MAX_METADATA)
            encoded(1, 2, {'timeout_ms': options.timeout_ms, 'command': captured}, payload)
        except (ValueError, TypeError, UnicodeError, RecursionError):
            raise error('validation', mutating) from None
        self.sequence += 1
        future = owned(asyncio.get_running_loop().create_future())
        job = Job(self.sequence, captured, payload, options,
                  asyncio.get_running_loop().time() + options.timeout_ms / 1000, mutating, future)
        self.pending[job.id] = job
        self.outbound.put_nowait(job)
        try:
            return await wait(future, options, mutating=mutating, sent=lambda: job.sent)
        except (SdkError, asyncio.CancelledError):
            if not job.sent:
                self.pending.pop(job.id, None)
                if not future.done():
                    future.set_exception(error('cancelled', mutating))
            raise

    async def _write(self):
        try:
            while not self.closed:
                job = await self.outbound.get()
                if job.future.done():
                    continue
                remaining = int((job.deadline - asyncio.get_running_loop().time()) * 1000)
                if cancelled(job.options.signal) or remaining <= 0 or self.closing:
                    self.pending.pop(job.id, None)
                    job.future.set_exception(error('cancelled' if cancelled(job.options.signal) or self.closing else 'deadline', job.mutating))
                    continue
                data = encoded(1, job.id, {'timeout_ms': remaining, 'command': job.command}, job.payload)
                job.sent = True
                self.writer.write(data)
                await self.writer.drain()
        except asyncio.CancelledError:
            raise
        except (OSError, ValueError, ConnectionError):
            self.fence('transport-uncertain')

    async def _read(self):
        try:
            while not self.closed:
                kind, id, metadata, payload = await frame(self.reader)
                job = self.pending.pop(id, None)
                if job is None or kind not in (0x81, 0x82, 0xff):
                    raise ValueError()
                if job.future.done() and self.closing:
                    continue
                if kind == 0xff:
                    if (payload or set(metadata) != {'kind', 'certainty', 'operation', 'problem', 'observation'}
                            or metadata['kind'] not in ERROR_KINDS or metadata['certainty'] not in CERTAINTIES
                            or metadata['certainty'] == 'recorded'
                            or metadata['operation'] is not None or metadata['observation'] is not None):
                        raise ValueError()
                    problem = metadata['problem']
                    if problem is not None and (type(problem) is not dict or set(problem) != {'code', 'category', 'reason'}
                            or type(problem['code']) is not int or not 0 <= problem['code'] <= 65535
                            or any(type(problem[k]) is not str for k in ('category', 'reason'))):
                        raise ValueError()
                    job.future.set_exception(SdkError(metadata['kind'], metadata['certainty'] if job.mutating else 'read-only',
                        problem=CanonicalProblem(**problem) if problem is not None else None))
                else:
                    job.future.set_result((kind, metadata, payload))
        except asyncio.CancelledError:
            raise
        except asyncio.IncompleteReadError:
            self.fence('transport-uncertain')
        except (OSError, ConnectionError):
            self.fence('transport-uncertain')
        except (ValueError, TypeError, KeyError, UnicodeError, RecursionError):
            # Include the popped request when a malformed response is fenced.
            if 'job' in locals() and job is not None and not job.future.done():
                job.future.set_exception(error('protocol', job.mutating, job.sent))
            self.fence('protocol')

    def mark_closing(self):
        self.closing = True
        for job in self.pending.values():
            if not job.future.done():
                job.future.set_exception(error('cancelled', job.mutating, job.sent))

    def fence(self, kind: ErrorKind):
        self.closed = True
        for job in self.pending.values():
            if not job.future.done():
                job.future.set_exception(error(kind, job.mutating, job.sent))
        self.pending.clear()
        self.writer.close()
        for task in self.tasks:
            if task is not asyncio.current_task():
                task.cancel()

    async def close(self):
        self.fence('cancelled')
        # A partially written channel is retired, never reused or replayed.
        for task in self.tasks:
            task.cancel()
        await asyncio.gather(*self.tasks, return_exceptions=True)
        try:
            await asyncio.wait_for(self.writer.wait_closed(), 1)
        except (OSError, ConnectionError):
            pass
        except TimeoutError:
            self.writer.transport.abort()
            raise error('cleanup-incomplete') from None


async def open_channel(helper: _SdkHelper) -> Channel:
    deadline=asyncio.get_running_loop().time()+4
    def options():
        remaining=int((deadline-asyncio.get_running_loop().time())*1000)
        if remaining<=0:
            raise error('deadline')
        return RequestOptions(remaining)
    def supports(value):
        if (type(value) is not dict or set(value)!={'profile','runtime_role','selected_transport','fallback_reason','features'}
                or value['profile']!='connect/2' or value['runtime_role']!='rust' or value['selected_transport']!='quic'
                or value['fallback_reason'] is not None or type(value['features']) is not list or len(value['features'])>128
                or any(type(v) is not str or not re.fullmatch(r'[a-z0-9-]{1,128}',v) for v in value['features'])
                or len(set(value['features']))!=len(value['features'])):
            raise ValueError()
        return 'native-programmatic-media' in value['features'] and 'managed-media-ipc-v1' in value['features']
    if not await helper.request({'type':'capabilities_refresh'},options(),supports):
        raise error('unsupported')
    token = secrets.token_hex(32)
    accepted = owned(asyncio.get_running_loop().create_future())
    handlers: set[asyncio.Task] = set()
    writers: set[asyncio.StreamWriter] = set()
    transferred = None
    accepting = True

    async def accept(connection):
        try:
            reader,writer=await asyncio.open_connection(sock=connection,
                limit=MAX_METADATA+MAX_PAYLOAD+HEADER.size)
        except BaseException:
            connection.close()
            raise
        writers.add(writer)
        try:
            if not accepting or len(writers) > 2 or accepted.done():
                return
            async with asyncio.timeout_at(deadline):
                kind, id, hello, payload = await frame(reader)
                if (kind != 1 or id != 1 or payload or hello != {'profile': PROFILE, 'token': token}
                        or not accepting or accepted.done()):
                    return
                writer.write(encoded(0x81, 1, hello))
                await writer.drain()
                accepted.set_result((reader, writer))
        except (ValueError, OSError, TimeoutError, asyncio.IncompleteReadError):
            pass
        finally:
            if not accepted.done() or accepted.cancelled() or accepted.result()[1] is not writer:
                writer.close()
                try:
                    await asyncio.wait_for(writer.wait_closed(),1)
                except (OSError,TimeoutError):
                    writer.transport.abort()
                writers.discard(writer)
    # A server-owned transport cannot be transferred by merely closing its
    # listener: newer Python versions wait for live clients in wait_closed().
    # Accept a socket explicitly; its StreamWriter then belongs only to Channel.
    listener=socket.socket(socket.AF_INET,socket.SOCK_STREAM)
    listener.setblocking(False)
    listener.bind(('127.0.0.1',0))
    listener.listen(2)
    async def accepts():
        try:
            while True:
                connection,_=await asyncio.get_running_loop().sock_accept(listener)
                if len(handlers)>=2:
                    connection.close()
                    continue
                task=owned(asyncio.create_task(accept(connection)))
                handlers.add(task);task.add_done_callback(handlers.discard)
        except OSError:
            if not accepted.done():
                accepted.set_exception(error('configuration'))
    acceptor=owned(asyncio.create_task(accepts()))
    attach = None
    try:
        endpoint = listener.getsockname()
        def descriptor(value):
            if (value != {'profile': PROFILE, 'attached': True, 'window': WINDOW,
                         'header_bytes': HEADER.size, 'metadata_max_bytes': MAX_METADATA, 'payload_max_bytes': MAX_PAYLOAD}
                    or value['attached'] is not True
                    or any(type(value[k]) is not int for k in ('window','header_bytes','metadata_max_bytes','payload_max_bytes'))):
                raise error('protocol')
            return value
        # This original opening is owned independently of an application's wait.
        attach = owned(asyncio.create_task(helper.request({'type': 'media_attach', 'attachment': {
            'endpoint': f'127.0.0.1:{endpoint[1]}', 'token': token}}, options(), descriptor)))
        async with asyncio.timeout_at(deadline+1):
            await asyncio.gather(attach, accepted)
        reader, writer = accepted.result()
        transferred = writer
        return Channel(reader, writer)
    finally:
        accepting = False
        listener.close()
        acceptor.cancel()
        await asyncio.gather(acceptor,return_exceptions=True)
        for writer in writers:
            if writer is not transferred:
                writer.close()
        for task in tuple(handlers):
            task.cancel()
        if handlers:
            await asyncio.gather(*handlers, return_exceptions=True)
        for writer in writers:
            if writer is not transferred:
                try:
                    await asyncio.wait_for(writer.wait_closed(),1)
                except (OSError,TimeoutError):
                    writer.transport.abort()
        if attach is not None and not attach.done():
            attach.cancel()
            await asyncio.gather(attach, return_exceptions=True)
