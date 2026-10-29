"""Explicit finite HTTP control with isolated, joined per-request ownership."""
from __future__ import annotations

import asyncio
from dataclasses import dataclass
from datetime import datetime,timezone
import sys
from typing import Any, Mapping, Sequence
import uuid

from ._sdk_helper import _SdkHelper
from ._sdk_http_protocol import HttpControlTransport, READS, _Endpoint, _canonical, _reply, _request
from .client import _bad, _object
from .sdk_types import (ConnectionId,ConversationId,Json,OperationId,OperationKey,RequestOptions,Scope,SdkCancelledError,SdkError,SessionId,cancelled,freeze,identifier)


@dataclass(frozen=True)
class HttpCommandScope(Scope):
    operation_id: OperationId | None = None
    expected_revision: int | None = None

    def __post_init__(self):
        super().__post_init__()
        if (self.operation_id is not None and not identifier(self.operation_id)
                or self.expected_revision is not None and (type(self.expected_revision) is not int or not 0<=self.expected_revision<=9_007_199_254_740_991)):
            raise SdkError("validation","not-submitted")

    def _envelope_fields(self) -> dict:
        return {k:v for k,v in (("cid",self.conversation_id),("sid",self.session_id),("connid",self.connection_id),("operation_id",self.operation_id),("expected_revision",self.expected_revision)) if v is not None}


@dataclass(frozen=True,repr=False)
class HttpEnvelope:
    value: Mapping[str,Json]

    def __post_init__(self):
        object.__setattr__(self,"value",freeze(self.value))

    def to_json(self) -> Json:
        return _canonical(self.value,1024*1024)[0]


@dataclass(frozen=True,init=False,repr=False)
class CanonicalHttpClient:
    _endpoint: _Endpoint
    _active: set[_SdkHelper]
    _cleanups: dict[_SdkHelper,asyncio.Task[None]]
    _loop: asyncio.AbstractEventLoop | None
    _closed: bool

    def __init__(self,endpoint: str,credential: str,transport: HttpControlTransport="loopback-http"):
        object.__setattr__(self,"_endpoint",_Endpoint.parse(endpoint,credential,transport))
        object.__setattr__(self,"_active",set())
        object.__setattr__(self,"_cleanups",{})
        object.__setattr__(self,"_loop",None)
        object.__setattr__(self,"_closed",False)

    def _bind(self):
        loop=asyncio.get_running_loop()
        if self._loop is not None and self._loop is not loop:
            raise SdkError("configuration","read-only")
        object.__setattr__(self,"_loop",loop)
        for helper in tuple(self._active):
            cleanup=self._cleanups.get(helper)
            if helper._joined and cleanup is not None and cleanup.done():
                self._active.discard(helper);self._cleanups.pop(helper,None)

    def _cleanup(self,helper: _SdkHelper) -> asyncio.Task[None]:
        task=self._cleanups.get(helper)
        if task is None or task.done() and not helper._joined:
            task=asyncio.create_task(helper.close(RequestOptions(5000)))
            task.add_done_callback(lambda done:done.exception() if not done.cancelled() else None)
            self._cleanups[helper]=task
        return task

    async def request(self,type: str,payload: Mapping[str,Json],scope: HttpCommandScope,options: RequestOptions) -> Sequence[HttpEnvelope]:
        started=asyncio.get_running_loop().time();key=None
        try:
            if options.__class__ is not RequestOptions or scope.__class__ is not HttpCommandScope or not isinstance(payload,Mapping):
                raise ValueError()
            if type not in READS and scope.operation_id is not None:
                key=OperationKey(scope.operation_id,type,Scope(scope.conversation_id,scope.session_id,scope.connection_id))
            frame={"v":2,"id":"env_"+uuid.uuid4().hex,"ts":datetime.now(timezone.utc).isoformat().replace("+00:00","Z"),"type":type,"payload":payload,**scope._envelope_fields()}
            captured,_,key=_request(frame)
        except SdkError:
            raise
        except (ValueError,TypeError,UnicodeError,RecursionError,AttributeError):
            raise SdkError("validation","not-submitted",key) from None
        fail=lambda kind:SdkError(kind,"not-submitted" if key is not None else "read-only",key)
        if cancelled(options.signal):
            raise fail("cancelled")
        try:
            self._bind()
        except SdkError:
            raise fail("configuration") from None
        if self._closed:
            raise fail("configuration")
        if len(self._active)>=16:
            raise fail("application")
        remaining=int(options.timeout_ms-(asyncio.get_running_loop().time()-started)*1000)
        if remaining<1:
            raise fail("deadline")
        helper=_SdkHelper("explicit-http",sys.executable,http=True)
        self._active.add(helper)
        def decode(value: Any):
            if not _object(value,("requestId","envelopes")) or value["requestId"]!=captured["id"]:
                return _bad()
            frames=_reply({"profile":"connect/2","request_id":value["requestId"],"envelopes":value["envelopes"]},captured,200,key)
            return tuple(HttpEnvelope(frame) for frame in frames)
        try:
            return await helper.request({"type":"http_request","config":self._endpoint.wire(),"envelope":captured},RequestOptions(remaining,options.signal),decode,key,key is not None)
        finally:
            try:
                await asyncio.shield(self._cleanup(helper))
            except asyncio.CancelledError:
                raise SdkCancelledError(SdkError("cancelled","uncertain" if key is not None else "read-only",key)) from None
            except SdkError as error:
                raise SdkError(error.kind,"uncertain" if key is not None else "read-only",key) from None
            finally:
                cleanup=self._cleanups.get(helper)
                if helper._joined and cleanup is not None and cleanup.done():
                    self._active.discard(helper);self._cleanups.pop(helper,None)

    async def close(self,options: RequestOptions) -> None:
        if options.__class__ is not RequestOptions:
            raise SdkError("validation","read-only")
        self._bind();object.__setattr__(self,"_closed",True)
        tasks={self._cleanup(helper) for helper in self._active}
        expires=asyncio.get_running_loop().time()+options.timeout_ms/1000
        try:
            while tasks:
                remaining=expires-asyncio.get_running_loop().time()
                if remaining<=0 or cancelled(options.signal):
                    for helper in self._active:helper._terminate(True)
                    raise SdkError("cleanup-incomplete","read-only")
                _,tasks=await asyncio.wait(tasks,timeout=min(remaining,.01))
            for task in self._cleanups.values():
                task.result()
            self._bind()
        except asyncio.CancelledError:
            for helper in self._active:helper._terminate(True)
            raise SdkCancelledError(SdkError("cleanup-incomplete","read-only")) from None

    async def __aenter__(self) -> CanonicalHttpClient:
        return self

    async def __aexit__(self,exc_type,exc,traceback) -> None:
        await self.close(RequestOptions(5000))
