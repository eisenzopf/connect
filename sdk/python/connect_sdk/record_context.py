"""Projected record metadata is data, never authority to send or reuse memory."""
from typing import Literal, TypedDict

class ContextRecord(TypedDict):
    uuid: str
    session_id: str
    revision: int
    state: Literal["available", "unavailable", "deleted"]
    content_hash: str | None
    valid_until: int | None

def valid_context_record(value):
    return (
        isinstance(value, dict)
        and set(value) == {"uuid", "session_id", "revision", "state", "content_hash", "valid_until"}
        and isinstance(value["uuid"], str) and isinstance(value["session_id"], str)
        and type(value["revision"]) is int and 0 < value["revision"] <= 9007199254740991
        and value["state"] in ("available", "unavailable", "deleted")
        and ((isinstance(value["content_hash"], str) and bool(value["content_hash"])) if value["state"] == "available" else value["content_hash"] is None)
        and (value["valid_until"] is None or (type(value["valid_until"]) is int and 0 <= value["valid_until"] <= 9007199254740991))
    )
