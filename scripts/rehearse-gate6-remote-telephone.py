#!/usr/bin/env python3
"""Owned Linux SIP/RTP peer through an observed UDP NAT fixture to a Mac host."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import random
import runpy
import secrets
import select
import signal
import socket
import sqlite3
import subprocess
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / ".local/gate-6/telephone-remote"
IMAGE = "sha256:533cdd9c6983863f0c7ae50b3a4f9a6157bfe5b88f84b06dd44a5c61e3234bd0"
CASE = "canonical_native_connections::consultation::groups::replacement::telephone::initial::remote::distinct_network_telephone_server_child"
HELPERS = runpy.run_path(str(ROOT / "scripts/rehearse-gate6-native-media.py"))


def digest(path):
    with Path(path).open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def private_json(path, value):
    with open(path, "x", opener=lambda name, flags: os.open(name, flags, 0o600)) as out:
        json.dump(value, out, indent=2)
        out.write("\n")


def reserve(ip, count):
    for _ in range(64):
        first = random.SystemRandom().randint(20000, 59984)
        held = []
        try:
            for port in range(first, first + count):
                sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                held.append(sock)
                sock.bind((ip, port))
            return first, held
        except OSError:
            for sock in held:
                sock.close()
    raise RuntimeError("No bounded owned UDP range available")


class PeerNat:
    """Two fixed authenticated guest mappings; no arbitrary target forwarding."""
    def __init__(self, directory, nonce, host_ip, server_sip, media_start):
        self.directory, self.nonce, self.host_ip = directory, nonce, host_ip
        self.targets = {"sip": server_sip, "rtp": None}
        self.media_start = media_start
        sip = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sip.bind(("0.0.0.0", 0))
        _, media = reserve("0.0.0.0", 2)
        self.sockets = {"sip": sip, "rtp": media[0], "rtcp": media[1]}
        self.front = {name: sock.getsockname()[1] for name, sock in self.sockets.items()}
        self.peers = {}
        self.calls = set()
        self.cancel = threading.Event()
        self.stats = {name: {"guest_to_server": 0, "server_to_guest": 0} for name in self.sockets}
        self.stats.update(foreign_datagrams=0, journal_before_first_invite=False)
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        try:
            while not self.cancel.is_set():
                for sock in select.select(list(self.sockets.values()), [], [], .02)[0]:
                    name = next(key for key, value in self.sockets.items() if value is sock)
                    data, source = sock.recvfrom(16384)
                    if name == "rtcp":
                        target = self.targets["rtp"]
                        if target is None or source[0] != target[0] or source[1] not in [target[1], target[1] + 1]:
                            self.stats["foreign_datagrams"] += 1
                            continue
                        assert len(data) >= 8 and data[0] >> 6 == 2
                        self.stats["owned_rtcp_sink_packets"] = self.stats.get("owned_rtcp_sink_packets", 0) + 1
                        continue
                    if data == f"CONNECT-G6-PHONE {self.nonce} {name}".encode():
                        assert name not in self.peers or self.peers[name] == source
                        self.peers[name] = source
                        sock.sendto(b"mapped", source)
                        continue
                    if source == self.peers.get(name):
                        target, direction = self.targets[name], "guest_to_server"
                    elif source == self.targets[name] and name in self.peers:
                        target, direction = self.peers[name], "server_to_guest"
                        if name == "sip" and data.startswith(b"INVITE "):
                            head, body = data.decode().split("\r\n\r\n", 1)
                            headers = dict((line.split(":", 1)[0].lower(), line.split(":", 1)[1].strip()) for line in head.split("\r\n")[1:])
                            call = headers["call-id"]
                            if call not in self.calls:
                                assert not self.calls, "more than one original call"
                                ready = json.loads((self.directory / "ready.json").read_text())
                                with sqlite3.connect(f'file:{ready["database"]}?mode=ro', uri=True) as db:
                                    durable = db.execute("SELECT EXISTS(SELECT 1 FROM initial_telephone_operations i JOIN operation_effects e ON e.id=i.effect_id WHERE i.signaling_started=1 AND i.runtime_connection_id IS NOT NULL AND i.sip_call_id=? AND e.state='submitting' AND e.attempt=1 AND e.provider_ref=? AND e.correlation_key=i.operation_record_id)", (call, call)).fetchone()[0]
                                assert durable, "initial call was not journaled before network forwarding"
                                self.stats["journal_before_first_invite"] = True
                                self.calls.add(call)
                            addresses = [line.removeprefix("c=IN IP4 ") for line in body.splitlines() if line.startswith("c=IN IP4 ")]
                            audio = [line.split() for line in body.splitlines() if line.startswith("m=audio ")]
                            assert addresses == [self.host_ip] and len(audio) == 1
                            port = int(audio[0][1])
                            assert self.media_start <= port <= self.media_start + 15
                            expected = (self.host_ip, port)
                            assert self.targets["rtp"] in [None, expected]
                            self.targets["rtp"] = expected
                    else:
                        self.stats["foreign_datagrams"] += 1
                        continue
                    assert target is not None
                    sock.sendto(data, target)
                    self.stats[name][direction] += 1
        except Exception as error:
            self.stats["error"] = type(error).__name__ + ": " + str(error)

    def close(self):
        self.cancel.set()
        self.thread.join(timeout=1)
        assert not self.thread.is_alive(), "original peer NAT worker did not join"
        for sock in self.sockets.values():
            sock.close()
        self.stats["worker_joined"] = True
        self.stats["unique_calls"] = len(self.calls)


def stop(process):
    if process is not None and process.poll() is None:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()
        return True
    return False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server-build", type=Path, required=True)
    parser.add_argument("--server-inputs", type=Path, required=True)
    parser.add_argument("--host-media-ip", required=True, help="Explicit concrete IPv4 interface on this owned Mac")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--ffmpeg", default="ffmpeg")
    args = parser.parse_args()
    socket.inet_pton(socket.AF_INET, args.host_media_ip)
    assert not args.host_media_ip.startswith(("127.", "0.", "224."))
    BASE.mkdir(mode=0o700, exist_ok=True)
    out = args.output.absolute()
    assert out.parent.resolve().is_relative_to(BASE.resolve())
    out.mkdir(mode=0o700, exist_ok=False)
    build = json.loads(args.server_build.read_text())
    inputs = json.loads(args.server_inputs.read_text())
    assert build["exit_code"] == 0 and not build["changed_inputs"] and inputs
    assert all(digest(ROOT / name) == value for name, value in inputs.items())
    binary = Path(build["artifact"]["sealed"])
    assert digest(binary) == build["artifact"]["sha256"]
    assert CASE + ": test" in subprocess.check_output([str(binary), "--list"], text=True).splitlines()
    image = json.loads(subprocess.check_output(["docker", "image", "inspect", IMAGE]))[0]
    assert image["Id"] == IMAGE and image["Architecture"] == "arm64" and image["Os"] == "linux"
    def sources():
        result = HELPERS["HELPERS"]["sources"]()
        for folder in [ROOT / "scripts/telephone"]:
            for path in sorted(folder.rglob("*.py")):
                result[str(path.relative_to(ROOT))] = digest(path)
        result[str(Path(__file__).relative_to(ROOT))] = digest(__file__)
        return result
    before = sources()
    report = {"passed": False, "scope": "Owned Linux arm64 Python SIP/RTP peer across the local Lima VM network to Mac Remote SIP+PCMU and a public native SDK peer. Explicit fixed peer-side UDP NAT translations, real opposite decoded tones and original drains; no direct WAN/provider/PSTN, TURN/browser/native x86 performance or full G6-054 qualification.",
              "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
              "image_id": IMAGE, "server_sha256": digest(binary), "source_inputs_sha256": before,
              "started_at": datetime.datetime.now(datetime.timezone.utc).isoformat(), "profiles": []}
    try:
        for algorithm in ["EdDSA", "ES256"]:
            directory = out / algorithm
            directory.mkdir(mode=0o700)
            for name, hz in [("host", 440), ("guest", 660)]:
                subprocess.run([args.ffmpeg, "-v", "error", "-f", "lavfi", "-i", f"sine=frequency={hz}:sample_rate=8000:duration=4", "-ac", "1", "-c:a", "pcm_mulaw", "-f", "mulaw", str(directory / f"{name}-send.pcmu")], check=True, timeout=15)
            sip_port, sip_held = reserve(args.host_media_ip, 1)
            media_start, media_held = reserve(args.host_media_ip, 16)
            server_sip = (args.host_media_ip, sip_port)
            nat = PeerNat(directory, secrets.token_hex(32), args.host_media_ip, server_sip, media_start)
            private_json(directory / "guest-input.json", {"nonce": nat.nonce, "gateway": "192.168.5.2", "host_media_ip": args.host_media_ip, "sip_front": nat.front["sip"], "rtp_front": nat.front["rtp"], "media_port_start": media_start})
            private_json(directory / "server-input.json", {"algorithm": algorithm, "host_media_ip": args.host_media_ip, "server_sip": f"{args.host_media_ip}:{sip_port}", "peer_sip": f'{args.host_media_ip}:{nat.front["sip"]}', "media_port_start": media_start})
            guest = server = None
            row = {"algorithm": algorithm, "passed": False}
            try:
                command = ["docker", "run", "--rm", "--cidfile", str(directory / "container-id"), "--read-only", "--cpus", "2", "--memory", "1g", "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--tmpfs", "/tmp:rw,size=64m", "--mount", f"type=bind,source={ROOT},target={ROOT},readonly", "--mount", f"type=bind,source={directory},target=/fixture", IMAGE, "python3", str(ROOT / "scripts/telephone/linux_remote_phone.py"), "/fixture"]
                with (directory / "guest.log").open("w") as guest_log, (directory / "server.log").open("w") as server_log:
                    guest = subprocess.Popen(command, stdout=guest_log, stderr=subprocess.STDOUT, start_new_session=True)
                    until = time.monotonic() + 15
                    while not (directory / "guest-ready").exists():
                        assert guest.poll() is None and "error" not in nat.stats, "guest mapping failed"
                        assert time.monotonic() < until, "guest mapping deadline"
                        time.sleep(.02)
                    for held in sip_held + media_held:
                        held.close()
                    env = dict(os.environ, CONNECT_G6_REMOTE_PHONE_DIR=str(directory), TMPDIR=str(directory))
                    server = subprocess.Popen([str(binary), CASE, "--exact", "--ignored", "--nocapture", "--test-threads=1"], cwd=ROOT, env=env, stdout=server_log, stderr=subprocess.STDOUT, start_new_session=True)
                    row["server_exit_code"] = server.wait(timeout=75)
                    if row["server_exit_code"] != 0:
                        (directory / "host-stopped").write_text("failed\n")
                    row["guest_exit_code"] = guest.wait(timeout=15)
                row["guest"] = json.loads((directory / "guest-summary.json").read_text())
                raw = (directory / "server.log").read_text()
                row["server_witness"] = [json.loads(line.split("remote-telephone-server-witness ", 1)[1]) for line in raw.splitlines() if "remote-telephone-server-witness " in line]
                assert row["server_exit_code"] == row["guest_exit_code"] == 0 and row["guest"]["passed"]
                assert len(row["server_witness"]) == 1 and "test result: ok. 1 passed; 0 failed; 0 ignored;" in raw
                row["host_decoded"] = HELPERS["decoded_tone"](directory / "host-received.pcmu", 660, 440, args.ffmpeg)
                row["guest_decoded"] = HELPERS["decoded_tone"](directory / "guest-received.pcmu", 440, 660, args.ffmpeg)
                assert row["host_decoded"]["passed"] and row["guest_decoded"]["passed"]
                row["passed"] = True
            finally:
                row["forced_server_cleanup"] = stop(server)
                row["forced_guest_cleanup"] = stop(guest)
                cidfile = directory / "container-id"
                if cidfile.exists():
                    cid = cidfile.read_text().strip()
                    assert len(cid) == 64 and all(c in "0123456789abcdef" for c in cid)
                    subprocess.run(["docker", "rm", "--force", cid], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                nat.close()
                for held in sip_held + media_held:
                    held.close()
                row["peer_nat"] = nat.stats
                row["passed"] = bool(row["passed"] and not row["forced_server_cleanup"] and not row["forced_guest_cleanup"] and nat.stats["journal_before_first_invite"] and nat.stats["unique_calls"] == 1 and nat.stats["foreign_datagrams"] == 0 and "error" not in nat.stats and all(nat.stats["rtp"][k] >= 50 for k in ["guest_to_server", "server_to_guest"]))
                row["logs_sha256"] = {name: digest(directory / name) for name in ["server.log", "guest.log"] if (directory / name).exists()}
                report["profiles"].append(row)
                print(json.dumps({"algorithm": algorithm, "passed": row["passed"]}), flush=True)
            assert row["passed"]
    except Exception as error:
        report["failure"] = {"type": type(error).__name__, "message": str(error)}
    finally:
        report["source_inputs_unchanged"] = sources() == before
        report["build_inputs_unchanged"] = all(digest(ROOT / name) == value for name, value in inputs.items())
        report["server_unchanged"] = digest(binary) == report["server_sha256"]
        report["passed"] = bool(len(report["profiles"]) == 2 and all(row["passed"] for row in report["profiles"]) and report["source_inputs_unchanged"] and report["build_inputs_unchanged"] and report["server_unchanged"])
        report["finished_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        private_json(out / "summary.json", report)
        print(json.dumps({"passed": report["passed"], "summary": str(out / "summary.json")}), flush=True)
    raise SystemExit(0 if report["passed"] else 1)


if __name__ == "__main__":
    main()
