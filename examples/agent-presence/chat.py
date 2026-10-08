#!/usr/bin/env python3
"""SDK example. Keep the same request ID and body when recovering a send."""
import argparse
import json
from pathlib import Path
import time
from connect_sdk import NativeClient

parser = argparse.ArgumentParser()
parser.add_argument("--config", required=True)
parser.add_argument("--manifest", required=True)
parser.add_argument("--request-id", required=True)
parser.add_argument("--body", required=True)
parser.add_argument("--executable", default="connect")
args = parser.parse_args()
conversation = json.loads(Path(args.manifest).read_text())["conversation_id"]
with NativeClient(args.config, args.executable) as client:
    receipt = client.send(args.request_id, conversation, args.body)
    print(json.dumps({"accepted": receipt}))
    for _ in range(60):
        deliveries = client.provider_status(conversation)["deliveries"]
        if any(d["message_id"] == receipt["message_id"] and d["state"] != "pending" and d["state"] != "sending" for d in deliveries):
            print(json.dumps({"history": client.history(conversation), "deliveries": deliveries}))
            break
        time.sleep(1)
    else:
        print(json.dumps({"pending": client.provider_status(conversation)}))
