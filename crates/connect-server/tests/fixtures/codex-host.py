#!/usr/bin/env python3
"""Offline Codex process double; real loopback Connect MCP remains authoritative."""
import json
import os
import sys
import time
import urllib.request
import uuid
from pathlib import Path

args = sys.argv[1:]
state = json.loads(Path("host-state.json").read_text())
manifest = json.loads(Path("install.json").read_text())
prompt = sys.stdin.read()
assert prompt == state["prompt"]
assert "--last" not in args and "--ephemeral" not in args
assert "--ignore-user-config" in args
assert "--json" in args
if state["session_id"]:
    assert args[:3] == ["exec", "resume", state["session_id"]]
else:
    assert args[0] == "exec" and "resume" not in args
effective = args[args.index("-c") + 1]
saved_config = Path("mcp-override.txt").read_text()
autonomy = ',default_tools_approval_mode="approve"'
assert autonomy in effective
# An intact legacy export gains this invocation policy without a file rewrite.
assert effective.replace(autonomy, "") == saved_config.replace(autonomy, "")
assert "TELNYX_API_KEY" not in os.environ and "VAPI_PRIVATE_KEY" not in os.environ
assert Path(".agents/skills/connect/SKILL.md").is_file()
mode = Path("fixture-mode").read_text()
if mode == "before":
    sys.exit(2)
session = state["session_id"] or str(uuid.uuid4())
if mode == "switch":
    session = str(uuid.uuid4())
print(json.dumps({"type": "thread.started", "thread_id": session}), flush=True)
if mode == "stall":
    time.sleep(60)
    sys.exit(1)
if mode == "missing":
    sys.exit(2)
headers = {
    "Content-Type": "application/json",
    "Accept": "application/json, text/event-stream",
    "MCP-Protocol-Version": "2025-11-25",
    "Authorization": "Bearer " + os.environ["CONNECT_TASK_GRANT"],
}
def call(method, params, number):
    body = json.dumps({"jsonrpc": "2.0", "id": number, "method": method, "params": params}).encode()
    request = urllib.request.Request(manifest["mcp_uri"], body, headers)
    with urllib.request.urlopen(request, timeout=5) as response:
        return json.load(response)
try:
    call("initialize", {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "offline-codex-fixture", "version": "1"}}, 1)
    result = call("tools/call", {"name": "connect_task_status", "arguments": {}}, 2)
    assert result["result"]["structuredContent"]["data"]["id"] == manifest["task_id"]
    print(json.dumps({"type": "item.completed", "item": {"type": "mcp_tool_call", "result": result}}), flush=True)
    pending = call("tools/call", {"name": "connect_task_wait", "arguments": {"after": None, "limit": 100, "wait_ms": 0}}, 3)
    print(json.dumps({"type": "item.completed", "item": {"type": "mcp_tool_call", "result": pending}}), flush=True)
except Exception:
    print(json.dumps({"type": "turn.failed", "error": {"message": "Connect denied or MCP fixture failed"}}), flush=True)
    sys.exit(5)
# Even an accidentally echoed connection secret must not enter the host journal.
print(json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": os.environ["CONNECT_TASK_GRANT"]}}), flush=True)
if mode == "interrupt":
    sys.exit(3)
print(json.dumps({"type": "turn.completed", "usage": {}}), flush=True)
