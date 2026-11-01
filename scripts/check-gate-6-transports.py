#!/usr/bin/env python3
"""G6-058 matrix inventory and explicit non-media control component.

The control lane never populates full matrix cells. Browser/media, topology,
participant/movement and capture producers still need their own execution.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import runpy
import subprocess
import time
import traceback

from lib.package_environment import fixture_environment
from lib.sealed_executable import seal_executable
from lib.transport_matrix import CONTROL_CASES, attach_control, digest, inventory

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / ".local/gate6-transports"


def sdk_inputs():
    dist = ROOT / "sdk/typescript/dist"
    assert dist.is_dir() and not dist.is_symlink(), "Build the current browser SDK before control execution"
    result = {}
    for path in sorted(dist.rglob("*")):
        assert not path.is_symlink(), "SDK runtime input is not ordinary"
        if path.is_file():
            result[str(path.relative_to(dist))] = digest(path)
    assert "browser_client.js" in result and "uctp_signing.js" in result
    return result


def step_result(name, log, code, elapsed):
    text = log.read_text()
    result = dict(name=name, exit_code=code, elapsed_seconds=round(elapsed, 3),
        log_sha256=digest(log), test_results=[line for line in text.splitlines() if line.startswith("test result:")],
        compiler_errors=[], failure_locations=[])
    for line in text.splitlines():
        location = re.search(r"panicked at ([^\s]+\.rs):(\d+):(\d+)", line)
        if location:
            result["failure_locations"].append(dict(file=location[1], line=int(location[2]), column=int(location[3])))
        try:
            value = json.loads(line)
        except ValueError:
            continue
        if not isinstance(value, dict):
            continue
        if value.get("reason") == "compiler-message" and value["message"]["level"] == "error":
            message = value["message"]
            result["compiler_errors"].append(dict(code=(message.get("code") or {}).get("code"), message=message["message"][:500],
                locations=[dict(file=p["file_name"], line=p["line_start"]) for p in message["spans"] if p["is_primary"]]))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--phase", choices=("inventory", "control"), required=True)
    parser.add_argument("--case", choices=CONTROL_CASES)
    parser.add_argument("--output", type=Path, help="A new result directory under ignored .local/")
    args = parser.parse_args()
    if args.case and args.phase != "control":
        parser.error("Exact case selection requires the control component")
    os.umask(0o077)
    output = (args.output or BASE / "evidence" / args.phase).resolve()
    if not output.is_relative_to((ROOT / ".local").resolve()):
        parser.error("Results must remain under ignored .local/")
    output.mkdir(parents=True, exist_ok=False)
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    matrix = inventory(ROOT, revision)
    matrix["execution_phase"] = args.phase
    if args.phase == "inventory":
        matrix["inventory_valid"] = True
        (output / "matrix.json").write_text(json.dumps(matrix, indent=2) + "\n")
        print(json.dumps(dict(inventory_valid=True, required_cells=32, matrix_qualified=False, task_complete=False)))
        return 0

    private = BASE / "private" / args.phase
    private.mkdir(parents=True, exist_ok=False)
    (private / "tmp").mkdir()
    selected = (args.case,) if args.case else CONTROL_CASES
    control = dict(format="connect/transport-control-component/1", source_revision=revision,
        service_contract_sha256=matrix["service_contract_sha256"], selected_cases=list(selected),
        cases=[dict(name=name, status="not-run", reason="component-not-executed") for name in selected],
        steps=[], passed=False, all_cases_executed=False, executable_sha256=None,
        task_complete=False, matrix_qualified=False, whole_gate_qualified=False,
        scope="Actual isolated native QUIC/independent Rust HTTP3/TLS WSS and Node public browser-SDK control only. No actual browser, decoded media, packet-capture, movement, remote-host or complete matrix qualification. No G6-054 media/SIP/TURN, provider or model execution.")
    sources = runpy.run_path(str(ROOT / "scripts/check-gate6-linux-hosting.py"))["sources"]
    bounded = runpy.run_path(str(ROOT / "scripts/check-linux-package-build.py"))["run_bounded"]
    before = None
    runtime_inputs = None
    sealed = None
    executable = None
    phase = "dependencies"
    env = fixture_environment()
    env.update(CARGO_PROFILE_RELEASE_DEBUG="0", TMPDIR=str(private / "tmp"))
    try:
        log = private / "dependencies.log"
        started = time.monotonic()
        code = bounded(["bash", "scripts/prepare-dependencies.sh"], log, env, 600)
        control["steps"].append(step_result("dependencies", log, code, time.monotonic() - started))
        assert code == 0, "Verified source preparation failed"
        before = sources()
        source_path = output / "source-inputs.json"
        source_path.write_text(json.dumps(before, sort_keys=True) + "\n")
        control.update(source_inputs_sha256=digest(source_path), source_count=len(before))
        phase = "host-preflight"
        system, machine = platform.system(), platform.machine()
        assert (system, machine) in (("Linux", "x86_64"), ("Darwin", "arm64"))
        if system == "Linux":
            release = platform.freedesktop_os_release()
            assert release["ID"] == "ubuntu" and release["VERSION_ID"] == "24.04"
            assert re.search(r"^vendor_id\s*:", Path("/proc/cpuinfo").read_text(), re.M)
        rust = subprocess.check_output(["rustc", "-Vv"], cwd=ROOT, env=env, text=True, timeout=5)
        assert rust.startswith("rustc 1.97.1 ")
        node = subprocess.check_output(["node", "--version"], cwd=ROOT, env=env, text=True, timeout=5).strip()
        assert node.startswith("v") and int(node[1:].split(".")[0]) >= 22
        control["host"] = dict(system=system, machine=machine, platform=platform.platform(), rust=rust, node=node)
        runtime_inputs = sdk_inputs()
        control["sdk_dist_inputs"] = runtime_inputs
        # Full test compilation includes Rust type validation. A separate
        # cargo-check dependency graph duplicated most of the cold build cost.
        control["type_validation"] = "full-optimized-test-compilation"
        control["separate_typecheck_run"] = False
        for name, command, budget in [
            ("compile", ["cargo", "test", "--release", "--locked", "--message-format=json", "-p", "connect-server", "--features", "test-support", "--test", "gate6_canonical_webtransport", "--no-run"], 5400),
        ]:
            phase = name
            log = private / (name + ".log")
            started = time.monotonic()
            code = bounded(command, log, env, budget)
            result = step_result(name, log, code, time.monotonic() - started)
            control["steps"].append(result)
            print(json.dumps(dict(phase=name, exit_code=code, compiler_errors=len(result["compiler_errors"]))), flush=True)
            assert code == 0 and not result["compiler_errors"], "Control fixture compilation failed; runtime unrun"
        artifacts = []
        for line in (private / "compile.log").read_text().splitlines():
            try:
                value = json.loads(line)
            except ValueError:
                continue
            if value.get("reason") == "compiler-artifact" and value.get("executable") and value["target"]["name"] == "gate6_canonical_webtransport":
                assert value["profile"]["test"] and value["profile"]["opt_level"] == "3"
                assert "test-support" in value["features"]
                control["fixture_features"] = value["features"]
                artifacts.append(Path(value["executable"]))
        assert len(artifacts) == 1
        executable = artifacts[0]
        control["executable_sha256"] = digest(executable)
        sealed = private / "qualified-control"
        seal_executable(executable, sealed, control["executable_sha256"], pool=private / "sealed")
        listing = subprocess.check_output([str(sealed), "--list"], cwd=ROOT, env=env, text=True, timeout=10)
        available = {line.removesuffix(": test") for line in listing.splitlines() if line.endswith(": test")}
        assert set(selected) <= available, "Named control functions absent; no broad substitute selection"
        for index, name in enumerate(selected):
            phase = "case-" + str(index)
            log = private / (phase + ".log")
            started = time.monotonic()
            code = bounded([str(sealed), name, "--exact", "--test-threads=1", "--nocapture"], log, env, 180)
            result = step_result(name, log, code, time.monotonic() - started)
            passed = code == 0 and len(result["test_results"]) == 1 and re.fullmatch(
                r"test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; \d+ filtered out; finished in .+", result["test_results"][0]) is not None
            control["cases"][index] = dict(result, status="passed" if passed else "failed")
            print(json.dumps(dict(phase="control-case", index=index, passed=passed)), flush=True)
        control["all_cases_executed"] = all(value["status"] != "not-run" for value in control["cases"])
        control["passed"] = control["all_cases_executed"] and all(value["status"] == "passed" for value in control["cases"])
    except BaseException as error:
        (private / "failure.log").write_text(traceback.format_exc())
        control["failure"] = dict(phase=phase, name=type(error).__name__, line=traceback.extract_tb(error.__traceback__)[-1].lineno)
    finally:
        # Failed integrity observations must still publish the negative report.
        try:
            control["source_inputs_unchanged"] = before is not None and sources() == before
        except (OSError, ValueError):
            control["source_inputs_unchanged"] = False
        try:
            control["sdk_dist_inputs_unchanged"] = runtime_inputs is not None and sdk_inputs() == runtime_inputs
        except (OSError, AssertionError):
            control["sdk_dist_inputs_unchanged"] = False
        try:
            control["executables_unchanged"] = sealed is not None and digest(sealed) == control["executable_sha256"] and digest(executable) == control["executable_sha256"]
        except OSError:
            control["executables_unchanged"] = False
        control["passed"] = bool(control["passed"] and control["source_inputs_unchanged"] and control["sdk_dist_inputs_unchanged"] and control["executables_unchanged"])
        control["cacheable_build"] = bool(control["executables_unchanged"] and control["source_inputs_unchanged"])
        report_path = output / "control.json"
        report_path.write_text(json.dumps(control, indent=2) + "\n")
        if before is not None:
            reference = {key: control[key] for key in ("format", "source_revision", "service_contract_sha256",
                "source_inputs_sha256", "executable_sha256", "selected_cases", "passed", "all_cases_executed")}
            reference["report_sha256"] = digest(report_path)
            attach_control(matrix, reference)
        (output / "matrix.json").write_text(json.dumps(matrix, indent=2) + "\n")
        if os.environ.get("GITHUB_OUTPUT"):
            with open(os.environ["GITHUB_OUTPUT"], "a") as stream:
                stream.write("cacheable_build=" + str(control["cacheable_build"]).lower() + "\n")
    print(json.dumps(dict(control_component_passed=control["passed"], matrix_qualified=False, task_complete=False)))
    return 0 if control["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
