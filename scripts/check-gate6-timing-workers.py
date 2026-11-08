#!/usr/bin/env python3
"""Bounded original rvoip worker regressions; no browser/provider qualification."""
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
import time

ROOT = Path(__file__).resolve().parents[1]
PREFIXES = (
    "conference::",
    "staged_streams::tests::guarded_mixer::",
    "staged_streams::tests::media_scene::",
    "staged_streams::media_scene::tick_summary::",
)
REQUIRED = {
    "staged_streams::tests::guarded_mixer::original_guarded_worker_delivers_audio_while_unguarded_registry_is_locked",
    "staged_streams::tests::media_scene::original_guarded_worker_delivers_video_while_unguarded_registry_is_locked",
    "conference::timing::tests::late_wakeup_and_slow_work_are_distinct_with_equal_completion_lateness",
    "conference::timing::tests::skipped_schedule_uses_the_actual_deadline_without_resetting_the_clock",
    "conference::timing::tests::cancellation_does_not_fabricate_a_completed_tick",
    "conference::timing::tests::original_worker_reports_an_interrupted_registry_wait_through_the_enabled_filter",
}


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def inputs():
    result = {}
    # Bind the full prepared Rust/manifest graph, including originally unchanged
    # sources. Exclude Git internals, build products and raw private material.
    for source in sorted((ROOT / ".deps").iterdir()):
        if not source.is_dir() or source.name.startswith("."):
            continue
        for path in source.rglob("*"):
            if any(part in {".git", "target", "node_modules"} for part in path.parts):
                continue
            if path.is_file() and (path.suffix == ".rs" or path.name in {"Cargo.toml", "Cargo.lock"}):
                result[str(path.relative_to(ROOT))] = digest(path)
    for name in ("dependencies/source-lock.json", "rust-toolchain.toml",
                 "scripts/check-gate6-timing-workers.py"):
        result[name] = digest(ROOT / name)
    lock = json.loads((ROOT / "dependencies/source-lock.json").read_text())
    for patch in lock["patches"]:
        result[patch["path"]] = digest(ROOT / patch["path"])
        assert result[patch["path"]] == patch["sha256"], patch["path"]
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True,
                        help="New ignored .local directory for private logs and result")
    args = parser.parse_args()
    output = args.output.resolve()
    if not output.is_relative_to((ROOT / ".local").resolve()):
        parser.error("output must be inside this worktree's ignored .local directory")
    output.mkdir(parents=True, exist_ok=False)
    environment = dict(os.environ, CARGO_TARGET_DIR=str(ROOT / ".local/timing-investigation/target"),
                       CARGO_PROFILE_DEV_DEBUG="0", CARGO_PROFILE_TEST_DEBUG="0", CARGO_INCREMENTAL="0")
    report = dict(format="connect/timing-worker-check/1", passed=False,
                  date=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  source_revision=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                  platform=platform.platform(), machine=platform.machine(), steps=[],
                  scope="Original rvoip worker, scene, mixer and scheduling component regressions only.",
                  full_task_qualified=False, acceptance_qualified=False)

    def execute(name, command, timeout):
        log = output / (name + ".log")
        started = time.monotonic()
        forced = False
        with log.open("w") as stream:
            process = subprocess.Popen(command, cwd=ROOT, env=environment, stdout=stream,
                                       stderr=subprocess.STDOUT, start_new_session=True)
            try:
                code = process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                # Retire the owned command group, including compiler children.
                # A killed command is a failed check, never a successful drain.
                forced = True
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
                code = -1
        step = dict(name=name, command=command, exit_code=code,
                    forced_cleanup=forced,
                    elapsed_seconds=time.monotonic() - started,
                    log=str(log.relative_to(ROOT)), log_sha256=digest(log))
        report["steps"].append(step)
        print(json.dumps({"step": name, "exit_code": code}), flush=True)
        assert code == 0, name
        return log.read_text(errors="replace"), step

    try:
        execute("prepare", ["bash", "scripts/prepare-dependencies.sh"], 300)
        before = inputs()
        report["source_inputs_sha256"] = before
        text, _ = execute("build", ["cargo", "test", "--locked", "--offline", "--jobs", "2",
            "--manifest-path", ".deps/rvoip/Cargo.toml", "-p", "rvoip-core", "--features", "opus",
            "--lib", "--no-run", "--message-format=json"], 1800)
        artifacts = [json.loads(line) for line in text.splitlines() if line.startswith("{")]
        binaries = [row["executable"] for row in artifacts if row.get("reason") == "compiler-artifact"
                    and row["target"]["name"] == "rvoip_core" and row["profile"]["test"] and row.get("executable")]
        assert len(binaries) == 1, "one original Core test artifact required"
        binary = binaries[0]
        report["binary_sha256"] = digest(binary)
        text, _ = execute("inventory", [binary, "--list"], 30)
        inventory = {line.removesuffix(": test") for line in text.splitlines() if line.endswith(": test")}
        assert REQUIRED <= inventory, sorted(REQUIRED - inventory)
        observed = set()
        for index, prefix in enumerate(PREFIXES):
            expected = {name for name in inventory if name.startswith(prefix)}
            assert expected, prefix
            text, step = execute("group-" + str(index), [binary, prefix, "--test-threads=1"], 180)
            passed = set(re.findall(r"^test (\S+) \.\.\. ok$", text, re.M))
            assert passed == expected, sorted(expected - passed)
            step["passed_tests"] = sorted(passed)
            observed.update(passed)
        assert REQUIRED <= observed
        report["passed_tests"] = sorted(observed)
        report["sources_unchanged"] = before == inputs()
        report["binary_unchanged"] = report["binary_sha256"] == digest(binary)
        assert report["sources_unchanged"] and report["binary_unchanged"]
        report["passed"] = True
    except Exception as error:
        report["error"] = str(error)
    finally:
        (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    if not report["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
