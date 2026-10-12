#!/usr/bin/env python3
"""Process one durable incoming contact through public native SDK and packaged Codex."""
import argparse
import json
from pathlib import Path
import time
import tomllib
from connect_sdk import NativeClient
from incoming.host import Host
from incoming.intake import Policy, ensure_task
from incoming.state import Ledger


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for argument in ("config", "policy", "state"):
        parser.add_argument("--" + argument, type=Path, required=True)
    parser.add_argument("--http-address", required=True)
    parser.add_argument("--mcp-address", required=True)
    parser.add_argument("--executable", default="connect")
    parser.add_argument("--codex-binary", default="codex")
    args = parser.parse_args()
    if args.policy.stat().st_size > 16384:
        raise ValueError("Policy exceeds 16 KiB")
    policy = Policy(json.loads(args.policy.read_text()))
    config = tomllib.loads(args.config.read_text())
    binding = {"identity_id": config["identity_id"], "credential_id": config["credential_id"], "policy": policy.value}
    ledger = Ledger(args.state, binding)
    client = None
    try:
        client = NativeClient(args.config, executable=args.executable)
        if ledger.pending() is None:
            page = client.inbox(cursor=ledger.get("cursor"), limit=100, wait_ms=0)
            ledger.stage(page, policy.classify, int(time.time()))
        job = ledger.pending()
        if job is None:
            print(json.dumps({"status": "page-processed", "cursor": ledger.get("cursor")}))
            return
        task = ensure_task(client, ledger, job, policy)
        if task["state"] in ("cancelled", "failed", "expired", "completed"):
            ledger.finish(job["key"], "task-closed")
            print(json.dumps({"status": "task-closed", "task_id": task["id"]}))
            return
        client.close()
        client = None
        host = Host(args.executable, args.codex_binary, args.config.resolve(), args.http_address, args.mcp_address)
        directory = args.state.resolve() / job["key"]
        result = host.run(directory, task)
        ledger.finish(job["key"], "host-turn-completed")
        print(json.dumps({**result, "installation": str(directory / "host"),
                          "conversation_id": task["conversation_id"], "continuation": "explicit-resume"}))
    finally:
        if client is not None:
            client.close()
        ledger.close()


if __name__ == "__main__":
    main()
