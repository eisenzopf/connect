"""Validate attributed task results, preserving pending and uncertainty."""
from ._validation import array, fields, integer, shape

TASK_STATES = {"ready", "running", "waiting", "completed", "cancelled", "failed", "expired"}
EFFECT_STATES = {"prepared", "dispatched", "succeeded", "failed", "uncertain", "cancelled"}
PERMISSIONS = {
    "resolve", "capabilities", "send", "history", "context_view", "context_deliveries",
    "inbox", "request_status", "artifact_begin", "artifact_append", "artifact_finish",
    "artifact_get", "artifact_delete", "session_invite", "session_accept", "session_reject",
    "session_cancel", "session_end", "session_get", "call_start", "call_status", "call_end",
    "sip_dial", "sip_status", "sip_end",
}


def scope(value):
    return (
        shape(value, extra=("operations", "recipients", "sms_enrollments"))
        and array(value.get("operations"), lambda x: isinstance(x, str) and x in PERMISSIONS)
        and array(value.get("recipients"), lambda x: isinstance(x, str))
        and ("sms_enrollments" not in value or array(
            value["sms_enrollments"],
            lambda x: shape(x, ("source", "grant_id", "purpose")),
        ))
    )


def record(value):
    return (
        shape(
            value, ("id", "owner_id", "conversation_id", "state"),
            ("initiating_event_id", "reason"),
            ("scope", "revision", "expires_at", "created_at", "updated_at"),
        )
        and scope(value.get("scope"))
        and value["state"] in TASK_STATES
        and all(integer(value.get(k)) for k in (
            "revision", "expires_at", "created_at", "updated_at",
        ))
    )


def reply(value):
    return shape(
        value, ("event_id", "conversation_id"),
        ("task_id", "reply_to", "assigned_by"), ("consumed",),
    ) and type(value.get("consumed")) is bool


def claim(value):
    return (
        shape(value, ("task_id", "step_id", "worker_id"), extra=("revision", "expires_at"))
        and integer(value.get("revision")) and integer(value.get("expires_at"))
    )


def outcome(value, validate, depth):
    return (
        fields(value, ("kind",)) and "data" in value
        and validate(value["kind"], value["data"], depth + 1)
    )


def effect(value, validate, depth):
    return (
        shape(
            value, ("id", "task_id", "step_id", "request_id", "state"),
            extra=("outcome", "error"),
        )
        and value["state"] in EFFECT_STATES
        and "outcome" in value
        and (value["outcome"] is None or outcome(value["outcome"], validate, depth))
        and ("error" not in value or value["error"] is None
             or fields(value["error"], ("code", "message")))
    )


def event(value, validate, depth):
    if not (
        shape(value, ("id", "task_id", "actor_id"), extra=("sequence", "created_at", "data"))
        and integer(value.get("sequence")) and integer(value.get("created_at"))
        and shape(value.get("data"), ("kind",), extra=("data",))
        and "data" in value["data"]
    ):
        return False
    kind, data = value["data"]["kind"], value["data"]["data"]
    if kind == "state":
        return (
            shape(data, ("state",), ("reason",), ("revision",))
            and data["state"] in TASK_STATES and integer(data.get("revision"))
        )
    if kind == "reply":
        return reply(data)
    if kind == "effect":
        return effect(data, validate, depth)
    if kind == "claim":
        return claim(data)
    return False


def source(value):
    return (
        shape(
            value, ("source_id", "kind", "actor_id", "provenance", "state"),
            ("body",), ("created_at",),
        )
        and "created_at" in value
        and (value["created_at"] is None or integer(value["created_at"]))
        and value["state"] in ("available", "deleted", "expired", "unavailable")
    )


def valid_task_outcome(value, validate, depth):
    if depth > 8 or not shape(value, ("kind",), extra=("data",)) or "data" not in value:
        return False
    kind, data = value["kind"], value["data"]
    if kind == "record":
        return record(data)
    if kind == "replies":
        return (
            shape(data, nullable=("next",), extra=("items",))
            and array(data.get("items"), reply)
        )
    if kind == "inbox":
        return (
            shape(data, nullable=("next",), extra=("events",))
            and array(data.get("events"), lambda x: event(x, validate, depth))
        )
    if kind == "recall":
        return (
            shape(data, nullable=("next",), extra=("sources", "omitted"))
            and integer(data.get("omitted")) and array(data.get("sources"), source)
        )
    if kind == "effect":
        return effect(data, validate, depth)
    if kind == "claim":
        return data is None or claim(data)
    if kind == "reply_accepted":
        return (
            shape(data, ("request_id", "message_id"), extra=("reply",))
            and reply(data.get("reply"))
        )
    if kind == "tool":
        if not fields(data, ("status", "task_id")):
            return False
        if data["status"] == "completed":
            return (
                shape(data, ("status", "task_id"), extra=("outcome",))
                and outcome(data.get("outcome"), validate, depth)
            )
        if data["status"] == "pending":
            return shape(data, ("status", "task_id"), ("effect_id",))
        return (
            data["status"] == "uncertain"
            and shape(data, ("status", "task_id", "effect_id"))
        )
    return False
