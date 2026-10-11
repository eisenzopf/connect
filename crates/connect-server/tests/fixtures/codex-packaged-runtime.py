#!/usr/bin/env python3
"""Test-only exec shim: select scripted Responses and empty account directories."""
import json
import os
import sys
from pathlib import Path

config = json.loads(Path(__file__).with_suffix(".json").read_text())
args = sys.argv[1:]
assert args[0] == "exec" and args[-1] == "-"
assert "--ignore-user-config" in args
overrides = [
    "-c", 'model_provider="connect_packaged_fixture"',
    "-c", 'model_providers.connect_packaged_fixture={name="Connect scripted fixture",'
    f'base_url={json.dumps(config["provider"])},wire_api="responses",'
    'requires_openai_auth=false,supports_websockets=false,'
    'request_max_retries=0,stream_max_retries=0}',
]
env = {key: os.environ[key] for key in ("PATH", "LANG", "TMPDIR", "CONNECT_TASK_GRANT")
       if key in os.environ}
env.update(HOME=config["user_home"], CODEX_HOME=config["codex_home"])
# Preserve the launcher's PID, process group, stdin and JSONL output. Only the
# inference endpoint and account directories differ from the shipped command.
os.execve(config["binary"], [config["binary"], *args[:-1], *overrides, args[-1]], env)
