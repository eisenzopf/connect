#!/usr/bin/env python3
"""Explicit live native audio rehearsal through the public Python SDK.

Never invoked by CI/offline qualification. Uses an already configured Connect
server; it does not purchase numbers or configure provider accounts. Each run
has an exclusive private directory, persisted logical intents and a bounded
duration. Failed/uncertain runs are retained and never retried automatically.
"""
import argparse
import json
import os
import math
import struct
import pathlib
import subprocess
import sys
import threading
import time
import uuid
import wave

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "sdk/python"))
sys.path.insert(0, str(ROOT / "examples/conversation-client"))
from connect_sdk import NativeAudioClient, NativeClient
from pcmu import decode, encode


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--allow-live-provider", action="store_true", required=True)
    parser.add_argument("--expect-remote-end", action="store_true", help="Require an explicit remote media-ended notification")
    parser.add_argument("--config", type=pathlib.Path, required=True)
    parser.add_argument("--assistant-config", type=pathlib.Path, required=True)
    parser.add_argument("--conversation", required=True)
    parser.add_argument("--assistant", required=True)
    parser.add_argument("--scenario", type=pathlib.Path, required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--executable", default=str(ROOT / "target/debug/connect"))
    args = parser.parse_args()
    output = args.output.resolve()
    if not output.is_relative_to(ROOT / ".local"):
        parser.error("Private results must be under the ignored .local directory")
    scenario = json.loads(args.scenario.read_text())
    prompts, duration = [], 0
    for turn in scenario["turns"]:
        with wave.open(str(args.scenario.parent / turn["wav"])) as source:
            if (source.getframerate(), source.getnchannels(), source.getsampwidth()) != (8000, 1, 2):
                parser.error("Prompts must be mono signed 16-bit 8 kHz WAV")
            data = encode(source.readframes(source.getnframes()))
        data += b"\xff" * (-len(data) % 160)
        wait = turn.get("wait_seconds", 10)
        if not isinstance(wait, (int, float)) or not math.isfinite(wait) or not 0 <= wait <= 20:
            parser.error("Wait per turn must be between zero and twenty seconds")
        silence = int(wait * 50)
        frames = [data[i:i + 160] for i in range(0, len(data), 160)]
        wait_for_reply = turn.get("wait_for_reply", False)
        if not isinstance(wait_for_reply, bool):
            parser.error("wait_for_reply must be boolean")
        prompts.append((frames, silence, wait_for_reply))
        duration += (len(frames) + silence) / 50 + (12 if wait_for_reply else 0)
    if not 1 <= len(scenario["turns"]) <= 6 or duration + 2 > 120:
        parser.error("Use one to six turns bounded by 120 seconds")
    os.umask(0o077)
    output.mkdir(parents=True, exist_ok=False)
    run_id = str(uuid.uuid4())
    intent = {op: f"{run_id}-{op}" for op in ["invite", "accept", "end"]}
    (output / "intent.json").write_text(json.dumps(intent, indent=2))
    result = {"source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
              "dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT)),
              "transport": "native UCTP/QUIC -> Connect -> Vapi WebSocket", "result": "running"}
    received, errors = bytearray(), []
    finishing = threading.Event()
    reply_audio = threading.Event()
    remote_ended = threading.Event()
    turn_times = []
    started = time.monotonic()
    session, media = None, None
    sender = None
    try:
        with NativeClient(args.config, args.executable) as client:
            session = client.session_invite(intent["invite"], args.conversation, args.assistant, ["audio"])
            intent["session_id"] = session["id"]
            (output / "intent.json").write_text(json.dumps(intent, indent=2))
        with NativeClient(args.assistant_config, args.executable) as client:
            client.session_accept(intent["accept"], args.conversation, session["id"])
        with (output / "helper.log").open("ab", buffering=0) as log:
            media = NativeAudioClient(args.config, args.conversation, session["id"], args.assistant,
                                      executable=args.executable, timeout=20, stderr=log)
            result["publisher_stream_id"] = media.publisher_stream_id

            def send():
                deadline = time.monotonic()
                try:
                    def send_frame(frame):
                        nonlocal deadline
                        media.send_pcmu(frame)
                        deadline += .02
                        time.sleep(max(0, deadline - time.monotonic()))
                    for _ in range(100):
                        send_frame(b"\xff" * 160)
                    for frames, silence, wait_for_reply in prompts:
                        turn_times.append(time.monotonic() - started)
                        for frame in frames:
                            send_frame(frame)
                        if wait_for_reply:
                            reply_audio.clear()
                            reply_deadline = time.monotonic() + 12
                            while not reply_audio.is_set():
                                if time.monotonic() >= reply_deadline:
                                    raise TimeoutError("Reply audio deadline exceeded")
                                send_frame(b"\xff" * 160)
                        for _ in range(silence):
                            send_frame(b"\xff" * 160)
                except Exception as error:
                    if args.expect_remote_end and getattr(error, "code", None) == "media-ended":
                        remote_ended.set()
                    else:
                        errors.append({"class": type(error).__name__, "code": getattr(error, "code", None)})
                finally:
                    finishing.set()
                    media.close()

            sender = threading.Thread(target=send)
            sender.start()
            try:
                while sender.is_alive():
                    frame = media.receive(timeout=20)
                    received.extend(frame)
                    samples = struct.unpack("<" + "h" * len(frame), decode(frame))
                    if samples and sum(value * value for value in samples) / len(samples) > 500 ** 2:
                        reply_audio.set()
            except Exception as error:
                # Closing the sending helper terminates the receiver normally.
                if args.expect_remote_end and getattr(error, "code", None) == "media-ended":
                    remote_ended.set()
                elif not finishing.is_set():
                    errors.append({"class": type(error).__name__, "code": getattr(error, "code", None)})
            sender.join(timeout=5)
        if args.expect_remote_end and not remote_ended.is_set():
            errors.append({"class": "RemoteEndMissing", "code": None})
        result.update(remote_end_observed=remote_ended.is_set(), received_frames=len(received) // 160, received_seconds=len(received) / 8000,
                      errors=errors, turn_start_seconds=turn_times, result="captured" if received and not errors else "failed")
    except Exception as error:
        result.update(result="failed", error_class=type(error).__name__)
    finally:
        if media:
            media.close()
        if sender:
            sender.join(timeout=5)
        with wave.open(str(output / "received.wav"), "wb") as recording:
            recording.setparams((1, 2, 8000, 0, "NONE", "not compressed"))
            recording.writeframes(decode(received))
        if session:
            try:
                with NativeClient(args.config, args.executable) as client:
                    client.session_end(intent["end"], args.conversation, session["id"], "automated-rehearsal-complete")
                    (output / "context.json").write_text(json.dumps(client.context_view(args.conversation, 100), indent=2))
            except Exception as error:
                result.update(result="failed", teardown_error_class=type(error).__name__)
        (output / "result.json").write_text(json.dumps(result, indent=2))
    print(json.dumps({k: v for k, v in result.items() if k != "publisher_stream_id"}))
    return 0 if result["result"] == "captured" else 1


if __name__ == "__main__":
    raise SystemExit(main())
