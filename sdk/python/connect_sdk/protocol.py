"""Bounded response validation shared by both transports."""
import json
import math
from .errors import ConnectError

MAX_REQUEST = 65537
MAX_RESPONSE = 600000
KINDS = {
    "resolve": "identity", "create": "conversation", "close": "conversation",
    "send": "accepted", "history": "history", "inbox": "inbox",
    "request_status": "request_status", "provider_status": "provider_status",
    "call_start": "call", "call_status": "call", "call_end": "call", "health": "health",
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
    if kind == "history":
        return _fields(data, nullable=("next",)) and _array(data.get("messages"), lambda x: _fields(x, ("id", "conversation_id", "sender", "body")))
    if kind == "inbox":
        def event(x):
            return _fields(x, ("id", "kind", "conversation_id", "cursor")) and "message" in x and (x["message"] is None or _fields(x["message"], ("id", "conversation_id", "sender", "body"))) and (x.get("call") is None or valid_data("call", x["call"], depth + 1))
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
