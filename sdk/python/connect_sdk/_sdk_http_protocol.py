"""Current finite HTTP envelope profile; no alternate command dialect."""
from __future__ import annotations

from dataclasses import dataclass
import re
from typing import Any, Literal, cast
from urllib.parse import urlsplit

from ._sdk_helper import _encode
from .client import _bad, _object
from .sdk_raw import _scope, _wire
from .sdk_types import CanonicalProblem, ErrorKind, OperationKey, Scope, SdkError, identifier

HttpControlTransport = Literal["loopback-http", "remote-https"]
PROFILE = "connect-sdk-http-control/1"
MAX_REQUEST = 65536
MAX_RESPONSE = 2 * 1024 * 1024
READS = frozenset(("conversation.get", "conversation.list", "conversation.subscribe", "message.history", "operation.get"))
COMMANDS = READS | frozenset((
    "conversation.create", "conversation.close", "conversation.participant.add", "conversation.participant.update", "conversation.participant.remove",
    "session.invite", "session.accept", "session.reject", "session.cancel", "session.end", "session.update", "session.consult", "session.consult.return", "session.handoff",
    "connection.replace", "connection.update", "connection.end", "stream.subscribe", "stream.unsubscribe", "message.send", "message.read", "capability.advertise", "dtmf.send", "operation.cancel", "operation.reconcile",
))
TOP_FIELDS = frozenset(("v", "id", "ts", "type", "payload", "cid", "sid", "connid", "operation_id", "expected_revision", "in_reply_to"))


def _canonical(value: Any, maximum: int) -> tuple[Any,bytes]:
    # The outer owner permits framing overhead; the HTTP profile retains its
    # own depth32/100000-value and interoperable JSON number bounds.
    captured,raw=_encode(value,maximum,True)
    nodes=0
    def visit(item,depth):
        nonlocal nodes
        nodes+=1
        if nodes>100000 or depth>32:
            _bad()
        if type(item) is int and not -9_007_199_254_740_991<=item<=9_007_199_254_740_991:
            _bad()
        if type(item) is float and item.is_integer() and abs(item)>9_007_199_254_740_991:
            _bad()
        if type(item) is dict:
            for child in item.values():visit(child,depth+1)
        elif type(item) is list:
            for child in item:visit(child,depth+1)
    visit(captured,0)
    return captured,raw


@dataclass(frozen=True, repr=False)
class _Endpoint:
    origin: str
    host: str
    port: int
    credential: str
    transport: HttpControlTransport

    @classmethod
    def parse(cls, endpoint: Any, credential: Any, transport: Any) -> _Endpoint:
        try:
            if (type(endpoint) is not str or not 0 < len(endpoint) <= 4096 or any(ord(c) <= 32 or ord(c)==127 for c in endpoint)
                    or type(credential) is not str or not 0 < len(credential) <= 16384
                    or any(not 33 <= ord(c) <= 126 for c in credential)
                    or type(transport) is not str or transport not in ("loopback-http", "remote-https")):
                raise ValueError()
            url = urlsplit(endpoint)
            if (url.username is not None or url.password is not None or url.query or url.fragment or url.path not in ("", "/")
                    or not url.hostname or transport=="loopback-http" and (url.scheme!="http" or url.hostname not in ("127.0.0.1", "::1", "localhost"))
                    or transport=="remote-https" and url.scheme!="https"):
                raise ValueError()
            port = url.port if url.port is not None else (443 if url.scheme=="https" else 80)
            if not 1 <= port <= 65535:
                raise ValueError()
            host = url.hostname.encode("idna").decode("ascii")
            authority = "["+host+"]" if ":" in host else host
            if port != (443 if url.scheme=="https" else 80):
                authority += ":"+str(port)
            return cls(url.scheme+"://"+authority, host, port, credential, cast(HttpControlTransport,transport))
        except (ValueError, TypeError, UnicodeError):
            raise SdkError("configuration", "not-submitted") from None

    def wire(self) -> dict:
        return {"endpoint":self.origin,"credential":self.credential,"transport":self.transport}


def _request(value: Any) -> tuple[dict, bytes, OperationKey | None]:
    captured, body = _canonical(value, MAX_REQUEST)
    _wire(captured)
    if not set(captured) <= TOP_FIELDS - {"in_reply_to"}:
        return _bad()
    scope = Scope(captured.get("cid"), captured.get("sid"), captured.get("connid"))
    key = None
    if captured["type"] not in READS and identifier(captured.get("operation_id")):
        key = OperationKey(captured["operation_id"], captured["type"], scope)
    if captured["type"] not in COMMANDS or captured["type"]=="conversation.subscribe" and captured["payload"].get("live") is not False:
        raise SdkError("unsupported", "not-submitted", key)
    if captured["type"] not in READS and key is None:
        raise SdkError("validation", "not-submitted")
    return captured, body, key


def _problem(value: Any) -> CanonicalProblem:
    if (not _object(value, ("code", "category", "reason")) or type(value["code"]) is not int or not 400 <= value["code"] <= 599
            or type(value["category"]) is not str or re.fullmatch(r"[a-z][a-z-]{0,63}",value["category"]) is None
            or type(value["reason"]) is not str or re.fullmatch(r"[a-z][a-z_-]{0,127}",value["reason"]) is None):
        return _bad()
    return CanonicalProblem(value["code"],value["category"],value["reason"])


def _refused(problem: CanonicalProblem, key: OperationKey | None) -> SdkError:
    kind: ErrorKind = ("authentication" if problem.code in (401,403) else "resync-required" if problem.reason=="resync_required"
                       else "conflict" if problem.code==409 else "unsupported" if problem.code in (488,501)
                       else "validation" if problem.code in (400,413) else "application")
    return SdkError(kind, "refused" if key is not None else "read-only", key, problem)


def _reply(value: Any, original: dict, status: int, key: OperationKey | None) -> tuple[dict, ...]:
    value,_=_canonical(value,MAX_RESPONSE)
    if type(value) is not dict or value.get("profile")!="connect/2":
        return _bad()
    if "problem" in value:
        if not _object(value,("profile","problem")):
            return _bad()
        problem = _problem(value["problem"])
        if problem.code!=status:
            return _bad()
        raise _refused(problem,key)
    if (not _object(value,("profile","request_id","envelopes")) or value["request_id"]!=original["id"]
            or type(value["envelopes"]) is not list or not 1 <= len(value["envelopes"]) <= 128):
        return _bad()
    frames=[];seen=set();terminal=False
    for raw in value["envelopes"]:
        frame,_ = _canonical(raw,1024*1024)
        _wire(frame)
        if (not set(frame)<=TOP_FIELDS or frame.get("in_reply_to")!=original["id"] or frame["id"] in seen or terminal):
            return _bad()
        _scope(frame,original)
        frames.append(frame);seen.add(frame["id"]);terminal=frame["type"] in ("ack","error")
    last=frames[-1]
    if last["type"]=="error":
        problem = _problem(last["payload"])
        if problem.code!=status:
            return _bad()
        raise _refused(problem,key)
    if last["type"]!="ack" or status!=200:
        return _bad()
    return tuple(frames)
