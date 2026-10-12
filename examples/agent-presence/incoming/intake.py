"""Explicit incoming requests become tasks inside their existing conversation."""
import json
import re
from typing import get_args
from connect_sdk.task_types import TaskPermission


class Policy:
    """Developer configuration chooses contacts and capabilities once, not per action."""

    def __init__(self, value):
        if set(value) != {"contacts", "operations", "task_ttl_seconds"}:
            raise ValueError("Policy requires contacts, operations and task_ttl_seconds")
        self.contacts = value["contacts"]
        self.operations = value["operations"]
        self.ttl = value["task_ttl_seconds"]
        if (not isinstance(self.contacts, dict) or not 1 <= len(self.contacts) <= 100
                or any(not isinstance(k, str) or not k or not isinstance(v, str) or not v
                       for k, v in self.contacts.items())):
            raise ValueError("Configure concrete contact identities and addresses")
        if (not isinstance(self.operations, list) or not self.operations
                or any(op not in get_args(TaskPermission) for op in self.operations)
                or len(set(self.operations)) != len(self.operations)):
            raise ValueError("Configure distinct supported task operations")
        if type(self.ttl) is not int or not 300 <= self.ttl <= 3600:
            raise ValueError("Task lifetime must be 300–3600 seconds")
        self.value = value

    def classify(self, event, now):
        message = event.get("message")
        if event["kind"] != "message" or not message:
            return "ignored", None, None
        if message["sender"] not in self.contacts:
            return "denied", None, None
        if message["conversation_id"] != event["conversation_id"]:
            raise ValueError("Journal/message conversation mismatch")
        if len(message["body"].encode()) > 8192:
            return "oversized", None, None
        try:
            contact = json.loads(message["body"])
        except (ValueError, TypeError):
            return "unassigned", None, None
        if not isinstance(contact, dict) or contact.get("action") != "contact_assistant":
            return "unassigned", None, None
        if (set(contact) != {"action", "request_id", "body"}
                or not isinstance(contact["request_id"], str)
                or not re.fullmatch(r"[A-Za-z0-9_.:-]{1,128}", contact["request_id"])
                or not isinstance(contact["body"], str) or not contact["body"].strip()):
            return "invalid", None, None
        source = {"event_id": event["id"], "sender": message["sender"],
                  "conversation_id": event["conversation_id"],
                  "request_id": contact["request_id"], "body": contact["body"]}
        definition = {"conversation_id": event["conversation_id"],
                      "scope": {"operations": self.operations, "recipients": [message["sender"]], "sms_enrollments": []},
                      "expires_at": now + self.ttl, "initiating_event_id": event["id"]}
        return "queued", source, definition


def binding_field(record, field):
    value = record[field]
    if field == "scope":
        # The public contract omits empty enrollment policy on serialization.
        # Normalize that optional default without changing saved request bytes.
        return {**value, "sms_enrollments": value.get("sms_enrollments", [])}
    return value


def ensure_task(client, ledger, job, policy):
    """Replay the original create after uncertainty; never infer a replacement task."""
    source = job["source"]
    address = policy.contacts[source["sender"]]
    if client.resolve(address)["id"] != source["sender"]:
        raise ValueError("Configured contact address changed identity")
    client.negotiate_tasks(address, source["conversation_id"])
    if job["task"] is not None:
        current = client.negotiate_task(job["task"]["id"])
        immutable = ("id", "owner_id", "conversation_id", "scope", "expires_at", "initiating_event_id")
        if any(binding_field(current, field) != binding_field(job["task"], field) for field in immutable):
            raise ValueError("Current task binding changed")
        if current["state"] in ("cancelled", "failed", "expired", "completed"):
            return current
    outcome = client.task(job["command"])
    if outcome["kind"] != "record":
        raise ValueError("Expected task creation receipt")
    task = outcome["data"]
    definition = job["command"]["definition"]
    if any(binding_field(task, field) != binding_field(definition, field) for field in ("conversation_id", "scope", "expires_at", "initiating_event_id")):
        raise ValueError("Task receipt differs from saved incoming intent")
    ledger.save_task(job["key"], task)
    current = client.negotiate_task(task["id"])
    if any(binding_field(current, field) != binding_field(task, field) for field in ("id", "owner_id", "conversation_id", "scope", "expires_at", "initiating_event_id")):
        raise ValueError("Current task binding changed")
    return current
