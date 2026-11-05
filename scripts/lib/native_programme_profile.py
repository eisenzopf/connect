"""Required original programme Worker receipt; logical IDs are not OS TIDs."""
import math
import re

PROFILE = "connect/native-programme-worker/1"
MAX_REPORT_BYTES = 8 * 1024 * 1024


def number(value):
    return type(value) in (int, float) and math.isfinite(value) and value >= 0


def original_worker_owner(value):
    return (isinstance(value, dict) and set(value) == {"profile", "original_node_pid",
        "logical_worker_thread_id", "created_monotonic_seconds", "started_monotonic_seconds", "original_global_deadline_monotonic_seconds"}
        and value["profile"] == PROFILE
        and type(value["original_node_pid"]) is int and value["original_node_pid"] > 1
        and type(value["logical_worker_thread_id"]) is int and value["logical_worker_thread_id"] > 0
        and number(value["created_monotonic_seconds"]) and number(value["started_monotonic_seconds"])
        and value["started_monotonic_seconds"] >= value["created_monotonic_seconds"]
        and number(value["original_global_deadline_monotonic_seconds"])
        and value["started_monotonic_seconds"] < value["original_global_deadline_monotonic_seconds"] <= value["created_monotonic_seconds"] + 300)


def worker_join_verified(value, owner, terminal):
    try:
        return (original_worker_owner(owner) and value["owner"] == owner and number(terminal)
            and value["original_terminal_monotonic_seconds"] == terminal
            and value["original_exit_observed"] is True and type(value["exit_code"]) is int and value["exit_code"] == 0
            and value["forced"] is False and value["stdio_joined"] is True
            and value["report_digest_verified"] is True and value["failure"] is None
            and isinstance(value["report_sha256"], str) and re.fullmatch(r"[a-f0-9]{64}", value["report_sha256"]) is not None
            and type(value["report_bytes"]) is int and 0 < value["report_bytes"] <= MAX_REPORT_BYTES
            and number(value["exit_monotonic_seconds"]) and number(value["joined_monotonic_seconds"])
            and terminal <= value["exit_monotonic_seconds"] <= value["joined_monotonic_seconds"] <= terminal + 10
            and value["joined_monotonic_seconds"] <= owner["original_global_deadline_monotonic_seconds"])
    except (KeyError, TypeError):
        return False
