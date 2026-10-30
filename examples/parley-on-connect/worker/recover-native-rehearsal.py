#!/usr/bin/env python3
"""Resume one retained native fixture. Model observation is strictly read-only."""
import argparse
import importlib.util
import json
import os
import pathlib
import socket
import subprocess
import time
import tomllib

spec = importlib.util.spec_from_file_location("native_fixture", pathlib.Path(__file__).with_name("native-rehearsal.py"))
native = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--original", required=True, type=pathlib.Path)
    parser.add_argument("--executable", required=True, type=pathlib.Path)
    parser.add_argument("--expected-sha256", required=True)
    args = parser.parse_args()
    original, binary = args.original.resolve(), args.executable.resolve()
    assert ".local" in original.parts and native.sha(binary) == args.expected_sha256
    initial_summary = json.loads((original / "summary.json").read_text())
    assert initial_summary["actual_model_requested"] and initial_summary["server_exit"] == 0 and initial_summary["helpers_joined"]
    assert not initial_summary["forced_server_cleanup"] and not initial_summary["forced_worker_group_cleanup"]
    inputs = json.loads((original / "input.json").read_text())
    inputs["resumeOriginal"] = True
    inputs["modelConfig"] = str(pathlib.Path(inputs["modelConfig"]).resolve())
    model_config = json.loads(pathlib.Path(inputs["modelConfig"]).read_text())
    assert model_config["approved_model_execution"] is True
    consumed = pathlib.Path(inputs["modelConfig"]).with_suffix(".invocation.json")
    assert consumed.is_file()
    consumed_before = native.sha(consumed)
    requests = list(pathlib.Path(inputs["modelDirectory"]).glob("*/request.json"))
    assert len(requests) == 1
    request_before = native.sha(requests[0])
    output = original / "recovery"
    output.mkdir(mode=0o700)  # Refuse an implicit second recovery scene.
    native.private(output / "input.json", json.dumps(inputs, indent=2) + "\n")
    names = subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=native.ROOT).decode().split("\0")
    names = sorted({name for name in names if name and (native.ROOT / name).is_file() and
                    (name.startswith(("crates/", "sdk/", "contracts/", "scripts/", "dependencies/", "patches/", "examples/")) or
                     name in ["Cargo.toml", "Cargo.lock", "AGENTS.md"])})
    snapshot = lambda: {name: native.sha(native.ROOT / name) for name in names}
    before = snapshot()
    native.private(output / "source-inputs.json", json.dumps(before, indent=2) + "\n")
    server_config = original / "installation/server.toml"
    bind = tomllib.loads(server_config.read_text())["bind"]
    host, port = bind.rsplit(":", 1)
    pid_file = original / "helper-pids"
    old_pids = len(pid_file.read_text().splitlines())
    started = time.monotonic()
    with (output / "server.log").open("xb") as server_log:
        os.chmod(server_log.name, 0o600)
        server = subprocess.Popen([str(binary), "serve", "--config", str(server_config)],
                                  stdout=server_log, stderr=subprocess.STDOUT)
        code, forced, server_forced, error = None, False, False, None
        try:
            deadline = time.monotonic() + 30
            while b"ready transport=quic" not in (output / "server.log").read_bytes():
                assert server.poll() is None and time.monotonic() < deadline
                time.sleep(0.025)
            with (output / "runtime.log").open("xb") as log:
                os.chmod(log.name, 0o600)
                code, forced = native.owned_worker(["node", str(pathlib.Path(__file__).with_name("native-rehearsal.mjs")), str(output / "input.json")], log)
        except Exception as failure:
            error = type(failure).__name__ + ": " + str(failure)
        finally:
            if server.poll() is None:
                server.terminate()
            try:
                server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                server_forced = True
                server.kill()
                server.wait(timeout=5)
    joined = True
    for pid in pid_file.read_text().splitlines()[old_pids:]:
        try:
            os.kill(int(pid), 0)
            joined = False
        except ProcessLookupError:
            pass
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as listener:
        listener.bind((host, int(port)))
    assert len(list(pathlib.Path(inputs["modelDirectory"]).glob("*/request.json"))) == 1
    after = snapshot()
    report = {"profile": "parley-worker-native-recovery-summary/1", "runtime_exit": code,
              "server_exit": server.returncode, "forced_worker_cleanup": forced,
              "forced_server_cleanup": server_forced, "helpers_joined": joined,
              "listener_reclaimed": True, "seconds": time.monotonic() - started,
              "source_count": len(before), "changed_inputs": [name for name in before if before[name] != after[name]],
              "binary_unchanged": native.sha(binary) == args.expected_sha256,
              "original_request_unchanged": native.sha(requests[0]) == request_before,
              "authorization_consumption_unchanged": native.sha(consumed) == consumed_before,
              "new_model_invocations": 0, "same_original_fixture": True, "error": error,
              "binary_producer_commit": "852e4528572a535283fd1eb56c06f276458a86c1",
              "scope": "Current worker recovery code with the preserved source-bound native fixture; no new model turn or media qualification."}
    native.private(output / "summary.json", json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))
    assert code == 0 and server.returncode == 0 and joined and not forced and not server_forced and error is None
    assert not report["changed_inputs"] and report["binary_unchanged"] and report["original_request_unchanged"] and report["authorization_consumption_unchanged"]


if __name__ == "__main__":
    main()
