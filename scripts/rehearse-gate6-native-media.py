#!/usr/bin/env python3
"""Owned Linux guest to Mac canonical audio; synthetic fixtures, no providers."""
import argparse
import array
import datetime
import hashlib
import json
import math
import os
from pathlib import Path
import re
import runpy
import select
import signal
import socket
import subprocess
import sys
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
HELPERS = runpy.run_path(str(ROOT / "scripts/rehearse-gate6-fifty-participants.py"))
CASE = "canonical_native_connections::remote::media::distinct_network_audio_server_child"


class LossProxy:
    """One owned guest tuple and one fixed local server; no arbitrary forwarding."""
    def __init__(self, server_port, fixture, every):
        self.front = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.front.bind(("0.0.0.0", 0))
        self.upstream = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.upstream.bind(("127.0.0.1", 0))
        self.server = ("127.0.0.1", server_port)
        self.fixture = fixture
        self.every = every
        self.cancel = threading.Event()
        self.stats = {"drop_every_after_media_go": every, "foreign_datagrams": 0,
                      "guest_to_server": {"eligible": 0, "dropped": 0, "forwarded": 0},
                      "server_to_guest": {"eligible": 0, "dropped": 0, "forwarded": 0}}
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        guest = None
        try:
            while not self.cancel.is_set():
                for source in select.select([self.front, self.upstream], [], [], .05)[0]:
                    packet, address = source.recvfrom(65535)
                    if source is self.front:
                        if guest is None:
                            guest = address
                        if address != guest:
                            self.stats["foreign_datagrams"] += 1
                            continue
                        direction, output, target = "guest_to_server", self.upstream, self.server
                    else:
                        if address != self.server or guest is None:
                            self.stats["foreign_datagrams"] += 1
                            continue
                        direction, output, target = "server_to_guest", self.front, guest
                    counters = self.stats[direction]
                    if (self.fixture / "media-go").exists():
                        counters["eligible"] += 1
                        if counters["eligible"] % self.every == 0:
                            counters["dropped"] += 1
                            continue
                    output.sendto(packet, target)
                    counters["forwarded"] += 1
        except Exception as error:
            self.stats["error"] = type(error).__name__ + ": " + str(error)

    def close(self):
        self.cancel.set()
        self.thread.join(timeout=1)
        try:
            assert not self.thread.is_alive(), "Owned UDP proxy did not drain"
            assert "error" not in self.stats, "Owned UDP proxy failed"
        finally:
            self.front.close()
            self.upstream.close()
        self.stats["worker_joined"] = True


def digest(path):
    with Path(path).open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def assert_sources(inputs):
    assert inputs and all(digest(ROOT / name) == value for name, value in inputs.items()), "Build inputs changed"


def stop(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()


def decoded_tone(path, expected, own, ffmpeg):
    decoded = subprocess.check_output([ffmpeg, "-v", "error", "-f", "mulaw", "-ar", "8000",
                                       "-ac", "1", "-i", str(path), "-f", "s16le", "-"], timeout=15)
    samples = array.array("h", decoded)
    if sys.byteorder != "little":
        samples.byteswap()
    active = matched = 0
    for offset in range(0, len(samples) - 159, 160):
        frame = samples[offset:offset + 160]
        if math.sqrt(sum(v * v for v in frame) / 160) < 100:
            continue
        active += 1
        def amplitude(frequency):
            real = sum(v * math.cos(2 * math.pi * frequency * i / 8000) for i, v in enumerate(frame))
            imag = sum(v * math.sin(2 * math.pi * frequency * i / 8000) for i, v in enumerate(frame))
            return math.hypot(real, imag)
        if amplitude(expected) > 4 * max(amplitude(own), 1):
            matched += 1
    result = {"expected_hz": expected, "excluded_self_hz": own, "decoded_samples": len(samples),
              "active_frames": active, "matching_frames": matched, "pcmu_sha256": digest(path),
              "decoded_s16le_sha256": hashlib.sha256(decoded).hexdigest()}
    result["passed"] = active >= 50 and matched >= .8 * active
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server-build", type=Path, required=True)
    parser.add_argument("--server-inputs", type=Path, required=True)
    parser.add_argument("--guest-build", type=Path, required=True)
    parser.add_argument("--image", default="connect-gate6-native-client:clang1971-ubuntu2404")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--ffmpeg", default="ffmpeg")
    parser.add_argument("--drop-every", type=int, choices=(0, 10), default=0,
                        help="Drop every tenth encrypted UDP datagram in each direction after media starts")
    parser.add_argument("--unreachable-port", action="store_true",
                        help="Keep an owned UDP port unresponsive; require client timeout before its offer")
    args = parser.parse_args()
    assert not (args.unreachable_port and args.drop_every), "Choose one bounded fault"
    out = args.output.absolute()
    assert out.parent.resolve().is_relative_to((ROOT / ".local/gate-6/native-remote").resolve())
    out.mkdir(mode=0o700, exist_ok=False)
    server = json.loads(args.server_build.read_text())
    guest = json.loads(args.guest_build.read_text())
    server_inputs = json.loads(args.server_inputs.read_text())
    assert server["exit_code"] == 0 and not server["changed_inputs"] and guest["passed"]
    assert_sources(server_inputs)
    assert_sources(guest["source_inputs_sha256"])
    binary = Path(server["artifact"]["sealed"])
    probe = Path(guest["executable"])
    assert digest(binary) == server["artifact"]["sha256"]
    assert digest(probe) == guest["executable_sha256"]
    image = json.loads(subprocess.check_output(["docker", "image", "inspect", args.image]))[0]
    assert image["Id"] == guest["image_id"] and image["Architecture"] == "amd64" and image["Os"] == "linux"
    assert CASE + ": test" in subprocess.check_output([str(binary), "--list"], text=True).splitlines()
    before = HELPERS["sources"]()
    report = {"passed": False, "scope": "Owned Linux/amd64 guest under QEMU on local Lima arm64 VM to macOS arm64 server; public canonical SDK audio or the selected bounded transport refusal. No native x86 performance, WAN, video, browser, SIP, relay, full loss-quality or ICE restart qualification.",
              "mode": "unreachable-port" if args.unreachable_port else "loss" if args.drop_every else "direct",
              "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
              "source_inputs_sha256": before, "server_executable_sha256": digest(binary),
              "guest_executable_sha256": digest(probe), "image_id": image["Id"],
              "runner_sha256": digest(__file__), "drop_every": args.drop_every, "profiles": [],
              "started_at": datetime.datetime.now(datetime.timezone.utc).isoformat()}
    try:
        for algorithm in ["EdDSA", "ES256"]:
            fixture = out / algorithm
            fixture.mkdir(mode=0o700)
            for name, hz in [("host", 440), ("guest", 660)]:
                subprocess.run([args.ffmpeg, "-v", "error", "-f", "lavfi", "-i",
                                f"sine=frequency={hz}:sample_rate=8000:duration=4", "-ac", "1",
                                "-c:a", "pcm_mulaw", "-f", "mulaw", str(fixture / f"{name}-send.pcmu")], check=True, timeout=15)
            env = dict(os.environ, CONNECT_G6_DISTINCT_NATIVE_DIR=str(fixture),
                       CONNECT_G6_DISTINCT_NATIVE_ALGORITHM=algorithm, TMPDIR=str(fixture))
            env.pop("CONNECT_G6_NATIVE_UNREACHABLE", None)
            if args.unreachable_port:
                env["CONNECT_G6_NATIVE_UNREACHABLE"] = "1"
            with (fixture / "server.log").open("w") as log:
                proxy = None
                blackhole = None
                blocked_datagrams = 0
                process = subprocess.Popen([str(binary), CASE, "--exact", "--ignored", "--test-threads=1", "--nocapture"],
                                           cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                try:
                    started = time.monotonic()
                    while not (fixture / "ready.json").exists():
                        assert process.poll() is None, "Server ended before readiness"
                        assert time.monotonic() - started < 60, "Server readiness deadline"
                        time.sleep(.02)
                    if args.drop_every or args.unreachable_port:
                        ready = json.loads((fixture / "ready.json").read_text())
                        if args.unreachable_port:
                            # Retain custody of this port, so another process
                            # cannot accidentally turn the negative into a dial.
                            blackhole = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                            blackhole.bind(("0.0.0.0", 0))
                            port = blackhole.getsockname()[1]
                        else:
                            proxy = LossProxy(ready["port"], fixture, args.drop_every)
                            port = proxy.front.getsockname()[1]
                        config_path = fixture / "client.toml"
                        config, replacements = re.subn(
                            r'^endpoint = "127\.0\.0\.1:' + str(ready["port"]) + r'"$',
                            'endpoint = "127.0.0.1:' + str(port) + '"',
                            config_path.read_text(), flags=re.M)
                        assert replacements == 1, "Fixture endpoint was not the designated local server"
                        config_path.write_text(config)
                    command = ["docker", "run", "--rm", "--cidfile", str(fixture / "container.id"), "--read-only",
                               "--platform", "linux/amd64", "--cpus", "2", "--memory", "1g", "--cap-drop", "ALL",
                               "--security-opt", "no-new-privileges", "--mount", f"type=bind,source={probe.parent},target=/probe,readonly",
                               "--mount", f"type=bind,source={fixture},target=/fixture", image["Id"],
                               "/probe/native_media_probe", "/fixture", "host.lima.internal"]
                    if args.unreachable_port:
                        command.append("--expect-unreachable")
                    with (fixture / "guest.log").open("w") as guest_log:
                        child = subprocess.Popen(command, stdout=guest_log, stderr=subprocess.STDOUT, start_new_session=True)
                        try:
                            guest_code = child.wait(timeout=120)
                        finally:
                            stop(child)
                            if (fixture / "container.id").exists():
                                cid = (fixture / "container.id").read_text().strip()
                                assert re.fullmatch("[0-9a-f]{64}", cid)
                                subprocess.run(["docker", "rm", "--force", cid], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                    (fixture / "done").write_text("complete\n")
                    server_code = process.wait(timeout=30)
                finally:
                    stop(process)
                    if proxy is not None:
                        proxy.close()
                    if blackhole is not None:
                        try:
                            blackhole.setblocking(False)
                            while True:
                                try:
                                    blackhole.recvfrom(65535)
                                    blocked_datagrams += 1
                                except BlockingIOError:
                                    break
                        finally:
                            blackhole.close()
            guest_rows = [json.loads(line) for line in (fixture / "guest.log").read_text().splitlines() if line.startswith("{")]
            server_raw = (fixture / "server.log").read_text()
            server_rows = [json.loads(line.split("distinct-media-server-witness ", 1)[1]) for line in server_raw.splitlines() if "distinct-media-server-witness " in line]
            row = {"algorithm": algorithm, "guest_exit_code": guest_code, "server_exit_code": server_code,
                   "elapsed_seconds": time.monotonic() - started,
                   "guest_witness": guest_rows, "server_witness": server_rows,
                   "guest_log_sha256": digest(fixture / "guest.log"), "server_log_sha256": digest(fixture / "server.log")}
            row["passed"] = guest_code == server_code == 0 and len(guest_rows) == len(server_rows) == 1
            if proxy is not None:
                row["transport_fault"] = proxy.stats
                row["passed"] = (row["passed"] and proxy.stats["worker_joined"]
                                 and proxy.stats["foreign_datagrams"] == 0
                                 and all(proxy.stats[d]["dropped"] >= 1 for d in ("guest_to_server", "server_to_guest")))
            if row["passed"]:
                gw, sw = guest_rows[0], server_rows[0]
                row["passed"] = (gw["algorithm"] == sw["algorithm"] == algorithm and gw["os"] == "linux"
                                 and gw["architecture"] == "x86_64" and sw["physical_peers_drained"]
                                 and bool(re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", server_raw)))
                if args.unreachable_port:
                    row["observed_unanswered_datagrams"] = blocked_datagrams
                    row["passed"] = (row["passed"] and blocked_datagrams > 0 and gw["open_refused"] == "timeout"
                                     and gw["no_offer_submitted"] and sw["durable_native_offers"] == 1
                                     and sw["guest_open_refused"] and sw["post_timeout_signed_heartbeat"]
                                     and not (fixture / "guest-ready").exists()
                                     and not (fixture / "guest-received.pcmu").exists())
                else:
                    row["decoded_at_host"] = decoded_tone(fixture / "host-received.pcmu", 660, 440, args.ffmpeg)
                    row["decoded_at_guest"] = decoded_tone(fixture / "guest-received.pcmu", 440, 660, args.ffmpeg)
                    row["passed"] = (row["passed"] and gw["owned_peer_shutdown"] and sw["durable_native_offers"] == 2
                                     and row["decoded_at_host"]["passed"] and row["decoded_at_guest"]["passed"])
            report["profiles"].append(row)
            print(json.dumps(row), flush=True)
            assert row["passed"], "Distinct-network media profile failed"
    except Exception as error:
        report["failure"] = {"type": type(error).__name__, "message": str(error)}
    finally:
        report["source_inputs_unchanged"] = HELPERS["sources"]() == before
        report["build_inputs_unchanged"] = all(
            (ROOT / name).is_file() and digest(ROOT / name) == value
            for inputs in (server_inputs, guest["source_inputs_sha256"])
            for name, value in inputs.items())
        report["executables_unchanged"] = digest(binary) == report["server_executable_sha256"] and digest(probe) == report["guest_executable_sha256"]
        report["passed"] = (len(report["profiles"]) == 2 and all(row["passed"] for row in report["profiles"])
                            and report["source_inputs_unchanged"] and report["build_inputs_unchanged"]
                            and report["executables_unchanged"])
        report["finished_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        (out / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps({"passed": report["passed"], "summary": str(out / "summary.json")}), flush=True)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
