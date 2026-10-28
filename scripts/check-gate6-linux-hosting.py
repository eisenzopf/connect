#!/usr/bin/env python3
"""Native Ubuntu x86 isolated configuration/HTTPS checks; no account/live lanes.

Raw compiler/test output and synthetic state stay private. Only source hashes,
tool versions, exit/result counts and artifact hashes enter uploadable evidence.
This is not installation, remote media or whole Gate 6 qualification.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / ".local/gate6-linux-ci"
EVIDENCE = BASE / "evidence"
PRIVATE = BASE / "private-logs"
sys.path.insert(0, str(ROOT / "scripts/lib"))
from source_build_cache import MANIFEST, snapshot


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def output(name, value):
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a") as stream:
            stream.write(f"{name}={value}\n")


def sources():
    files = {name: {k: v for k, v in value.items() if k != "mtime_ns"} for name, value in snapshot(ROOT).items()}
    for path in sorted((ROOT / ".deps").rglob("*")):
        if ".git" in path.parts or path.is_symlink() or not path.is_file():
            continue
        files[str(path.relative_to(ROOT))] = {"sha256": digest(path), "mode": path.stat().st_mode & 0o777}
    return files


def preflight():
    assert platform.system() == "Linux" and platform.machine() == "x86_64", "Native Linux x86_64 profile required"
    cpuinfo = Path("/proc/cpuinfo").read_text()
    assert re.search(r"^vendor_id\s*:", cpuinfo, re.M), "An x86 kernel is required; an emulated amd64 userland on an ARM kernel is insufficient"
    release = platform.freedesktop_os_release()
    assert release.get("ID") == "ubuntu" and release.get("VERSION_ID") == "24.04", "Ubuntu24.04 profile required"
    rust = subprocess.check_output(["rustc", "-Vv"], text=True)
    assert "rustc 1.97.1 " in rust and "host: x86_64-unknown-linux-gnu" in rust
    native = {
        "rust": rust,
        "clang": subprocess.check_output(["clang", "--version"], text=True),
        "packages": subprocess.check_output(["dpkg-query", "-W", "libopus-dev", "libssl-dev", "libc6-dev", "clang", "g++", "binutils", "pkg-config", "cmake"], text=True),
    }
    key = hashlib.sha256(json.dumps(native, sort_keys=True).encode()).hexdigest()
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    (EVIDENCE / "preflight.json").write_text(json.dumps({"os": release, "platform": platform.platform(),
        "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "runner_image_version": os.environ.get("ImageVersion"), "native_key": key, "toolchain": native,
        "scope": "Native Ubuntu24.04x86 CI fixtures only. No named remote/provider, model account, media/performance, installation or full acceptance qualification."}, indent=2) + "\n")
    output("native_key", key)


def run():
    preflight()
    PRIVATE.mkdir(parents=True, exist_ok=True, mode=0o700)
    temporary = BASE / "private-tmp"
    temporary.mkdir(exist_ok=True, mode=0o700)
    os.environ["TMPDIR"] = str(temporary)
    before = sources()
    (EVIDENCE / "source-inputs.json").write_text(json.dumps(before, indent=2) + "\n")
    report = {"passed": False, "started_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "steps": [], "runner_sha256": digest(__file__), "source_count": len(before),
              "source_inputs_sha256": digest(EVIDENCE / "source-inputs.json"),
              "scope": "Bounded configuration/security, ICE/DTLS owned-close and actual HTTPS surface/CLI fixtures. No actual TURN allocation, remote media, SIP, browser/performance or installation qualification."}

    def execute(name, command, timeout):
        log = PRIVATE / (name + ".log")
        started = time.monotonic()
        with log.open("wb") as stream:
            child = subprocess.Popen(command, cwd=ROOT, stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
            timed = False
            forced = False
            try:
                child.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                timed = True
                os.killpg(child.pid, signal.SIGTERM)
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    forced = True
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait(timeout=10)
        text = log.read_text(errors="replace")
        summary = [line for line in text.splitlines() if line.startswith("test result:")]
        step = {"name": name, "exit_code": child.returncode, "timed_out": timed, "forced_kill": forced,
                "seconds": time.monotonic() - started, "log_sha256": digest(log), "test_results": summary}
        step["panic_locations"] = [
            {"file": location[0], "line": int(location[1]), "column": int(location[2])}
            for location in re.findall(r"^thread '[^']+' panicked at ([^\n]+?):([0-9]+):([0-9]+):", text, re.M)
        ][:16]
        if child.returncode != 0:
            diagnostics = []
            for line in text.splitlines():
                if not line.startswith("{"):
                    continue
                try:
                    item = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if item.get("reason") == "compiler-message" and item["message"]["level"] == "error":
                    message = item["message"]
                    diagnostics.append({"code": (message.get("code") or {}).get("code"), "message": message["message"],
                                        "locations": [{k: span[k] for k in ("file_name", "line_start", "column_start")} for span in message["spans"]]})
            step["compiler_diagnostics"] = diagnostics
        report["steps"].append(step)
        print(json.dumps(step), flush=True)
        assert child.returncode == 0 and not timed and not forced, f"Phase failed: {name}; raw output retained privately"
        return text

    artifacts = {}
    try:
        for name, target in (("library", ["--lib"]), ("hosting", ["--test", "gate6_canonical_runtime"])):
            text = execute("compile-" + name, ["cargo", "test", "--locked", "--offline", "--jobs", "2", "-p", "connect-server",
                           "--features", "test-support", *target, "--no-run", "--message-format=json"], 3600)
            rows = [json.loads(line) for line in text.splitlines() if line.startswith("{")]
            assert any(row.get("reason") == "build-finished" and row.get("success") for row in rows)
            wanted = "connect_server" if name == "library" else "gate6_canonical_runtime"
            executable = [row["executable"] for row in rows if row.get("reason") == "compiler-artifact"
                          and row["target"]["name"] == wanted and row["profile"]["test"] and row.get("executable")]
            assert len(executable) == 1
            artifacts[name] = executable[0]
        assert sources() == before, "Sources changed during compilation"
        # A failed compile must never stamp source times as a completed build.
        MANIFEST.write_text(json.dumps({"format": 1, "files": snapshot(ROOT)}, sort_keys=True) + "\n")
        report["compiled_artifacts"] = {name: digest(path) for name, path in artifacts.items()}
        for name, binary, pattern, expected in (
            ("addressing", "library", "canonical_webrtc::addressing::tests::", 4),
            ("config", "library", "canonical_webrtc::config::tests::", 4),
            ("owned-dtls", "library", "canonical_webrtc::tests::store_bound_peer_completes_actual_dtls_and_original_close_releases_capacity", 1),
            ("https-hosting", "hosting", "canonical_https_hosting::", 7),
        ):
            text = execute(name, [artifacts[binary], pattern, "--test-threads=1"], 90)
            counts = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;", text)
            assert len(counts) == 1 and int(counts[0][0]) >= expected and counts[0][1:] == ("0", "0"), "Missing/skipped fixture coverage"
        report["behavior_passed"] = True
    finally:
        report["source_inputs_unchanged"] = sources() == before
        report["passed"] = report.get("behavior_passed", False) and report["source_inputs_unchanged"]
        output("source_unchanged", str(report["source_inputs_unchanged"]).lower())
        report["finished_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        (EVIDENCE / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    assert report["passed"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preflight", action="store_true")
    args = parser.parse_args()
    preflight() if args.preflight else run()


if __name__ == "__main__":
    main()
