#!/usr/bin/env python3
"""Owned loopback TURN REST expiry + signed WSS decoded Opus/VP8 relay fixture.

Requires an already source-bound gate6_canonical_webtransport test executable,
coturn4.18 and existing synthetic AV fixtures. No account/provider/remote lane.
Private configs, synthetic credentials, logs and decoder outputs stay in .local.
"""
import argparse
import base64
import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import signal
import socket
import struct
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
PREFIX = "canonical_websocket_control::webrtc::subscriptions::av::"


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def private_write(path, value):
    with open(path, "x", opener=lambda name, flags: os.open(name, flags, 0o600)) as stream:
        stream.write(value)


def reserve_range(count):
    # Keep the entire range reserved until immediately before the owned launch.
    for _ in range(100):
        start = 20000 + secrets.randbelow(40000 - count)
        held = []
        try:
            for port in range(start, start + count):
                sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                held.append(sock)
                sock.bind(("127.0.0.1", port))
            return start, held
        except OSError:
            for sock in held:
                sock.close()
    raise RuntimeError("No bounded loopback range available")


def ready(child, port, transport):
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        assert child.poll() is None, "Owned coturn exited during startup"
        try:
            if transport == "tcp":
                with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                    return
            else:
                transaction = secrets.token_bytes(12)
                request = struct.pack("!HHI12s", 1, 0, 0x2112A442, transaction)
                with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
                    sock.settimeout(0.2)
                    sock.sendto(request, ("127.0.0.1", port))
                    response, _ = sock.recvfrom(2048)
                if len(response) >= 20 and response[8:20] == transaction:
                    return
        except OSError:
            pass
        time.sleep(0.05)
    raise RuntimeError("Owned coturn readiness deadline")


def bounded_child(command, env, log, timeout):
    started = time.monotonic()
    with log.open("xb") as stream:
        child = subprocess.Popen(command, cwd=ROOT, env=env, stdout=stream,
                                 stderr=subprocess.STDOUT, start_new_session=True)
        timed_out = forced = False
        try:
            child.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(child.pid, signal.SIGTERM)
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                forced = True
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
    return {"exit_code": child.returncode, "timed_out": timed_out, "forced_kill": forced,
            "seconds": time.monotonic() - started, "log_sha256": digest(log)}


def run(args):
    manifest_path = args.source_manifest.resolve(strict=True)
    source_inputs = json.loads(manifest_path.read_text())
    assert len(source_inputs) > 100 and all(
        not Path(name).is_absolute() and ".." not in Path(name).parts
        and re.fullmatch("[0-9a-f]{64}", value) for name, value in source_inputs.items()
    ), "Expected the original build's repository-relative SHA256 input manifest"
    def changed_sources():
        return [name for name, value in source_inputs.items()
                if not (ROOT / name).is_file() or digest(ROOT / name) != value]
    assert not changed_sources(), "Source inputs differ from the original build"
    binary = args.binary.resolve(strict=True)
    fixtures = args.fixtures.resolve(strict=True)
    decoder = Path(shutil.which(args.ffmpeg) or args.ffmpeg).resolve(strict=True)
    turnserver = Path(shutil.which(args.turnserver) or args.turnserver).resolve(strict=True)
    base = args.output.resolve()
    assert base.is_relative_to(ROOT / ".local"), "Private output must be under repository .local"
    base.mkdir(parents=True, mode=0o700, exist_ok=False)
    assert args.ice_policy != "all" or args.transport == "udp", "Shared STUN/TURN lane requires UDP control"
    if args.client == "browser":
        assert args.ice_policy == "relay", "Browser fixture requires explicit relay-only policy"
        test = "canonical_webrtc_browser::canonical_chrome_relay_original_peer_ice_restart_clean_" + args.algorithm.lower()
    else:
        test = PREFIX + ("configured_all_shared_stun_turn_decoded_duplex" if args.ice_policy == "all"
                         else f"configured_relay_{args.transport}_expiry_and_decoded_duplex")
    listing = subprocess.check_output([str(binary), "--list"], text=True)
    assert f"{test}: test" in listing, "Wrong integration test partition or stale executable"
    version = subprocess.check_output([str(turnserver), "--version"], text=True).strip()
    assert version == "4.18.0", "Requalify allocation-drain oracle for another coturn version"
    report = {"format": 1, "passed": False, "test": test, "transport": args.transport, "ice_policy": args.ice_policy,
              "client": args.client,
              "scope": ("Owned loopback browser relay-only same-peer ICE recovery with unchanged clean60 AV quality; no distinct-network/SIP/remote qualification."
                        if args.client == "browser" else "Owned loopback TURN REST expiry and signed WSS/Store/Core Opus/VP8 duplex under the explicit All/shared-STUN-TURN or relay-only policy; no distinct-network/browser/SIP/remote qualification."),
              "source_manifest_sha256": digest(manifest_path), "source_count": len(source_inputs),
              "coturn_version": version, "binary_sha256": digest(binary),
              "turnserver_sha256": digest(turnserver), "decoder_sha256": digest(decoder),
              "runner_sha256": digest(__file__), "fixtures_sha256": {
                  name: digest(fixtures / name) for name in ("red.vp8", "blue.vp8")}}
    media_start, media = reserve_range(8)
    relay_start, relay = reserve_range(16)
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.bind(("127.0.0.1", 0))
    port = listener.getsockname()[1]
    shared = secrets.token_hex(32)
    # Coturn's documented REST format uses HMAC-SHA1, not RFC7635 OAuth.
    def credentials(expiry):
        user = f"{expiry}:synthetic-g6"
        password = base64.b64encode(hmac.new(shared.encode(), user.encode(), hashlib.sha1).digest()).decode()
        return user, password
    user, password = credentials(int(time.time()) + 110)
    expired_user, expired_password = credentials(int(time.time()) - 60)
    config = base / "turnserver.conf"
    private_write(config, "\n".join([
        "listening-ip=127.0.0.1", "relay-ip=127.0.0.1", f"listening-port={port}",
        f"min-port={relay_start}", f"max-port={relay_start + 15}", "relay-threads=1",
        "realm=gate6.invalid", "use-auth-secret", f"static-auth-secret={shared}",
        "allow-loopback-peers", "allowed-peer-ip=127.0.0.1",
        "denied-peer-ip=0.0.0.0-255.255.255.255", "no-multicast-peers",
        "no-tls", "no-tcp-relay", "fingerprint", "verbose",
        "no-tcp" if args.transport == "udp" else "no-udp",
        "user-quota=8", "total-quota=8", "drain-min-allocations=0",
        f"userdb={base / 'turn.sqlite'}", f"pidfile={base / 'turn.pid'}",
        "log-file=stdout", "simple-log", "new-log-timestamp=false", ""]))
    manifest = base / "fixture.json"
    private_write(manifest, json.dumps({"listener_port": port,
        "media_port_start": media_start, "media_port_end": media_start + 7,
        "all_policy": args.ice_policy == "all", "transport":args.transport}) + "\n")
    env = os.environ.copy()
    refs = base / "secret-references.toml"
    private_write(refs, 'version = 1\n' + ''.join(
        f'\n[references.{name}]\nstorage = "environment"\nvariable = "{name}"\n'
        for name in ("CONNECT_GATE6_TURN_USER", "CONNECT_GATE6_TURN_PASSWORD")))
    env.update(CONNECT_GATE6_TURN_FIXTURE=str(manifest), CONNECT_SECRET_REFERENCES=str(refs),
               CONNECT_GATE6_TURN_USER=user, CONNECT_GATE6_TURN_PASSWORD=password,
               CONNECT_GATE6_TURN_EXPIRED_USER=expired_user, CONNECT_GATE6_TURN_EXPIRED_PASSWORD=expired_password,
               CONNECT_GATE6_AV_FIXTURE_DIR=str(fixtures), CONNECT_GATE6_FFMPEG=str(decoder))
    if args.client == "browser":
        browser_output = base / "browser"
        browser_output.mkdir(mode=0o700)
        env["CONNECT_GATE6_CANONICAL_BROWSER_ARTIFACTS"] = str(browser_output)
    owned = None
    held = [listener, *media, *relay]
    try:
        for sock in held:
            sock.close()
        with (base / "coturn.log").open("xb") as log:
            owned = subprocess.Popen([str(turnserver), "-c", str(config)], cwd=base,
                                     stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            report["server_pid"] = owned.pid
            ready(owned, port, args.transport)
            if args.transport == "tcp":
                with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
                    probe.settimeout(0.3)
                    probe.sendto(struct.pack("!HHI12s", 1, 0, 0x2112A442, secrets.token_bytes(12)), ("127.0.0.1", port))
                    try:
                        probe.recvfrom(2048)
                        raise AssertionError("TCP lane unexpectedly has a UDP TURN listener")
                    except TimeoutError:
                        report["udp_listener_unreachable"] = True
            step = bounded_child([str(binary), test, "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                                 env, base / "test.log", 120 if args.client == "browser" else 90)
            text = (base / "test.log").read_text(errors="replace")
            step["results"] = re.findall(r"^test result:.*$", text, re.M)
            step["panic_locations"] = re.findall(r"panicked at ([^\n]+)", text)
            report["execution"] = step
            # Coturn exits drain mode only when its active allocation count is zero.
            # Do this while the daemon still owns its sockets, before killing it.
            report["drain_observation_version"] = 2
            report["allocation_drain_limit_seconds"] = 10
            report["post_drain_join_limit_seconds"] = 10
            started_drain = time.monotonic()
            owned.send_signal(signal.SIGUSR1)
            # Coturn's listener runs five-second event-loop ticks, including
            # another dispatch after reporting drain completion. Allocation
            # release and joining that original process are distinct facts.
            while True:
                observed = (base / "coturn.log").read_text(errors="replace")
                zero = "Drain complete (0 allocations remaining, threshold 0)" in observed
                if zero:
                    report["allocation_drain_seconds"] = time.monotonic() - started_drain
                    break
                assert owned.poll() is None, "Coturn exited without its zero-allocation drain observation"
                assert time.monotonic() - started_drain < 10, "Coturn allocation drain deadline"
                time.sleep(0.05)
            assert report["allocation_drain_seconds"] <= 10, "Coturn allocation drain deadline"
            started_join = time.monotonic()
            owned.wait(timeout=10)
            report["post_drain_join_seconds"] = time.monotonic() - started_join
            report["drain_exit_code"] = owned.returncode
            assert owned.returncode == 0, "Coturn allocation drain did not finish cleanly"
        coturn = (base / "coturn.log").read_text(errors="replace")
        report["coturn_log_sha256"] = digest(base / "coturn.log")
        report["allocation_successes"] = len(re.findall(r"ALLOCATE processed, success", coturn))
        assert report["allocation_successes"] >= 4, "Missing four actual client/server TURN allocations"
        assert "drain" in coturn.lower(), "Missing allocation drain observation"
        assert step["exit_code"] == 0 and not step["timed_out"] and not step["forced_kill"], "Relay fixture failed; private test.log retained; zero-allocation drain checked separately"
        assert len(step["results"]) == 1 and "1 passed; 0 failed; 0 ignored;" in step["results"][0]
        report["passed"] = True
    finally:
        for sock in held:
            sock.close()
        if owned is not None and owned.poll() is None:
            report["cleanup_required_sigterm"] = True
            os.killpg(owned.pid, signal.SIGTERM)
            try:
                owned.wait(timeout=5)
            except subprocess.TimeoutExpired:
                report["cleanup_required_sigkill"] = True
                os.killpg(owned.pid, signal.SIGKILL)
                owned.wait()
        report["server_joined"] = owned is not None and owned.poll() is not None
        report["changed_source_inputs"] = changed_sources()
        report["passed"] = report["passed"] and not report["changed_source_inputs"]
        private_write(base / "summary.json", json.dumps(report, indent=2) + "\n")
        print(json.dumps(report), flush=True)
    assert report["passed"]


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--source-manifest", type=Path, required=True)
    parser.add_argument("--fixtures", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--transport", choices=("udp", "tcp"), required=True)
    parser.add_argument("--ice-policy", choices=("all", "relay"), default="relay")
    parser.add_argument("--client", choices=("native", "browser"), default="native")
    parser.add_argument("--algorithm", choices=("EdDSA", "ES256"), default="EdDSA")
    parser.add_argument("--ffmpeg", default="ffmpeg")
    parser.add_argument("--turnserver", default="turnserver")
    run(parser.parse_args())
