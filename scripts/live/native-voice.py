#!/usr/bin/env python3
"""Explicit live native audio rehearsal through the public Python SDK.

Live accounts are never used by CI/offline qualification; local tests inject
provider fixtures. Uses an already configured Connect server and does not
purchase numbers or configure provider accounts. Each run
has an exclusive private directory, persisted logical intents and a bounded
duration. Failed/uncertain runs are retained and never retried automatically.
"""
import argparse
import json
import os
import struct
import pathlib
import threading
import time
import uuid
import wave

ROOT = pathlib.Path(__file__).resolve().parents[2]
from task_voice import TaskVoice, require_task_config
from voice_runtime import load, save
from voice_scenario import load_scenario


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--allow-live-provider", action="store_true", required=True)
    parser.add_argument("--expect-remote-end", action="store_true", help="Require an explicit remote media-ended notification")
    parser.add_argument("--config", type=pathlib.Path, required=True)
    parser.add_argument("--assistant-config", type=pathlib.Path)
    parser.add_argument("--task-id", help="Use an existing task with an exported Native grant")
    parser.add_argument("--session-id", help="Existing accepted task Session with protected Vapi setup")
    parser.add_argument("--runtime-root", type=pathlib.Path, help="Extracted release directory containing manifest.json")
    parser.add_argument("--conversation", required=True)
    parser.add_argument("--assistant", required=True)
    parser.add_argument("--scenario", type=pathlib.Path, required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--executable")
    args = parser.parse_args()
    if bool(args.task_id) != bool(args.session_id):
        parser.error("Task rehearsal requires both --task-id and --session-id")
    if bool(args.assistant_config) == bool(args.task_id):
        parser.error("Choose ordinary --assistant-config or the prepared task/session mode")
    if args.task_id:
        require_task_config(args.config)
    runtime = load(ROOT, args.runtime_root, args.executable)
    NativeClient, NativeAudioClient = runtime.client, runtime.audio
    encode, decode = runtime.encode, runtime.decode
    args.executable = runtime.binary
    output = args.output.resolve()
    if not output.is_relative_to(ROOT / ".local"):
        parser.error("Private results must be under the ignored .local directory")
    prompts = load_scenario(args.scenario, encode)
    os.umask(0o077)
    output.mkdir(parents=True, exist_ok=False)
    run_id = str(uuid.uuid4())
    intent = {op: f"{run_id}-{op}" for op in ["invite", "accept", "inspect", "end"]}
    save(output / "intent.json", intent)
    result = {**runtime.evidence, "task_id": args.task_id,
              "transport": "native UCTP/QUIC -> Connect -> Vapi WebSocket", "result": "running"}
    received, errors = bytearray(), []
    finishing = threading.Event()
    reply_audio = threading.Event()
    remote_ended = threading.Event()
    turn_times = []
    started = time.monotonic()
    session, media = None, None
    task = TaskVoice(args.task_id, args.conversation, args.session_id, args.assistant, intent) if args.task_id else None
    if task:
        intent.update(task_id=args.task_id, session_id=args.session_id, conversation_id=args.conversation)
        save(output / "intent.json", intent)
    sender = None
    try:
        with NativeClient(args.config, args.executable) as client:
            session = task.bind(client) if task else client.session_invite(intent["invite"], args.conversation, args.assistant, ["audio"])
            intent["session_id"] = session["id"]
            save(output / "intent.json", intent)
        if not task:
            with NativeClient(args.assistant_config, args.executable) as client:
                client.session_accept(intent["accept"], args.conversation, session["id"])
        with (output / "helper.log").open("ab", buffering=0) as log:
            media = NativeAudioClient(args.config, args.conversation, session["id"], args.assistant,
                                      executable=args.executable, timeout=20, stderr=log)
            result["publisher_stream_id"] = media.publisher_stream_id
            capture_deadline = time.monotonic() + 120

            def send():
                deadline = time.monotonic()
                try:
                    def send_frame(frame):
                        nonlocal deadline
                        if finishing.is_set() or time.monotonic() >= capture_deadline:
                            raise TimeoutError("Audio capture budget ended")
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
                    elif not finishing.is_set():
                        errors.append({"class": type(error).__name__, "code": getattr(error, "code", None)})
                finally:
                    finishing.set()
                    media.close()

            sender = threading.Thread(target=send)
            sender.start()
            try:
                while sender.is_alive():
                    remaining = capture_deadline - time.monotonic()
                    if remaining <= 0:
                        raise TimeoutError("Audio capture budget ended")
                    frame = media.receive(timeout=min(20, remaining))
                    if len(received) + len(frame) > 8000 * 120:
                        raise ValueError("Received audio exceeds the capture budget")
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
            finishing.set()
            sender.join(timeout=5)
        if args.expect_remote_end and not remote_ended.is_set():
            errors.append({"class": "RemoteEndMissing", "code": None})
        result.update(remote_end_observed=remote_ended.is_set(), received_frames=len(received) // 160, received_seconds=len(received) / 8000,
                      errors=errors, turn_start_seconds=turn_times, result="captured" if received and not errors else "failed")
    except Exception as error:
        result.update(result="failed", error_class=type(error).__name__)
    finally:
        finishing.set()
        if media:
            try:
                media.close()
            except Exception as error:
                result.update(result="failed", media_cleanup_error_class=type(error).__name__)
        if sender:
            sender.join(timeout=5)
            if sender.is_alive():
                result.update(result="failed", sender_cleanup="unfinished")
        try:
            with wave.open(str(output / "received.wav"), "wb") as recording:
                recording.setparams((1, 2, 8000, 0, "NONE", "not compressed"))
                recording.writeframes(decode(received))
        except Exception as error:
            result.update(result="failed", recording_error_class=type(error).__name__)
        if session:
            try:
                with NativeClient(args.config, args.executable) as client:
                    if task:
                        context = task.finish(client)
                    else:
                        client.session_end(intent["end"], args.conversation, session["id"], "automated-rehearsal-complete")
                        context = client.context_view(args.conversation, 100)
                    save(output / "context.json", context)
            except Exception as error:
                result.update(result="failed", teardown_error_class=type(error).__name__)
        save(output / "result.json", result)
    print(json.dumps({k: v for k, v in result.items() if k != "publisher_stream_id"}))
    return 0 if result["result"] == "captured" else 1


if __name__ == "__main__":
    raise SystemExit(main())
