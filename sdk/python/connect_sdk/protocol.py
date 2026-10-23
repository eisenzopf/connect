from .record_context import valid_context_record
"""Bounded response validation shared by both transports."""
import json
import math
from .media_validation import valid_media
from .errors import ConnectError
from .contact_validation import valid_contact
from ._validation import fields as _fields, array as _array, integer as _integer
from .task_protocol import valid_task_outcome
from .event_validation import valid_events

MAX_REQUEST = 65537
MAX_RESPONSE = 600000
KINDS = {
    "membership":"membership","membership_snapshot":"membership_snapshot","membership_invitations":"membership_invitations",
    "conversation_snapshot":"conversation_snapshot", "conversation_events":"conversation_events", "session_observations":"session_observations",
    "contact_pairing_start": "contact_pairing", "contact_pairing_complete": "contact_pairing",
    "contact_pairing_revoke": "contact_pairing", "contact_state": "contact",
    "task": "task",
    "capabilities": "capabilities", "browser_binding": "browser_binding", "context_deliveries": "context_deliveries",
    "resolve": "identity", "create": "conversation", "close": "conversation", "conversations":"conversations",
    "send": "accepted", "history": "history", "inbox": "inbox",
    "request_status": "request_status", "provider_status": "provider_status",
    "call_start": "call", "call_status": "call", "call_end": "call", "health": "health",
    "session_invite": "session", "session_accept": "session", "session_reject": "session",
    "session_cancel": "session", "session_end": "session", "session_get": "session",
    "sip_dial": "sip_leg", "sip_status": "sip_leg", "sip_end": "sip_leg",
    "context_view": "context_view", "availability_set": "availability",
    "message_receipts": "message_receipts", "message_receipt": "receipt_acknowledged",
    "receipt_preferences_get": "receipt_preferences", "receipt_preferences_set": "receipt_preferences",
    "availability_get": "availability", "artifact_begin": "artifact",
    "transition": "durable_operation", "durable_operation": "durable_operation",
    "artifact_append": "artifact", "artifact_finish": "artifact",
    "artifact_get": "artifact_chunk", "artifact_inspect": "artifact",
    "artifact_upload_link": "artifact_link", "artifact_download_link": "artifact_link",
    "artifact_link_get": "artifact_chunk", "artifact_link_append": "artifact", "artifact_delete": "artifact",
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


def _readiness(value):
    states = ("ready", "degraded", "unavailable", "uncertain")
    return _fields(value, ("state",)) and value["state"] in states and ("media" not in value or valid_media(value["media"])) and _array(
        value.get("components"), lambda item: _fields(item, ("component", "state", "reason"))
        and item["state"] in states and type(item.get("observed_at")) is int
        and ("configured" not in item or type(item["configured"]) is bool))


def _message(value):
    return (_fields(value, ("id","conversation_id","sender","body","content_type","body_encoding","label"), ("sender_participant_id","in_reply_to_msg"))
        and value["body_encoding"] in ("utf8","base64") and "accepted_at" in value
        and (value["accepted_at"] is None or _integer(value["accepted_at"]))
        and _array(value.get("audience"), lambda x: _fields(x,("participant_id",)) and _integer(x.get("membership_epoch")) and x["membership_epoch"]>0)
        and _array(value.get("attachments"), lambda x: _fields(x,("id","content_type")) and _integer(x.get("size_bytes"))))


def valid_mutation_receipt(value, operation_id, conversation_id):
    return (isinstance(value, dict) and set(value) == {"operation_id", "conversation_id", "state", "completion_boundary", "status_reference"}
        and value["operation_id"] == operation_id and value["conversation_id"] == conversation_id
        and value["state"] == "completed" and value["completion_boundary"] == "local-mutation"
        and value["status_reference"] == {"action": "get", "operation_id": operation_id})


def valid_data(kind, data, depth=0):
    if not isinstance(data, dict) or depth > 8:
        return False
    if kind in ('membership','membership_snapshot','membership_invitations'):
        from .membership_validation import valid_membership
        return valid_membership(kind,data)
    if kind in ("conversation_snapshot","conversation_events","session_observations"):
        return valid_events(kind,data,valid_data,depth)
    if kind in ("contact", "contact_pairing"):
        return valid_contact(kind, data)
    if kind == "task":
        return valid_task_outcome(data, valid_data, depth)
    if kind == "identity":
        return _fields(data, ("id", "alias")) and _array(data.get("capabilities"), lambda x: isinstance(x, str))
    if kind in ("conversation","conversations"):
        from .conversation_validation import valid_conversation
        if kind=="conversation":return valid_conversation(data,valid_mutation_receipt)
        return set(data)=={"conversations","next_cursor"} and isinstance(data["conversations"],list) and len(data["conversations"])<=100 and all(valid_conversation(v,valid_mutation_receipt) for v in data["conversations"]) and (data["next_cursor"] is None or isinstance(data["next_cursor"],str) and len(data["next_cursor"])<=2048)
    if kind == "accepted":
        return _fields(data, ("request_id", "message_id", "conversation_id")) and ("receipt" not in data or valid_mutation_receipt(data["receipt"], data["request_id"], data["conversation_id"]))
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
        return ("participant_id" not in data or data["participant_id"] is None or isinstance(data["participant_id"],str)) and ("extensions" not in data or _array(data["extensions"], lambda x: isinstance(x, str))) and _fields(data, ("identity_id",), ("conversation_id",)) and _array(data.get("routes"), route) and isinstance(limits, dict) and all(type(limits.get(k)) is int and limits[k] >= 0 for k in ("message_bytes", "artifact_bytes", "artifact_chunk_bytes", "page_size", "audio_frame_ms")) and isinstance(limits.get("audio_codec"), str) and "availability" in data and (data["availability"] is None or valid_data("availability", data["availability"], depth+1))
    if kind == "context_deliveries":
        return _fields(data, nullable=("next",)) and _array(data.get("deliveries"), _delivery)
    if kind == "context_view":
        delivery = lambda x: _fields(x, ("id", "provider", "state"), ("session_id", "provider_ref")) and all(type(x.get(k)) is int and x[k] >= 0 for k in ("watermark", "created_at", "updated_at"))
        return ("deliveries_omitted" not in data or (type(data["deliveries_omitted"]) is int and data["deliveries_omitted"] >= 0)) and _fields(data, ("conversation_id",)) and all(type(data.get(k)) is int and data[k] >= 0 for k in ("watermark", "omitted")) and _array(data.get("items"), lambda x: _fields(x, ("kind", "source_id", "actor_id", "body")) and type(x.get("sequence")) is int and x["sequence"] >= 0 and ("record" not in x or (valid_context_record(x["record"]) and x["record"]["uuid"] == x["source_id"]))) and _array(data.get("deliveries"), delivery)
    if kind == "message_receipts":
        return _fields(data, nullable=("next",)) and _array(data.get("receipts"), _receipt)
    if kind == "receipt_preferences":
        return _fields(data, ("conversation_id", "participant_id")) and type(data.get("membership_epoch")) is int and data["membership_epoch"] > 0 and type(data.get("delivery")) is bool and type(data.get("read")) is bool
    if kind == "receipt_acknowledged":
        return _fields(data, ("conversation_id", "message_id", "participant_id")) and type(data.get("membership_epoch")) is int and data["membership_epoch"] > 0 and "recorded" in data and data["recorded"] in (None, "delivered", "read")
    if kind == "durable_operation":
        effect = lambda x: _fields(x, ("id", "provider", "correlation_key"), ("compensates_effect_id",)) and x.get("idempotency") in ("none", "provider-key", "status-query") and x.get("state") in ("prepared", "submitting", "confirmed", "failed", "cancelled", "unknown") and all(type(x.get(k)) is int and x[k] >= 0 for k in ("attempt", "submission_generation"))
        return _fields(data, ("operation_id", "record_id", "command_type"), ("conversation_id", "session_id", "connection_id")) and data.get("state") in ("accepted", "preparing", "ready", "committing", "completed", "cancelled", "failed", "interrupted", "unknown") and all(type(data.get(k)) is int and data[k] >= 0 for k in ("revision", "created_at", "updated_at", "events_omitted")) and all(k in data and (data[k] is None or (type(data[k]) is int and data[k] >= 0)) for k in ("expected_revision", "expires_at")) and type(data.get("cancel_requested")) is bool and "result" in data and (data["result"] is None or isinstance(data["result"], dict)) and _array(data.get("causal_event_ids"), lambda x: isinstance(x, str)) and _array(data.get("effects"), effect)
    if kind == "artifact":
        return _fields(data, ("id", "conversation_id", "owner_id", "name", "media_type", "sha256", "state")) and all(type(data.get(k)) is int and data[k] >= 0 for k in ("size", "received", "created_at")) and ("expires_at" not in data or data["expires_at"] is None or (type(data["expires_at"]) is int and data["expires_at"] >= 0))
    if kind == "artifact_link":
        from urllib.parse import quote
        import re
        if not _fields(data, ("conversation_id", "artifact_id", "path"), ("message_id",)):
            return False
        prefix = "/v1/conversations/{}/artifacts/{}/links/".format(quote(data["conversation_id"], safe=""), quote(data["artifact_id"], safe=""))
        return type(data.get("expires_at")) is int and data["expires_at"] >= 0 and data.get("purpose") in ("upload", "download") and data["path"].startswith(prefix) and re.fullmatch(r"[A-Za-z0-9_-]+\.[a-f0-9]{64}", data["path"][len(prefix):]) is not None
    if kind == "artifact_chunk":
        return valid_data("artifact", data.get("artifact"), depth + 1) and _fields(data, ("data",)) and type(data.get("offset")) is int and data["offset"] >= 0 and type(data.get("eof")) is bool
    if kind == "history":
        return _fields(data, nullable=("next",)) and _array(data.get("messages"), _message)
    if kind == "inbox":
        def event(x):
            return _fields(x, ("id", "kind", "conversation_id", "cursor")) and "message" in x and (x["message"] is None or _message(x["message"])) and (x.get("call") is None or valid_data("call", x["call"], depth + 1)) and (x.get("session") is None or valid_data("session", x["session"], depth + 1)) and (x.get("artifact") is None or valid_data("artifact", x["artifact"], depth + 1)) and (x.get("receipt") is None or _receipt(x["receipt"]))
        return _fields(data, nullable=("cursor",)) and _array(data.get("events"), event)
    if kind == "provider_status":
        return _array(data.get("deliveries"), lambda x: _fields(x, ("job_id", "message_id", "connector", "state"), ("provider_id",)))
    if kind == "request_status":
        nested = data.get("outcome")
        return _fields(data, ("state",)) and "outcome" in data and (nested is None or (isinstance(nested, dict) and valid_data(nested.get("kind"), nested.get("data"), depth + 1)))
    if kind == "health":
        return _fields(data, ("installation_id", "profile")) and data["profile"] == "connect-control/1" and type(data.get("schema")) is int and data["schema"] >= 0 and ("readiness" not in data or _readiness(data["readiness"]))
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

def _receipt(x):
    return _fields(x, ("conversation_id", "message_id", "participant_id", "delivery_id", "provenance")) and type(x.get("membership_epoch")) is int and x["membership_epoch"] > 0 and type(x.get("observed_at")) is int and x["observed_at"] >= 0 and x.get("channel") in ("native", "provider") and x.get("state") in ("queued", "accepted", "delivered", "read", "failed", "unknown") and (x["state"] != "read" or (x["channel"] == "native" and x["provenance"] == "client_reported"))
