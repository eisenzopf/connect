#!/usr/bin/env python3
"""Run the isolated, source-bound fifty-peer native protocol rehearsal."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
CASE = (
    "canonical_native_connections::multiparty::"
    "production_fifty_native_participants_preserve_explicit_fanout_handles_and_lifecycle"
)


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def sources():
    names = subprocess.check_output(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=ROOT,
    ).decode().split("\0")
    directories = {"crates", "examples", "contracts", "dependencies", "patches", "scripts"}
    suffixes = {".rs", ".toml", ".json", ".sql", ".patch", ".py", ".sh", ".lock"}
    return {
        name: digest(ROOT / name)
        for name in sorted(set(names))
        if name
        and (name in {"Cargo.toml", "Cargo.lock"} or Path(name).parts[0] in directories)
        and Path(name).suffix in suffixes
        and (ROOT / name).is_file()
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=("release", "test"), default="release")
    parser.add_argument("--target-dir", type=Path)
    parser.add_argument("--python", default=sys.executable, help="Interpreter with scripts/requirements-gate6.txt installed")
    parser.add_argument("--output", type=Path, help="Fresh directory beneath .local/gate-6")
    args = parser.parse_args()
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    directory = Path(os.path.abspath(args.output or ROOT / ".local/gate-6/fifty-native" / stamp))
    if not directory.is_relative_to(ROOT / ".local/gate-6"):
        parser.error("--output must be beneath this workspace's .local/gate-6")
    directory.mkdir(parents=True, exist_ok=False)
    before = sources()
    report = {
        "task": "G6-046",
        "passed": False,
        "platform": platform.platform(),
        "profile": args.profile,
        "started_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "source_lock_sha256": digest(ROOT / "dependencies/source-lock.json"),
        "prepared_patch_count": len(json.loads((ROOT / "dependencies/source-lock.json").read_text())["patches"]),
        "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "working_tree_status": subprocess.check_output(["git", "status", "--short"], cwd=ROOT, text=True).splitlines(),
        "source_inputs_sha256": before,
        "scope": "Isolated local fifty-Participant native QUIC protocol rehearsal. SDK verifies signatures; Python validates captured schemas, correlations, selectors, handles, decoded counts and lifecycle. Other substrates, remote/hosting/provider profiles, remaining clauses and whole-gate acceptance are separate.",
        "steps": [],
    }
    env = dict(os.environ, CARGO_INCREMENTAL="0" if args.profile == "release" else "1")
    report["compiler_environment"] = {
        key: value
        for key, value in sorted(env.items())
        if key in {"CARGO_INCREMENTAL", "CARGO_BUILD_TARGET", "RUSTC", "RUSTC_WRAPPER", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"}
        or key.startswith("CARGO_PROFILE_")
    }
    report["runtime_environment"] = {key: env[key] for key in ("RUST_LOG",) if key in env}

    def run(name, command, build_json=False):
        log = directory / (name + ".log")
        started = time.monotonic()
        with log.open("w") as output:
            if build_json:
                with (directory / "build.jsonl").open("w") as artifacts:
                    result = subprocess.run(command, cwd=ROOT, env=env, stdout=artifacts, stderr=output)
            else:
                result = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT)
        report["steps"].append({"name": name, "command": command, "exit_code": result.returncode, "elapsed_seconds": time.monotonic() - started})
        print(json.dumps({"step": name, "exit_code": result.returncode}), flush=True)
        return result.returncode

    code = 1
    try:
        code = run("dependencies", ["bash", "scripts/prepare-dependencies.sh"])
        if code:
            return code
        code = run("compiler", ["rustc", "--version", "--verbose"])
        if code:
            return code
        code = run("validation-environment", [args.python, "-c", "import importlib.metadata,json,jsonschema,sys; print(json.dumps({'python':sys.version,'jsonschema':importlib.metadata.version('jsonschema')}))"])
        if code:
            return code
        command = ["cargo", "test", "-p", "connect-server", "--test", "gate6_canonical_runtime", "--no-run", "--locked", "--offline", "-j4", "--message-format=json"]
        if args.profile == "release":
            command.append("--release")
        if args.target_dir:
            command += ["--target-dir", str(args.target_dir.expanduser().absolute())]
        code = run("build", command, build_json=True)
        if code:
            return code
        artifacts = [json.loads(line) for line in (directory / "build.jsonl").read_text().splitlines()]
        executables = [item for item in artifacts if item.get("reason") == "compiler-artifact" and item.get("target", {}).get("name") == "gate6_canonical_runtime" and item.get("executable")]
        assert len(executables) == 1, "Expected exactly the requested native test executable"
        artifact = executables[0]
        assert artifact["target"]["kind"] == ["test"] and artifact["profile"]["test"], "Expected a compiled test target"
        binary = Path(artifact["executable"])
        report["executable"] = str(binary)
        report["executable_sha256"] = digest(binary)
        report["cargo_profile"] = artifact["profile"]
        if args.profile == "release":
            assert artifact["profile"]["opt_level"] == "3", "Expected the default optimized release profile"
        capture = directory / "observations.jsonl"
        capture.touch(exist_ok=False)
        env["CONNECT_FIFTY_PARTICIPANT_OBSERVATIONS"] = str(capture)
        code = run("runtime", [str(binary), CASE, "--exact", "--include-ignored", "--test-threads=1", "--nocapture"])
        if code:
            return code
        assert re.search(r"test result: ok\. 1 passed; 0 failed;", (directory / "runtime.log").read_text()), "The exact runtime method must execute and pass"
        code = run("validation", [args.python, "scripts/validate-gate6-fifty-participants.py", str(capture)])
        if code:
            return code
        report["validation"] = json.loads((directory / "validation.log").read_text())
        assert report["validation"]["passed"] is True
        return 0
    except Exception as error:
        code = 1
        report["failure"] = {"type": type(error).__name__, "message": str(error)}
        raise
    finally:
        report["source_inputs_unchanged"] = before == sources()
        report["passed"] = code == 0 and report.get("validation", {}).get("passed") is True and report["source_inputs_unchanged"]
        report["exit_code"] = code if code else (0 if report["passed"] else 1)
        report["artifacts_sha256"] = {p.name: digest(p) for p in sorted(directory.iterdir()) if p.is_file()}
        report["finished_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        (directory / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps({"passed": report["passed"], "summary": str(directory / "summary.json")}), flush=True)
        if code == 0 and not report["passed"]:
            raise SystemExit(1)


if __name__ == "__main__":
    raise SystemExit(main())
