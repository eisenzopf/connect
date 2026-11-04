#!/usr/bin/env python3
"""One new periodic control-custody regression on the owned ARM Linux VM."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import runpy
import subprocess
import time
import traceback

from lib.package_environment import fixture_environment

ROOT = Path(__file__).resolve().parents[1]
CASE = "canonical_runtime::control_work::tests::periodic_custodian_joins_completed_controls_without_new_submission_or_cancelling_pending"


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    assert platform.system() == "Linux" and platform.machine() == "aarch64"
    assert os.environ.get("CONNECT_GATE6_OWNED_LINUX_VM") == "connect-gate6"
    assert not os.environ.get("CI") and not os.environ.get("GITHUB_ACTIONS")
    assert not subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).strip()
    output = args.output.resolve()
    assert output.is_relative_to(ROOT / ".local")
    os.umask(0o077);output.mkdir(parents=True, exist_ok=False)
    env = fixture_environment();env["CARGO_PROFILE_RELEASE_DEBUG"] = "0"
    bounded = runpy.run_path(str(ROOT / "scripts/check-linux-package-build.py"))["run_bounded"]
    sources = runpy.run_path(str(ROOT / "scripts/check-gate6-linux-hosting.py"))["sources"]
    state = dict(format="connect/idle-control-custody/1", source_revision=subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(), controller_pid=os.getpid(),
        stage="preparing", source_guard_active=False, passed=False, case=CASE,
        scope="One production Service periodic-custody regression; no full media or idle-resource pass.")
    def save(): (output / "state.json").write_text(json.dumps(state, indent=2) + "\n")
    save();before=None;binary=None;started=time.monotonic()
    try:
        with (output / "prepare.log").open("x") as log:
            subprocess.run(["bash", "scripts/prepare-dependencies.sh"], cwd=ROOT, env=env,
                stdout=log, stderr=subprocess.STDOUT, timeout=180, check=True)
        before=sources();(output / "source-inputs.json").write_text(json.dumps(before, sort_keys=True) + "\n")
        state.update(stage="compiling", source_guard_active=True);save()
        log=output / "compile.log"
        code=bounded(["cargo", "test", "--release", "--locked", "-p", "connect-service", "--lib",
            "--no-run", "--message-format=json"], log, env, 3600)
        state["compile_exit_code"]=code;state["compile_log_sha256"]=digest(log);save()
        assert code == 0
        binaries=[]
        for line in log.read_text().splitlines():
            try: value=json.loads(line)
            except ValueError: continue
            if (value.get("reason")=="compiler-artifact" and value.get("executable")
                    and value["target"]["name"]=="connect_service" and value["profile"]["test"]):
                binaries.append(Path(value["executable"]))
        assert len(binaries)==1
        binary=binaries[0];state["binary_sha256"]=digest(binary)
        state.update(stage="new-regression");save()
        log=output / "case.log"
        code=bounded([str(binary), CASE, "--exact", "--nocapture", "--test-threads=1"], log, env, 30)
        state["test_exit_code"]=code;state["test_log_sha256"]=digest(log)
        state["passed"]=code==0 and "1 passed; 0 failed; 0 ignored" in log.read_text()
    except BaseException as error:
        (output / "failure.log").write_text(traceback.format_exc());state["failure"]=type(error).__name__
    finally:
        state["source_inputs_unchanged"]=before is not None and sources()==before
        state["executable_unchanged"]=binary is not None and digest(binary)==state["binary_sha256"]
        state["passed"] &= state["source_inputs_unchanged"] and state["executable_unchanged"]
        state.update(stage="complete", source_guard_active=False, elapsed_seconds=time.monotonic()-started);save()
    print(json.dumps(state), flush=True)
    return 0 if state["passed"] else 1


if __name__ == "__main__": raise SystemExit(main())
