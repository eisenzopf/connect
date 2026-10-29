"""Owned durable journal reader; cursors are opaque and ACK follows delivery."""
from __future__ import annotations

from typing import Any

from ._sdk_helper import _SdkHelper
from ._sdk_readers import _Reader, _Reply
from .client import _bad, _object
from .sdk_types import (
    AudienceMember, ConversationId, Cursor, EventSource, JournalEvent,
    RequestOptions, SdkError, freeze, identifier,
)


def _cursor(value: Any) -> bool:
    return type(value) is str and 0 < len(value) <= 2048 and all(33 <= ord(c) <= 126 for c in value)


def _text(value: Any, maximum: int) -> bool:
    return type(value) is str and 0 < len(value.encode("utf-8")) <= maximum


def _integer(value: Any, minimum: int = 0) -> bool:
    return type(value) is int and minimum <= value <= 9_007_199_254_740_991


def _event(value: Any, cid: ConversationId) -> JournalEvent:
    if (not _object(value, ("eventId", "cursor", "conversationId", "sessionId", "connectionId", "operationId", "participantId", "audience", "kind", "revision", "source", "observedAt", "data"))
            or not identifier(value["eventId"]) or not _cursor(value["cursor"]) or value["conversationId"] != cid
            or not _text(value["kind"], 128) or not _integer(value["revision"])
            or type(value["observedAt"]) is not int or not -(2**63) <= value["observedAt"] < 2**63
            or type(value["audience"]) is not list or len(value["audience"]) > 256
            or not _object(value["source"], ("kind", "correlationId"))
            or not _text(value["source"]["kind"], 128) or not _text(value["source"]["correlationId"], 2048)):
        return _bad()
    if any(value[k] is not None and not identifier(value[k]) for k in ("sessionId", "connectionId", "operationId", "participantId")):
        return _bad()
    seen = set()
    audience = []
    for member in value["audience"]:
        if (not _object(member, ("participantId", "membershipEpoch")) or not identifier(member["participantId"])
                or member["participantId"] in seen or not _integer(member["membershipEpoch"], 1)):
            return _bad()
        seen.add(member["participantId"])
        audience.append(AudienceMember(member["participantId"], member["membershipEpoch"]))
    body = value["data"].get("data") if type(value["data"]) is dict else None
    if type(body) is dict:
        session = body.get("session")
        if "conversation_id" in body and body["conversation_id"] != cid:
            return _bad()
        if type(session) is dict:
            if "conversation_id" in session and session["conversation_id"] != cid:
                return _bad()
            if "id" in session and value["sessionId"] is not None and session["id"] != value["sessionId"]:
                return _bad()
        for field, key in (("session_id", "sessionId"), ("connection_id", "connectionId"), ("operation_id", "operationId")):
            if field in body and value[key] is not None and body[field] != value[key]:
                return _bad()
        resource_kind = body.get("resource_kind")
        expected = {"conversation":cid, "session":value["sessionId"], "connection":value["connectionId"], "operation":value["operationId"]}.get(resource_kind) if type(resource_kind) is str else None
        if expected is not None and "resource_id" in body and body["resource_id"] != expected:
            return _bad()
    return JournalEvent(value["eventId"], Cursor(value["cursor"]), cid, value["sessionId"], value["connectionId"], value["operationId"], value["participantId"], tuple(audience), value["kind"], value["revision"], EventSource(value["source"]["kind"], value["source"]["correlationId"]), value["observedAt"], freeze(value["data"]))


class EventStream(_Reader[JournalEvent]):
    """Use async with (or aclose) to detach on loop break or application error."""
    def __init__(self, helper: _SdkHelper, cid: ConversationId, after: Cursor | None, options: RequestOptions):
        if not identifier(cid) or after is not None and not _cursor(after):
            raise SdkError("validation", "read-only")
        super().__init__(helper, options, "events-")
        self._cid, self._after = cid, after
        self._baseline: Cursor | None = None
        self._seen: dict[str, None] = {}

    def _poll_action(self, ready: bool) -> dict:
        return {"type":"events_poll", "reader_id":self._reader, "conversation_id":self._cid, "after":self._after, "acknowledged":self._acknowledged, "ready":ready}

    def _close_action(self) -> dict:
        return {"type":"events_close", "reader_id":self._reader, "conversation_id":self._cid}

    def _decode(self, value: Any) -> _Reply[JournalEvent]:
        if (not _object(value, ("reader_id", "conversation_id", "ready", "cursor", "event", "done"))
                or value["reader_id"] != self._reader or value["conversation_id"] != self._cid
                or type(value["ready"]) is not bool or type(value["done"]) is not bool
                or (not _cursor(value["cursor"]) if value["ready"] else value["cursor"] is not None)
                or value["done"] and value["event"] is not None or value["event"] is not None and not value["ready"]):
            return _bad()
        if value["ready"]:
            if (self._after is not None and value["cursor"] != self._after
                    or self._baseline is not None and value["cursor"] != self._baseline):
                return _bad()
            self._baseline = Cursor(value["cursor"])
        elif self._baseline is not None:
            return _bad()
        fact = None if value["event"] is None else _event(value["event"], self._cid)
        ack = None if fact is None else {"event_id":fact.event_id, "cursor":fact.cursor}
        return _Reply(fact, ack, value["ready"], self._baseline, value["done"])

    def _decode_close(self, value: Any) -> None:
        if (not _object(value, ("reader_id", "conversation_id", "closed")) or value["reader_id"] != self._reader
                or value["conversation_id"] != self._cid or value["closed"] is not True):
            _bad()

    def _delivered(self, value: JournalEvent) -> None:
        if value.event_id in self._seen:
            self._helper._fence("protocol")
            raise SdkError("protocol", "read-only")
        if len(self._seen) == 256:
            del self._seen[next(iter(self._seen))]
        self._seen[value.event_id] = None

    async def ready(self, options: RequestOptions | None = None) -> Cursor:
        """Finish the authorized bootstrap without consuming a journal fact."""
        reply = await self._next(ready=True, options=options)
        assert reply.cursor is not None
        return reply.cursor
