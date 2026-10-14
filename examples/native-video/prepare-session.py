#!/usr/bin/env python3
"""Accepted AV Session through public SDKs; persisted intent survives lost ACKs."""
import argparse
import json
import os
from pathlib import Path
import stat
import tomllib
import uuid
from connect_sdk import NativeClient


def save(path, intent):
    temporary = path.with_name(path.name + ".next")
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(fd, "w") as output:
            json.dump(intent, output); output.flush(); os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--human", type=Path, required=True)
    parser.add_argument("--peer", type=Path, required=True)
    parser.add_argument("--intent", type=Path, required=True)
    parser.add_argument("--connect", default="connect")
    args = parser.parse_args()
    human = tomllib.loads(args.human.read_text())["identity_id"]
    peer = tomllib.loads(args.peer.read_text())["identity_id"]
    if args.intent.exists():
        metadata = args.intent.lstat()
        if not stat.S_ISREG(metadata.st_mode) or stat.S_IMODE(metadata.st_mode) & 0o077:
            raise ValueError("Use a regular private intent file")
        intent = json.loads(args.intent.read_text())
        if intent["human"] != human or intent["peer"] != peer:
            raise ValueError("Retained intent belongs to different participants")
    else:
        intent = {"human": human, "peer": peer, "prefix": str(uuid.uuid4())}
        # Persist the logical ID before any network operation.
        fd = os.open(args.intent, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w") as output:
            json.dump(intent, output); output.flush(); os.fsync(output.fileno())
    left = NativeClient(args.human, executable=args.connect)
    right = NativeClient(args.peer, executable=args.connect)
    try:
        prefix = intent["prefix"]
        if "conversation" not in intent:
            intent["conversation"] = left.create(prefix + ":create", [peer])["id"]
            save(args.intent, intent)
        if "session" not in intent:
            intent["session"] = left.session_invite(prefix + ":invite", intent["conversation"], peer, ["audio", "video"])["id"]
            save(args.intent, intent)
        right.session_accept(prefix + ":accept", intent["conversation"], intent["session"])
        print(json.dumps({"conversation": intent["conversation"], "session": intent["session"], "human": human, "peer": peer}))
    finally:
        left.close(); right.close()


if __name__ == "__main__":
    main()
