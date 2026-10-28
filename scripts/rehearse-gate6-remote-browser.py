#!/usr/bin/env python3
"""Synthetic Linux browsers to an owned Mac Remote host; no devices/providers."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import random
import re
import runpy
import signal
import socket
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
HELPERS = runpy.run_path(str(ROOT / "scripts/rehearse-gate6-fifty-participants.py"))
CASE = "canonical_webrtc_browser::remote::distinct_network_browser_server_child"
IMAGE = "sha256:533cdd9c6983863f0c7ae50b3a4f9a6157bfe5b88f84b06dd44a5c61e3234bd0"


def digest(path):
    with Path(path).open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def stop(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()


def ports():
    for _ in range(32):
        first = random.SystemRandom().randint(20000, 59984)
        held = []
        try:
            for port in range(first, first + 16):
                sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                held.append(sock)
                sock.bind(("0.0.0.0", port))
            return first
        except OSError:
            continue
        finally:
            for sock in held:
                sock.close()
    raise RuntimeError("No owned 16-port fixture range available")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server-build", type=Path, required=True)
    parser.add_argument("--server-inputs", type=Path, required=True)
    parser.add_argument("--algorithm", choices=("EdDSA", "ES256"), required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--trace-audio", action="store_true", help="Bounded private source/mixer/publication/write diagnostics; not normal-profile qualification")
    args = parser.parse_args()
    base = ROOT / ".local/gate-6/browser-remote"
    base.mkdir(mode=0o700, exist_ok=True)
    out = args.output.absolute()
    assert out.parent.resolve().is_relative_to(base.resolve())
    out.mkdir(mode=0o700, exist_ok=False)
    build = json.loads(args.server_build.read_text())
    inputs = json.loads(args.server_inputs.read_text())
    assert build["exit_code"] == 0 and not build["changed_inputs"] and inputs
    assert all(digest(ROOT / name) == sha for name, sha in inputs.items())
    binary = Path(build["artifact"]["sealed"])
    assert digest(binary) == build["artifact"]["sha256"]
    assert CASE + ": test" in subprocess.check_output([str(binary), "--list"], text=True).splitlines()
    image = json.loads(subprocess.check_output(["docker", "image", "inspect", IMAGE]))[0]
    assert image["Id"] == IMAGE and image["Architecture"] == "arm64" and image["Os"] == "linux"
    module = ROOT / ".local/gate-6/browser-tools/node_modules/playwright/index.mjs"
    assert module.is_file() and (ROOT / "sdk/typescript/dist/browser.js").is_file()

    def sources():
        result = HELPERS["sources"]()
        dockerfile = "scripts/video/Dockerfile.remote-browser"
        result[dockerfile] = digest(ROOT / dockerfile)
        for folder in [ROOT / "scripts/video", ROOT / "examples/conversation-client", ROOT / "sdk/typescript/src", ROOT / "sdk/typescript/dist", module.parent, module.parent.parent / "playwright-core"]:
            for path in sorted(folder.rglob("*")):
                if path.is_file() and "__pycache__" not in path.parts:
                    result[str(path.relative_to(ROOT))] = digest(path)
        return result

    before = sources()
    report = {"passed": False, "scope": "Owned Linux arm64 Chromium151 on local Lima VM to macOS Remote WSS/Opus/VP8 host. Actual normal CA refusal/import, signed public browser SDK, independent decoded duplex and original drains. Not approved installed Chrome155/Firefox158 matrix, native Linux x86 performance, WAN, SIP, TURN, ICE restart or full Gate6 qualification.",
              "algorithm": args.algorithm, "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
              "source_inputs_sha256": before, "server_executable_sha256": digest(binary), "image_id": IMAGE,
              "runner_sha256": digest(__file__), "diagnostic_audio_trace": args.trace_audio,
              "started_at": datetime.datetime.now(datetime.timezone.utc).isoformat(), "guests": []}
    env = dict(os.environ, CONNECT_G6_REMOTE_BROWSER_DIR=str(out), CONNECT_G6_REMOTE_BROWSER_ALGORITHM=args.algorithm,
               CONNECT_G6_REMOTE_BROWSER_MEDIA_PORT=str(ports()), TMPDIR=str(out))
    env.pop("CONNECT_GATE6_CANONICAL_AUDIO_TRACE", None)
    if args.trace_audio:
        env["CONNECT_GATE6_CANONICAL_AUDIO_TRACE"] = str(out)
    process = None
    try:
        with (out / "server.log").open("w") as log:
            process = subprocess.Popen([str(binary), CASE, "--exact", "--ignored", "--test-threads=1", "--nocapture"],
                                       cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                started = time.monotonic()
                while not (out / "ready.json").exists():
                    assert process.poll() is None, "Remote browser server ended before readiness"
                    assert time.monotonic() - started < 45, "Server readiness deadline"
                    time.sleep(.02)
                for mode in ["untrusted", "media"]:
                    cidfile = out / (mode + ".container-id")
                    command = ["docker", "run", "--rm", "--cidfile", str(cidfile), "--read-only", "--cpus", "4", "--memory", "4g",
                               "--shm-size", "512m", "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
                               "--tmpfs", "/tmp:rw,size=512m", "--tmpfs", "/root:rw,size=32m",
                               "--mount", f"type=bind,source={ROOT},target={ROOT},readonly",
                               "--mount", f"type=bind,source={out},target=/fixture",
                               "--env", f"CONNECT_PLAYWRIGHT_MODULE={module}", IMAGE,
                               "node", str(ROOT / "scripts/video/linux_remote_browser.mjs"), "/fixture", mode]
                    with (out / (mode + ".log")).open("w") as guest_log:
                        child = subprocess.Popen(command, stdout=guest_log, stderr=subprocess.STDOUT, start_new_session=True)
                        try:
                            code = child.wait(timeout=140)
                        finally:
                            stop(child)
                            if cidfile.exists():
                                cid = cidfile.read_text().strip()
                                assert re.fullmatch("[0-9a-f]{64}", cid)
                                subprocess.run(["docker", "rm", "--force", cid], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                    raw = (out / (mode + ".log")).read_text()
                    rows = [json.loads(line) for line in raw.splitlines() if line.startswith("{")]
                    row = {"mode": mode, "exit_code": code, "log_sha256": digest(out / (mode + ".log")), "observations": rows,
                           "passed": code == 0 and len(rows) == 1 and (rows[0].get("unconfigured_ca_refused") if mode == "untrusted" else rows[0].get("status") == "passed")}
                    ownership = json.loads((out / f"browser-{mode}-ownership.json").read_text())
                    row["ownership"] = ownership
                    row["passed"] = bool(row["passed"] and ownership["passed"] and ownership["original_browser_joined"]
                                         and not ownership["forced_kill"] and ownership["os"] == "linux" and ownership["architecture"] == "arm64")
                    report["guests"].append(row)
                    print(json.dumps({"mode": mode, "passed": row["passed"], "exit_code": code}), flush=True)
                    assert row["passed"], "Linux browser fixture failed; private logs retained"
            finally:
                (out / "done").write_text("complete\n")
                try:
                    report["server_exit_code"] = process.wait(timeout=15)
                finally:
                    stop(process)
        raw = (out / "server.log").read_text()
        report["server_witness"] = [json.loads(line.split("remote-browser-server-witness ", 1)[1]) for line in raw.splitlines() if "remote-browser-server-witness " in line]
        report["server_passed"] = report["server_exit_code"] == 0 and len(report["server_witness"]) == 1 and bool(re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", raw))
    except Exception as error:
        report["failure"] = {"type": type(error).__name__, "message": str(error)}
    finally:
        if process is not None:
            stop(process)
        report["source_inputs_unchanged"] = sources() == before
        report["build_inputs_unchanged"] = all((ROOT / name).is_file() and digest(ROOT / name) == sha for name, sha in inputs.items())
        report["executable_unchanged"] = digest(binary) == report["server_executable_sha256"]
        report["passed"] = bool(len(report["guests"]) == 2 and all(row["passed"] for row in report["guests"])
                                and report.get("server_passed") and report["source_inputs_unchanged"]
                                and report["build_inputs_unchanged"] and report["executable_unchanged"])
        report["finished_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        (out / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps({"passed": report["passed"], "summary": str(out / "summary.json")}), flush=True)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
