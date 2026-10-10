"""Bounded response validation shared by both transports."""
import json
import math
from .errors import ConnectError

MAX_REQUEST = 65537
MAX_RESPONSE = 600000
KINDS = {
    "capabilities": "capabilities", "browser_binding": "browser_binding", "context_deliveries": "context_deliveries",
    "resolve": "identity", "create": "conversation", "close": "conversation",
    "send": "accepted", "history": "history", "inbox": "inbox",
    "request_status": "request_status", "provider_status": "provider_status",
    "call_start": "call", "call_status": "call", "call_end": "call", "health": "health",
    "session_invite": "session", "session_accept": "session", "session_reject": "session",
    "session_cancel": "session", "session_end": "session", "session_get": "session",
    "sip_dial": "sip_leg", "sip_status": "sip_leg", "sip_end": "sip_leg",
    "context_view": "context_view", "availability_set": "availability",
    "availability_get": "availability", "artifact_begin": "artifact",
    "artifact_append": "artifact", "artifact_finish": "artifact",
    "artifact_get": "artifact_chunk", "artifact_delete": "artifact",
}


def expected_kind(operation):
    try:
        return KINDS[operation["op"]]
    except (KeyError, TypeError) as error:
        raise ConnectError("invalid-request", "Unknown operation") from error


def timeout_value(value):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or not 0 < value <= 60:
        raise ValueError("Timeout must be positive and at most 60 seconds")
    return value


def _fields(value, strings=(), nullable=()):
    return isinstance(value, dict) and all(isinstance(value.get(k), str) for k in strings) and all(k in value and (value[k] is None or isinstance(value[k], str)) for k in nullable)


def _array(value, test):
    return isinstance(value, list) and all(test(item) for item in value)


def valid_data(kind, data, depth=0):
    if not isinstance(data, dict) or depth > 8:
        return False
    if kind == "identity":
        return _fields(data, ("id", "alias")) and _array(data.get("capabilities"), lambda x: isinstance(x, str))
    if kind == "conversation":
        return _fields(data, ("id",)) and type(data.get("closed")) is bool and _array(data.get("participants"), lambda x: isinstance(x, str))
    if kind == "accepted":
        return _fields(data, ("request_id", "message_id", "conversation_id"))
    if kind == "call":
        return _fields(data, ("id", "conversation_id", "route_id", "destination", "state", "direction"), ("provider_id", "ended_reason")) and type(data.get("end_requested")) is bool
    if kind == "session":
        return _fields(data, ("id", "conversation_id", "initiator_id", "participant_id", "state"), ("ended_reason",)) and _array(data.get("media"), lambda x: isinstance(x, str)) and all(type(data.get(k)) is int and data[k] >= 0 for k in ("created_at", "updated_at"))
    if kind == "sip_leg":
        return _fields(data, ("id", "conversation_id", "session_id", "destination", "state"), ("connection_id", "ended_reason")) and type(data.get("end_requested")) is bool and all(type(data.get(k)) is int and data[k] >= 0 for k in ("created_at", "updated_at"))
    if kind == "availability":
        return _fields(data, ("identity_id", "state"), ("preferred_route",)) and all(k in data and (data[k] is None or (type(data[k]) is int and data[k] >= 0)) for k in ("expires_at", "observed_at"))
    if kind == "browser_binding":
        return _fields(data, ("conversation_id", "participant_id", "signaling_url"), ("session_id",))
    if kind == "capabilities":
        route = lambda x: _fields(x, ("id", "transport", "readiness")) and x["readiness"] in ("configured", "unconfigured") and all(_array(x.get(k), lambda v: isinstance(v, str)) for k in ("media", "operations"))
        limits = data.get("limits")
        return _fields(data, ("identity_id",), ("conversation_id",)) and _array(data.get("routes"), route) and isinstance(limits, dict) and all(type(limits.get(k)) is int and limits[k] >= 0 for k in ("message_bytes", "artifact_bytes", "artifact_chunk_bytes", "page_size", "audio_frame_ms")) and isinstance(limits.get("audio_codec"), str) and "availability" in data and (data["availability"] is None or valid_data("availability", data["availability"], depth+1))
    if kind == "context_deliveries":
        return _fields(data, nullable=("next",)) and _array(data.get("deliveries"), _delivery)
    if kind == "context_view":
        delivery = lambda x: _fields(x, ("id", "provider", "state"), ("session_id", "provider_ref")) and all(type(x.get(k)) is int and x[k] >= 0 for k in ("watermark", "created_at", "updated_at"))
        return ("deliveries_omitted" not in data or (type(data["deliveries_omitted"]) is int and data["deliveries_omitted"] >= 0)) and _fields(data, ("conversation_id",)) and all(type(data.get(k)) is int and data[k] >= 0 for k in ("watermark", "omitted")) and _array(data.get("items"), lambda x: _fields(x, ("kind", "source_id", "actor_id", "body")) and type(x.get("sequence")) is int and x["sequence"] >= 0) and _array(data.get("deliveries"), delivery)
    if kind == "artifact":
        return _fields(data, ("id", "conversation_id", "owner_id", "name", "media_type", "sha256", "state")) and all(type(data.get(k)) is int and data[k] >= 0 for k in ("size", "received", "created_at")) and ("expires_at" not in data or data["expires_at"] is None or (type(data["expires_at"]) is int and data["expires_at"] >= 0))
    if kind == "artifact_chunk":
        return valid_data("artifact", data.get("artifact"), depth + 1) and _fields(data, ("data",)) and type(data.get("offset")) is int and data["offset"] >= 0 and type(data.get("eof")) is bool
    if kind == "history":
        return _fields(data, nullable=("next",)) and _array(data.get("messages"), lambda x: _fields(x, ("id", "conversation_id", "sender", "body")))
    if kind == "inbox":
        def event(x):
            return _fields(x, ("id", "kind", "conversation_id", "cursor")) and "message" in x and (x["message"] is None or _fields(x["message"], ("id", "conversation_id", "sender", "body"))) and (x.get("call") is None or valid_data("call", x["call"], depth + 1)) and (x.get("session") is None or valid_data("session", x["session"], depth + 1)) and (x.get("artifact") is None or valid_data("artifact", x["artifact"], depth + 1))
        return _fields(data, nullable=("cursor",)) and _array(data.get("events"), event)
    if kind == "provider_status":
        return _array(data.get("deliveries"), lambda x: _fields(x, ("job_id", "message_id", "connector", "state"), ("provider_id",)))
    if kind == "request_status":
        nested = data.get("outcome")
        return _fields(data, ("state",)) and "outcome" in data and (nested is None or (isinstance(nested, dict) and valid_data(nested.get("kind"), nested.get("data"), depth + 1)))
    if kind == "health":
        return _fields(data, ("installation_id", "profile")) and data["profile"] == "connect-control/1" and type(data.get("schema")) is int and data["schema"] >= 0
    return False


def decode_response(raw, expected):
    try:
        response = json.loads(raw)
        if not isinstance(response, dict) or response.get("profile") != "connect-control/1":
            raise ValueError()
        result = response["result"]
        if not isinstance(result, dict) or len(result) != 1:
            raise ValueError()
        if "Err" in result:
            if not _fields(result["Err"], ("code", "message")):
                raise ValueError()
            raise ConnectError(result["Err"]["code"], result["Err"]["message"])
        outcome = result["Ok"]
        if not isinstance(outcome, dict) or outcome.get("kind") != expected or not valid_data(expected, outcome.get("data")):
            raise ValueError()
        return outcome
    except (ValueError, KeyError, TypeError, RecursionError, UnicodeError) as error:
        raise ConnectError("protocol", "Malformed or unexpected Connect response") from error


def _delivery(value):
    return _fields(value, ("id", "provider", "state"), ("session_id", "provider_ref")) and all(type(value.get(k)) is int and value[k] >= 0 for k in ("watermark", "created_at", "updated_at"))
