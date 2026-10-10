"""Actual task grant over the public native/HTTP SDK; never use the parent token."""
import json
import sys
import tomllib
from connect_sdk import ConnectError, HttpClient, NativeClient

config, url, executable, mode = sys.argv[1:]
data = json.load(sys.stdin)
with open(config, "rb") as stream:
    token = tomllib.load(stream)["credential"]

def open_client():
    return NativeClient(config, executable=executable) if mode == "native" else HttpClient(url, token)

def denied(fn, code):
    try:
        fn()
    except ConnectError as error:
        assert error.code == code, error.code
    else:
        raise AssertionError("Expected denial")

command = {"action": "invoke", "task_id": data["task"], "request_id": "python-" + mode, "step_id": "read", "operation": {"op": "send", "request_id": "python-" + mode, "conversation_id": data["cid"], "body": "Task-scoped SDK message"}, "reply_to": None, "wait_for_reply": False}
client = open_client()
try:
    denied(lambda: client.task({"action": "get", "task_id": data["task"]}), "unsupported-profile")
    denied(lambda: client.negotiate_task(data["other"]), "forbidden")
    denied(lambda: client.task({"action": "get", "task_id": data["task"]}), "unsupported-profile")
    denied(lambda: client.negotiate_task(""), "invalid-request")
    assert client.negotiate_task(data["task"])["id"] == data["task"]
    denied(lambda: client.task({"action": "get", "task_id": data["other"]}), "forbidden")
    denied(lambda: client.send("primitive-" + mode, data["cid"], "not allowed"), "forbidden")
    outcome = client.task(command)
    assert outcome["kind"] == "tool" and outcome["data"]["status"] == "completed", outcome
    assert client.task(command) == outcome
    changed = json.loads(json.dumps(command))
    changed["operation"]["body"] = "Changed intent"
    denied(lambda: client.task(changed), "request-conflict")
finally:
    if mode == "native": client.close()
client = open_client()
try:
    client.negotiate_task(data["task"])
    assert client.task(command) == outcome
finally:
    if mode == "native": client.close()
print(json.dumps({"status": "ok", "mode": mode}))
