#!/usr/bin/env python3
"""Measure three owned native Linux process trees with real Connect public clients.

Synthetic loopback only. No model/provider/telephone/media execution.
"""
import hashlib
import json
import os
from pathlib import Path
import platform
import runpy
import selectors
import signal
import socket
import subprocess
import time
import tomllib
import urllib.request
from lib.package_environment import fixture_environment
from lib.operational_process_resources import ProcessResources
from lib.sealed_executable import seal_executable

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / ".local/gate6-operational"
PRIVATE = BASE / "private/processes"
EVIDENCE = BASE / "evidence"


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


class Lines:
    def __init__(self, stream):
        self.stream = stream
        self.buffer = b""

    def read(self, budget):
        deadline = time.monotonic() + budget
        with selectors.DefaultSelector() as selector:
            selector.register(self.stream, selectors.EVENT_READ)
            while b"\n" not in self.buffer:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not selector.select(remaining):
                    raise TimeoutError("Owned process line deadline")
                data = os.read(self.stream.fileno(), 65536)
                if not data:
                    raise RuntimeError("Owned process ended before its result")
                self.buffer += data
                if len(self.buffer) > 2 * 1024 * 1024:
                    raise ValueError("Owned process output budget exceeded")
        line, self.buffer = self.buffer.split(b"\n", 1)
        return json.loads(line)


def port(kind):
    with socket.socket(socket.AF_INET, kind) as reservation:
        reservation.bind(("127.0.0.1", 0))
        return reservation.getsockname()[1]


def browser_frame(reader, budget, report):
    value = reader.read(budget)
    if value.get("phase") == "failed":
        assert set(value) == {"phase", "stage", "name", "code"}
        assert value["stage"] in ("input", "browser-sdk", "browser-runtime", "browser-launch",
            "page-ready", "health-reads", "browser-drain")
        assert value["name"] in ("Error", "TypeError", "SyntaxError", "TimeoutError")
        assert value["code"] in (None, "ENOENT", "EACCES", "EADDRINUSE")
        report["browser_failure"] = value
        raise RuntimeError("Owned browser failed; inspect closed stage diagnostic")
    return value


def main():
    os.umask(0o077)
    assert platform.system() == "Linux" and platform.machine() == "x86_64"
    release = platform.freedesktop_os_release()
    assert release["ID"] == "ubuntu" and release["VERSION_ID"] == "24.04"
    PRIVATE.mkdir(parents=True, exist_ok=True)
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    sources = runpy.run_path(str(ROOT / "scripts/check-gate6-linux-hosting.py"))["sources"]
    bounded = runpy.run_path(str(ROOT / "scripts/check-linux-package-build.py"))["run_bounded"]
    before = sources()
    source_path = EVIDENCE / "source-inputs-process-profile.json"
    source_path.write_text(json.dumps(before, sort_keys=True) + "\n")
    report = dict(format="connect/gate6-operational-process-profile/1", passed=False,
        source_revision=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        source_inputs_sha256=sha(source_path), source_count=len(before), checks={},
        task_complete=False, whole_gate_qualified=False,
        scope="Native Ubuntu24.04x86_64 production CLI server, separate actual Rust SDK helper and owned Chromium/Node browser HTTP client; synthetic loopback read-only profile. No media/performance capacity or provider/account qualification.")
    env = fixture_environment()
    env["CARGO_PROFILE_RELEASE_DEBUG"] = "0"
    children = []
    files = []
    sampler = None
    try:
        dist = ROOT / "sdk/typescript/dist"
        assert (dist / "browser_http.js").is_file(), "Build the browser SDK before process profiling"
        browser_sdk = {str(p.relative_to(dist)): sha(p) for p in sorted(dist.rglob("*")) if p.is_file()}
        assert browser_sdk and not any(p.is_symlink() for p in dist.rglob("*"))
        report["browser_sdk_inputs"] = browser_sdk
        compile_log = PRIVATE / "compile.log"
        status = bounded(["cargo", "build", "--release", "--locked", "--message-format=json",
            "-p", "connect-server", "--bin", "connect"], compile_log, env, 3600)
        report["compile_exit_code"] = status
        assert status == 0, "Production CLI compilation failed; inspect private log"
        targets = []
        for line in compile_log.read_text().splitlines():
            try:
                value = json.loads(line)
            except ValueError:
                continue
            if value.get("reason") == "compiler-artifact" and value["target"]["name"] == "connect" and value.get("executable"):
                assert not value["profile"]["test"] and value["profile"]["opt_level"] == "3"
                assert "test-support" not in value["features"]
                targets.append(Path(value["executable"]))
        assert len(targets) == 1
        original = targets[0]
        binary = PRIVATE / "connect"
        binary_sha = sha(original)
        seal_executable(original, binary, binary_sha, pool=PRIVATE / "sealed")
        with binary.open("rb") as stream:
            header = stream.read(64)
        assert header[:6] == b"\x7fELF\x02\x01" and int.from_bytes(header[18:20], "little") == 62
        report["binary_sha256"] = binary_sha
        native_port, http_port = port(socket.SOCK_DGRAM), port(socket.SOCK_STREAM)
        state = PRIVATE / "state"
        status = bounded([str(binary), "init", "--directory", str(state),
            "--bind", f"127.0.0.1:{native_port}", "--http-bind", f"127.0.0.1:{http_port}"],
            PRIVATE / "initialize.log", env, 15)
        assert status == 0
        def log(name):
            stream = (PRIVATE / name).open("wb")
            files.append(stream)
            return stream
        application = tomllib.loads((state / "operator.application.toml").read_text())
        server = subprocess.Popen([str(binary), "serve", "--config", str(state / "server.toml")],
            cwd=ROOT, env=env, stdout=log("server.log"), stderr=subprocess.STDOUT, start_new_session=True)
        children.append(server)
        deadline = time.monotonic() + 15
        while True:
            assert server.poll() is None, "Owned server exited before readiness"
            try:
                request = urllib.request.Request(f"http://127.0.0.1:{http_port}/v1/operations",
                    data=b'{"op":"health"}', headers={"Content-Type":"application/json",
                    "Authorization":"Bearer " + application["credential"]})
                with urllib.request.urlopen(request, timeout=.5) as response:
                    health = json.load(response)["result"]["Ok"]
                    if health["kind"] == "health" and health["data"].get("readiness", {}).get("state") == "ready":
                        break
            except (OSError, ValueError):
                pass
            assert time.monotonic() < deadline, "Owned server readiness deadline"
            time.sleep(.05)
        sdk = state / "operator.sdk.toml"
        sdk.write_text('tenant_id = "tnt_default"\n[control]\n' +
            (state / "operator.toml").read_text() + '\n[application]\n' +
            (state / "operator.application.toml").read_text())
        helper = subprocess.Popen([str(binary), "sdk-stdio", "--config", str(sdk)], cwd=ROOT,
            env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log("helper.log"), start_new_session=True)
        children.append(helper)
        helper_lines = Lines(helper.stdout)
        reads = 0
        def helper_read():
            nonlocal reads
            exchange = f"operational-capabilities-{reads}"
            helper.stdin.write(json.dumps(dict(id=exchange, profile="connect-sdk-helper/4",
                request=dict(timeout_ms=4000, action=dict(type="capabilities_refresh")))).encode() + b"\n")
            helper.stdin.flush()
            value = helper_lines.read(6)
            assert value["id"] == exchange and value["profile"] == "connect-sdk-helper/4"
            assert value["phase"] == "request" and set(value["result"]) == {"Ok"}, "Actual SDK helper read failed"
            reads += 1
        helper_read()
        browser_input = PRIVATE / "browser-input.json"
        browser_input.write_text(json.dumps(dict(dist=str(ROOT / "sdk/typescript/dist"),
            endpoint=f"http://127.0.0.1:{http_port}", credential=application["credential"],
            profile=str(PRIVATE / "browser-profile"))))
        browser_env = dict(env, CONNECT_PLAYWRIGHT_MODULE=str(ROOT / "scripts/video/observer-tools/node_modules/playwright/index.mjs"))
        # This disposable CI profile uses the same explicit Chromium sandbox
        # mode as the successful Playwright installed-app fixture. The owned
        # launcher still bounds and joins the original browser process.
        browser_env["CONNECT_GATE6_OWNED_BROWSER_CONTAINER"] = "1"
        report["browser_sandbox"] = "disabled-in-isolated-synthetic-ci-profile"
        chrome = os.environ.get("CONNECT_GATE6_CHROME")
        if chrome:
            browser_env["CONNECT_GATE6_CHROME"] = chrome
        else:
            result = subprocess.check_output(["node", "--input-type=module", "-e",
                "import {chromium} from './scripts/video/observer-tools/node_modules/playwright/index.mjs'; process.stdout.write(chromium.executablePath())"],
                cwd=ROOT, env=env, timeout=5, text=True)
            browser_env["CONNECT_GATE6_CHROME"] = result
        browser = subprocess.Popen(["node", "scripts/operational-browser.mjs", str(browser_input)],
            cwd=ROOT, env=browser_env, stdout=subprocess.PIPE, stderr=log("browser.log"), start_new_session=True)
        children.append(browser)
        browser_lines = Lines(browser.stdout)
        assert browser_frame(browser_lines, 15, report) == {"phase": "ready"}
        assert sha(Path(f"/proc/{server.pid}/exe")) == binary_sha
        assert sha(Path(f"/proc/{helper.pid}/exe")) == binary_sha
        report["running_server_helper_binary_verified"] = True
        sampler = ProcessResources(dict(server=server.pid, helper=helper.pid, browser=browser.pid))
        deadline = time.monotonic() + 35
        next_helper = 0
        while browser.poll() is None:
            assert time.monotonic() < deadline, "Owned browser profile deadline"
            sampler.sample()
            if time.monotonic() >= next_helper:
                helper_read()
                next_helper = time.monotonic() + 1
            time.sleep(.25)
        observed = browser_frame(browser_lines, 3, report)
        assert browser.wait(timeout=3) == 0
        assert observed == dict(phase="complete", health_reads=45, administrator_health=True,
            helper_peer_observed=True, credential_absent=True, original_browser_joined=True)
        report["browser"] = observed
        report["helper_reads"] = reads
        assert reads >= 5
        helper.stdin.close()
        assert helper.wait(timeout=8) == 0, "Original SDK helper did not join"
        server.send_signal(signal.SIGTERM)
        assert server.wait(timeout=10) == 0, "Original production server did not drain"
        deadline = time.monotonic() + 5
        while True:
            final = sampler.sample()
            if all(v["live_processes"] == 0 and v["root_state"] == "retired" for v in final["groups"].values()):
                break
            assert time.monotonic() < deadline, "Owned descendants survived process drains"
            time.sleep(.1)
        resources = sampler.summary()
        assert resources["sample_count"] >= 10
        assert all(v["peak_sampled_rss_bytes"] > 0 and v["peak_live_processes"] > 0 and v["unavailable_samples"] == 0
            for v in resources["groups"].values())
        resources_path = EVIDENCE / "process-resources.json"
        resources_path.write_text(json.dumps(resources, indent=2) + "\n")
        report["resources_sha256"] = sha(resources_path)
        report["resources"] = resources["groups"]
        assert sha(binary) == binary_sha and sources() == before
        assert {str(p.relative_to(dist)): sha(p) for p in sorted(dist.rglob("*")) if p.is_file()} == browser_sdk
        report["browser_sdk_inputs_unchanged"] = True
        report["checks"] = dict(production_release=True, actual_separate_sdk_helper=True,
            actual_owned_browser=True, three_roles_measured=True, original_processes_joined=True,
            observed_descendants_retired=True, numeric_only_resource_report=True, read_only_loopback=True)
        report["passed"] = True
    finally:
        for child in reversed(children):
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
        if sampler is not None:
            # A failed browser driver cannot leave an observed renderer outside
            # its original group. Revalidate birth before retiring owned children.
            for (pid, started), _role in sampler.known.items():
                try:
                    value = sampler._stat(pid)
                    if value["start_ticks"] == started and value["state"] not in ("Z", "X"):
                        os.kill(pid, signal.SIGKILL)
                except (ProcessLookupError, FileNotFoundError):
                    pass
        for stream in files:
            stream.close()
        report["source_inputs_unchanged"] = sources() == before
        report["passed"] = report["passed"] and report["source_inputs_unchanged"]
        (EVIDENCE / "process-profile.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps(dict(passed=report["passed"], checks=report["checks"])), flush=True)
    assert report["passed"]


if __name__ == "__main__":
    main()
