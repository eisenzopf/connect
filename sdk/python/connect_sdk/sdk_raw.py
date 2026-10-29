"""Finite canonical exchanges over the original owned, verified Rust peer."""
from __future__ import annotations

import base64
from dataclasses import dataclass
from datetime import datetime
import re
from typing import Any, Sequence, cast
from weakref import WeakKeyDictionary

from ._sdk_helper import _SdkHelper, _encode, _parse
from ._sdk_validation import _validate
from .client import _bad, _key, _object
from .sdk_types import Certainty, ErrorKind, Json, OperationKey, RequestOptions, SdkError, freeze, identifier


@dataclass(frozen=True, repr=False)
class _Intent:
    key: OperationKey | None = None
    error: SdkError | None = None


@dataclass(frozen=True, repr=False)
class _Validated:
    value: Json
    intent: _Intent


class ValidatedEnvelope:
    """Immutable local validation metadata. Only request replies are authenticated."""
    __slots__ = ("__weakref__",)

    def __new__(cls):
        raise SdkError("validation", "not-submitted")

    @property
    def value(self) -> Json:
        saved = _tokens.get(self)
        if saved is None:
            raise SdkError("validation", "not-submitted")
        return saved.value

    def to_json(self) -> Json:
        """Export an independent JSON copy; payloads/signatures are private state."""
        return _encode(self.value, 1024 * 1024)[0]


_tokens: WeakKeyDictionary[ValidatedEnvelope, _Validated] = WeakKeyDictionary()


def _token(value: Any, intent: _Intent) -> ValidatedEnvelope:
    result = object.__new__(ValidatedEnvelope)
    _tokens[result] = _Validated(freeze(value), intent)
    return result


def _wire(value: Any) -> None:
    if (type(value) is not dict or type(value.get("v")) is not int or value["v"] != 2
            or not identifier(value.get("id")) or type(value.get("type")) is not str
            or re.fullmatch(r"[a-z][a-z.-]{1,63}", value["type"]) is None
            or type(value.get("ts")) is not str or type(value.get("payload")) is not dict):
        _bad()
    timestamp = datetime.fromisoformat(value["ts"].replace("Z", "+00:00"))
    if timestamp.tzinfo is None or "T" not in value["ts"]:
        _bad()
    for name in ("cid", "sid", "connid", "operation_id", "in_reply_to"):
        if name in value and not identifier(value[name]):
            _bad()
    if "expected_revision" in value and (type(value["expected_revision"]) is not int or not 0 <= value["expected_revision"] <= 9_007_199_254_740_991):
        _bad()


def _intent(value: Any, envelope: Any) -> _Intent:
    def key(value: Any) -> OperationKey | None:
        if value is None:
            return None
        result = _key(value)
        if (result.operation_id != envelope.get("operation_id") or result.command != envelope["type"]
                or result.scope.conversation_id != envelope.get("cid")
                or result.scope.session_id != envelope.get("sid")
                or result.scope.connection_id != envelope.get("connid")):
            return _bad()
        return result
    if _object(value, ("key",)):
        return _Intent(key(value["key"]))
    if not _object(value, ("error",)) or not _object(value["error"], ("kind", "certainty", "operation", "problem", "observation")):
        return _bad()
    error = value["error"]
    if (type(error["kind"]) is not str or error["kind"] not in ("unsupported", "validation")
            or type(error["certainty"]) is not str or error["certainty"] not in ("not-submitted", "read-only")
            or error["problem"] is not None or error["observation"] is not None):
        return _bad()
    operation = key(error["operation"])
    if operation is not None and error["certainty"] != "not-submitted":
        return _bad()
    return _Intent(error=SdkError(cast(ErrorKind,error["kind"]), cast(Certainty,error["certainty"]), operation))


def _scope(frame: dict, original: dict) -> None:
    events = ("conversation.event", "session.started", "session.participant.joined", "session.participant.left", "session.ended", "recording.vcon-ready", "message.read", "message.delivered")
    if all(original.get(k) is None for k in ("sid", "connid", "operation_id")) and frame["type"] in events:
        if frame.get("cid") != original.get("cid"):
            _bad()
        if frame["type"] == "conversation.event":
            payload = frame["payload"]
            fact = payload.get("data")
            if (type(fact) is not dict or not identifier(fact.get("event_id"))
                    or payload.get("event_id") != fact.get("event_id") or payload.get("cursor") != fact.get("cursor")
                    or frame.get("cid") != fact.get("conversation_id")
                    or any(frame.get(a) != fact.get(b) for a,b in (("sid","session_id"),("connid","connection_id"),("operation_id","operation_id")))):
                _bad()
        elif frame["type"].startswith("session.") or frame["type"] == "recording.vcon-ready":
            if not identifier(frame.get("sid")) or frame.get("connid") is not None or frame.get("operation_id") is not None:
                _bad()
            if frame["type"] == "recording.vcon-ready":
                handle = frame["payload"].get("vcon_handle")
                if (type(handle) is not dict or type(handle.get("uuid")) is not str
                        or re.fullmatch(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", handle["uuid"], re.I) is None
                        or type(handle.get("revision")) is not int or not 1 <= handle["revision"] <= 9_007_199_254_740_991):
                    _bad()
        else:
            evidence = frame["payload"].get("evidence")
            if frame.get("sid") is not None or frame.get("connid") is not None or type(evidence) is not dict or not identifier(evidence.get("event_id")):
                _bad()
        return
    if any(frame.get(k) != original.get(k) for k in ("sid", "connid", "operation_id")):
        _bad()
    if frame.get("cid") != original.get("cid") and not (original["type"] in ("conversation.create", "conversation.list") and "cid" not in original):
        _bad()


def _batch(value: Any, original: dict) -> tuple[ValidatedEnvelope, ...]:
    if (not _object(value, ("requestId", "envelopes")) or not identifier(value["requestId"])
            or type(value["envelopes"]) is not list or not 1 <= len(value["envelopes"]) <= 128):
        return _bad()
    seen = set()
    replies = []
    terminal = False
    for item in value["envelopes"]:
        if not _object(item, ("value", "request")):
            return _bad()
        frame, _ = _encode(item["value"], 1024 * 1024)
        _wire(frame)
        if frame["id"] in seen or frame.get("in_reply_to") != value["requestId"] or terminal or frame["type"] == "error":
            return _bad()
        signature = frame.get("signature")
        if (not _object(signature, ("keyid", "alg", "sig")) or type(signature["keyid"]) is not str
                or not 0 < len(signature["keyid"]) <= 512 or type(signature["alg"]) is not str
                or signature["alg"] not in ("EdDSA", "ES256") or type(signature["sig"]) is not str
                or re.fullmatch(r"[A-Za-z0-9_-]{86}", signature["sig"]) is None):
            return _bad()
        raw = base64.urlsafe_b64decode(signature["sig"] + "==")
        if len(raw) != 64 or base64.urlsafe_b64encode(raw).rstrip(b"=").decode() != signature["sig"]:
            return _bad()
        _scope(frame, original)
        seen.add(frame["id"])
        terminal = frame["type"] == "ack"
        replies.append(_token(frame, _intent(item["request"], frame)))
    if not terminal:
        return _bad()
    return tuple(replies)


@dataclass(frozen=True, repr=False)
class RawCanonical:
    _helper: _SdkHelper

    def validate(self, value: Json) -> ValidatedEnvelope:
        """Frozen Rust schema check, bounded to two seconds, without configuration."""
        try:
            captured, raw = _encode(value, 1024 * 1024)
            _wire(captured)
        except (ValueError, TypeError, UnicodeError, RecursionError):
            raise SdkError("validation", "not-submitted") from None
        response = _validate(self._helper._executable, "sdk-validate-envelope", raw)
        try:
            result = _parse(response)
            if (not _object(result, ("profile", "valid", "request")) or result["profile"] != "connect-sdk-validation/1"
                    or type(result["valid"]) is not bool):
                return _bad()
            if not result["valid"]:
                if result["request"] is not None:
                    return _bad()
                raise SdkError("validation", "not-submitted")
            return _token(captured, _intent(result["request"], captured))
        except SdkError:
            raise
        except (ValueError, TypeError, UnicodeError, RecursionError):
            self._helper._fence("protocol")
            raise SdkError("protocol", "not-submitted") from None

    async def request(self, envelope: ValidatedEnvelope, options: RequestOptions) -> Sequence[ValidatedEnvelope]:
        if type(envelope) is not ValidatedEnvelope or (saved := _tokens.get(envelope)) is None:
            raise SdkError("validation", "not-submitted")
        intent = saved.intent
        if intent.error is not None:
            raise SdkError(intent.error.kind, intent.error.certainty, intent.error.operation)
        captured, _ = _encode(saved.value, 1024 * 1024)
        return await self._helper.request({"type":"raw_request", "envelope":captured}, options,
                                          lambda value:_batch(value, captured), intent.key, intent.key is not None)
