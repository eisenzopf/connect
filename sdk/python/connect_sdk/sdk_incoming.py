"""Signed invitation hints; refresh the Session before a separate decision."""
from __future__ import annotations

import json
from typing import Any, cast

from ._sdk_readers import _Reader, _Reply
from .client import Client, _bad, _object
from .sdk_types import EventSource, IncomingInvitation, InviteMedium, RequestOptions, freeze, identifier


class InvitationStream(_Reader[IncomingInvitation]):
    def __init__(self, client: Client, options: RequestOptions):
        super().__init__(client._helper, options, "reader-")
        self._client = client

    def _poll_action(self, ready: bool) -> dict:
        return {"type":"incoming_poll", "reader_id":self._reader, "acknowledged":self._acknowledged}

    def _close_action(self) -> dict:
        return {"type":"incoming_close", "reader_id":self._reader, "acknowledged":self._acknowledged}

    def _decode(self, value: Any) -> _Reply[IncomingInvitation]:
        if not _object(value, ("reader_id", "delivery_id", "invitation")) or value["reader_id"] != self._reader:
            return _bad()
        if value["delivery_id"] is None and value["invitation"] is None:
            return _Reply()
        hint = value["invitation"]
        if (not identifier(value["delivery_id"])
                or not _object(hint, ("conversation_id", "session_id", "from_participant", "to_participant", "operation_id", "source", "medium", "intent", "capabilities"))
                or any(not identifier(hint[k]) for k in ("conversation_id", "session_id", "from_participant", "to_participant", "operation_id"))
                or hint["from_participant"] == hint["to_participant"] or not _object(hint["source"], ("kind", "correlation_id"))
                or hint["source"]["kind"] != "session.invite" or hint["source"]["correlation_id"] != hint["operation_id"]
                or type(hint["medium"]) is not str or hint["medium"] not in ("voice", "video", "voice+video", "screen-share", "text-chat", "mixed")
                or type(hint["intent"]) is not str or not 0 < len(hint["intent"].encode("utf-8")) <= 256
                or any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in hint["intent"])
                or len(json.dumps(hint, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode("utf-8")) > 63 * 1024):
            return _bad()
        session = self._client.conversations.by_id(hint["conversation_id"]).sessions.by_id(hint["session_id"])
        result = IncomingInvitation(session, hint["from_participant"], hint["to_participant"], hint["operation_id"], EventSource("session.invite", hint["operation_id"]), cast(InviteMedium,hint["medium"]), hint["intent"], freeze(hint["capabilities"]))
        return _Reply(result, value["delivery_id"])

    def _decode_close(self, value: Any) -> None:
        if not _object(value, ("reader_id", "closed")) or value["reader_id"] != self._reader or value["closed"] is not True:
            _bad()
