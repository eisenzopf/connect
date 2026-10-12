#!/usr/bin/env python3
"""Capture one turn on a prepared Vapi task session; never retry an uncertain POST."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import time
import uuid

from chat_transport import identifier, origin, request
from task_voice import require_task_config
from voice_runtime import load, save

ROOT = Path(__file__).resolve().parents[2]
DRIVERS = ("task-chat.py", "chat_transport.py", "task_voice.py", "voice_runtime.py")


def private_json(path):
    """Read bounded JSON from a private regular file without following a symlink."""
    import stat
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as stream:
        mode = os.fstat(stream.fileno()).st_mode
        if not stat.S_ISREG(mode) or mode & 0o077:
            raise ValueError("Expected private regular file")
        data = stream.read(65537)
    if len(data) > 65536:
        raise ValueError("Protected input exceeds 64 KiB")
    return json.loads(data)


def verify_session(session, intent, *, require_active=True):
    if (session.get("id") != intent["session_id"] or (require_active and session.get("status") != "active")
            or session.get("assistantId") != intent["assistant_id"]
            or session.get("metadata", {}).get("variableValues", {}).get("connect_invocation_id") != intent["nonce"]):
        raise ValueError("Provider session does not match the protected setup")
    if any(session.get(key) for key in ("transport", "phoneNumberId", "customer")):
        raise ValueError("This text rehearsal cannot use a delivery/SMS session")


def verify_chat(chat, intent):
    identifier(chat.get("id"))
    # Vapi normalizes a text turn into one user message in its stored receipt.
    # Exact equality also rejects extra messages, roles, fields or changed text.
    expected_message = [{"role": "user", "content": intent["input"]}]
    input_matches = chat.get("input") in (intent["input"], expected_message)
    if (chat.get("sessionId") != intent["session_id"] or chat.get("name") != intent["name"]
            or not input_matches):
        raise ValueError("Chat receipt does not match the original turn")
    if not isinstance(chat.get("output"), list) or not chat["output"]:
        raise ValueError("Chat has no captured output")


def require_current_setup(definition):
    expiry = definition.get("expires_at")
    if type(expiry) is not int or expiry <= time.time():
        raise ValueError("Prepared task invocation has expired; do not start a turn")


def execute(args):
    output = args.output.resolve()
    if not output.is_relative_to(ROOT / ".local"):
        raise ValueError("Private results must be under ignored .local")
    require_task_config(args.config)
    runtime = load(ROOT, args.runtime_root, args.executable, driver_files=DRIVERS)
    setup = private_json(args.setup / "intent.json")
    definition = setup["definition"]
    if (setup.get("profile") != "connect-vapi-bootstrap/1"
            or definition.get("origin") != "chat_session"
            or definition.get("task_id") != args.task_id or not setup.get("external_id")):
        raise ValueError("Use this task's prepared and reconciled chat-session setup")
    base = args.api_base.rstrip("/")
    origin(base)
    setup_digest = hashlib.sha256(json.dumps(setup, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    os.umask(0o077)
    if args.action == "capture":
        if not args.prompt or args.chat_id:
            raise ValueError("Capture requires --prompt and no --chat-id")
        require_current_setup(definition)
        with args.prompt.open("rb") as prompt_file:
            data = prompt_file.read(16385)
        if not 1 <= len(data) <= 16384:
            raise ValueError("Use a nonempty UTF-8 prompt of at most 16 KiB")
        prompt = data.decode()
        output.mkdir(parents=True, exist_ok=False)
        intent = {"profile": "connect-task-chat-rehearsal/1", "task_id": args.task_id,
                  "session_id": identifier(setup["external_id"]),
                  "assistant_id": identifier(definition["assistant_id"]), "nonce": setup["nonce"],
                  "name": "connect-" + uuid.uuid4().hex, "input": prompt,
                  "api_base": base, "setup_sha256": setup_digest,
                  "state": "prepared", "runtime": runtime.evidence}
        save(output / "intent.json", intent)
    else:
        if args.prompt or not args.chat_id:
            raise ValueError("Reconcile requires an exact --chat-id and no prompt")
        identifier(args.chat_id)
        intent = private_json(output / "intent.json")
        if (intent.get("profile") != "connect-task-chat-rehearsal/1"
                or intent["task_id"] != args.task_id or intent["session_id"] != setup["external_id"]
                or intent["assistant_id"] != definition["assistant_id"] or intent["nonce"] != setup["nonce"]
                or intent["api_base"] != base or intent["state"] != "dispatched"
                or intent.get("setup_sha256") != setup_digest):
            raise ValueError("Reconcile only the original uncertain turn and unchanged setup")
    # Each inspection has a fresh private directory, but the turn intent never changes.
    inspection = output / ("inspection-" + uuid.uuid4().hex)
    inspection.mkdir(mode=0o700)
    result = {"result": "failed", "action": args.action, "task_id": args.task_id,
              "session_id": intent["session_id"], "runtime": runtime.evidence,
              "evidence_directory": str(inspection)}
    try:
        with runtime.client(args.config, runtime.binary) as client:
            task = client.negotiate_task(args.task_id)
            result["conversation_id"] = task["conversation_id"]
            save(inspection / "before.json", client.task({"action": "recall", "task_id": args.task_id, "limit": 100}))
            session = request(base, "GET", "/session/" + intent["session_id"], None, inspection, "session")
            verify_session(session, intent, require_active=args.action == "capture")
            # Recheck current task authority after the provider observation.
            client.negotiate_task(args.task_id)
            if args.action == "capture":
                require_current_setup(definition)
                intent["state"] = "dispatched"
                save(output / "intent.json", intent)
                chat = request(base, "POST", "/chat", {"sessionId": intent["session_id"],
                               "input": intent["input"], "name": intent["name"], "stream": False}, inspection, "chat")
            else:
                chat = request(base, "GET", "/chat/" + args.chat_id, None, inspection, "chat")
            verify_chat(chat, intent)
            save(inspection / "after.json", client.task({"action": "recall", "task_id": args.task_id, "limit": 100}))
            intent.update(state="captured", chat_id=chat["id"])
            save(output / "intent.json", intent)
            result.update(result="captured", chat_id=chat["id"])
    except Exception as error:
        result.update(result="uncertain" if intent["state"] == "dispatched" else "failed",
                      error_class=type(error).__name__, error_code=getattr(error, "code", None))
    save(inspection / "result.json", result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("capture", "reconcile"))
    parser.add_argument("--allow-live-provider", action="store_true", required=True)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--setup", type=Path, required=True)
    parser.add_argument("--task-id", required=True)
    parser.add_argument("--runtime-root", type=Path)
    parser.add_argument("--executable")
    parser.add_argument("--api-base", default="https://api.vapi.ai")
    parser.add_argument("--prompt", type=Path)
    parser.add_argument("--chat-id")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = execute(args)
        print(json.dumps({key: result[key] for key in ("result", "action", "task_id", "session_id",
                         "error_class", "error_code", "evidence_directory") if key in result}))
        return 0 if result["result"] == "captured" else 1
    except Exception as error:
        print(json.dumps({"result": "failed", "error_class": type(error).__name__}))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
