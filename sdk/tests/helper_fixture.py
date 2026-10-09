#!/usr/bin/env python3
"""Offline child process that exercises framing, lifecycle and failure behavior."""
import json
import sys
import time


def media_frame(kind, payload=b""):
    sys.stdout.buffer.write(bytes([kind]) + len(payload).to_bytes(4, "big") + payload)
    sys.stdout.buffer.flush()


if len(sys.argv) > 1 and sys.argv[1] == "media-stdio":
    mode = sys.argv[sys.argv.index("--config") + 1]
    if mode == "media-stall":
        time.sleep(10)
        raise SystemExit(0)
    if mode == "media-oversize":
        sys.stdout.buffer.write(b"\x81" + (4097).to_bytes(4, "big"))
        sys.stdout.buffer.flush()
        time.sleep(10)
        raise SystemExit(0)
    if mode == "media-noisy":
        sys.stderr.write('media diagnostic\n' * 10000)
        sys.stderr.flush()
    media_frame(0x81, b"fixture-stream")
    if mode == "media-ended":
        time.sleep(.05)
        media_frame(0x83)
        time.sleep(.2)
        sys.exit(0)
    while True:
        header = sys.stdin.buffer.read(5)
        if not header:
            break
        if len(header) != 5:
            raise SystemExit(2)
        kind, length = header[0], int.from_bytes(header[1:], "big")
        payload = sys.stdin.buffer.read(length)
        if len(payload) != length:
            raise SystemExit(2)
        if kind == 0x02:
            if mode == "media-burst":
                frame = b'\x82' + len(payload).to_bytes(4, 'big') + payload
                sys.stdout.buffer.write(frame * 32)
                sys.stdout.buffer.flush()
            else:
                media_frame(0x82, payload)
        elif kind == 0x03:
            break
    raise SystemExit(0)

mode = sys.argv[-1]
for number, line in enumerate(sys.stdin):
    request = json.loads(line)
    response = {"profile": "connect-control/1", "result": {"Ok": {"kind": "health", "data": {"installation_id": "fixture", "schema": 7, "profile": "connect-control/1"}}}}
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
