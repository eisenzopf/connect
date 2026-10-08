#!/usr/bin/env python3
"""Offline child process that exercises framing, lifecycle and failure behavior."""
import json
import sys
import time
mode = sys.argv[-1]
for number, line in enumerate(sys.stdin):
    request = json.loads(line)
    response = {"profile": "connect-control/1", "result": {"Ok": {"kind": "health", "data": {"installation_id": "fixture", "schema": 6, "profile": "connect-control/1"}}}}
    if mode == "fragment":
        sys.stdout.write('{"id":'); sys.stdout.flush(); time.sleep(10); continue
    if mode == "stall":
        time.sleep(10); continue
    if mode == "oversize":
        sys.stdout.write("x" * 600001); sys.stdout.flush(); continue
    if mode == "malformed":
        sys.stdout.write("{broken}\n"); sys.stdout.flush(); continue
    if mode == "wrong-kind":
        response["result"]["Ok"]["kind"] = "identity"
    if mode == "bad-fields":
        response["result"]["Ok"]["data"]["schema"] = "six"
    if mode == "denied" and number == 0:
        response["result"] = {"Err": {"code": "forbidden", "message": "Fixture denial"}}
    value = {"id": "wrong" if mode == "wrong-id" else request["id"], "response": response}
    output = json.dumps(value) + "\n"
    if mode == "invalid-utf8":
        sys.stdout.buffer.write(output.encode().replace(b"fixture", b"\xff")); sys.stdout.buffer.flush(); continue
    if mode == "chunked":
        for i in range(0, len(output), 7):
            sys.stdout.write(output[i:i+7]); sys.stdout.flush(); time.sleep(.001)
    else:
        sys.stdout.write(output); sys.stdout.flush()
