#!/usr/bin/env python3
"""Exercise current operator commands; corrupt only copied negative backup fixtures."""
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import runpy
import shutil
import signal
import socket
import subprocess
import time
import tomllib
import traceback

from lib.package_environment import fixture_environment
from lib.sealed_executable import seal_executable

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / ".local/gate6-operations"
PRIVATE = BASE / "private"
EVIDENCE = BASE / "evidence"
TEMPLATES = ROOT / "docs/operations/requests"


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def inventory(directory):
    result = {}
    for path in sorted(directory.rglob("*")):
        assert not path.is_symlink(), "Fixture contains a symbolic link"
        if path.is_file():
            result[str(path.relative_to(directory))] = dict(sha256=sha(path), mode=path.stat().st_mode & 0o777)
    return result


def port(kind):
    with socket.socket(socket.AF_INET, kind) as reservation:
        reservation.bind(("127.0.0.1", 0))
        return reservation.getsockname()[1]


def main():
    os.umask(0o077)
    assert platform.system() == "Linux" and platform.machine() == "x86_64"
    release = platform.freedesktop_os_release()
    assert release["ID"] == "ubuntu" and release["VERSION_ID"] == "24.04"
    PRIVATE.mkdir(parents=True)  # Never reuse/delete a previous private rehearsal.
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    sources = runpy.run_path(str(ROOT / "scripts/check-gate6-linux-hosting.py"))["sources"]
    bounded = runpy.run_path(str(ROOT / "scripts/check-linux-package-build.py"))["run_bounded"]
    before = sources()
    source_path = EVIDENCE / "source-inputs.json"
    source_path.write_text(json.dumps(before, sort_keys=True) + "\n")
    report = dict(format="connect/operations-guide-rehearsal/1", task="G6-088", passed=False,
        source_revision=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        source_inputs_sha256=sha(source_path), source_count=len(before), checks=[], commands=[], servers=[],
        cli_envelope_profile="connect-control/1", canonical_transport_profile="connect/2",
        template_hashes={p.name: sha(p) for p in sorted(TEMPLATES.glob("*.json"))},
        scope="Disposable native Ubuntu24.04x86_64 production CLI and synthetic signed control only. Commands use public APIs; only copied negative backups are intentionally corrupted. No media, provider, telephone, model, external call, WAN or publication.",
        task_complete=False, whole_gate_qualified=False)
    env = fixture_environment()
    env["CARGO_PROFILE_RELEASE_DEBUG"] = "0"
    server = None
    streams = []
    stage = "compile"
    try:
        compile_log = PRIVATE / "compile.log"
        status = bounded(["cargo", "build", "--release", "--locked", "--message-format=json",
            "-p", "connect-server", "--bin", "connect"], compile_log, env, 3600)
        report["compile_exit_code"] = status
        assert status == 0, "Production CLI compilation failed"
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
        binary = PRIVATE / "connect"
        binary_sha = sha(targets[0])
        seal_executable(targets[0], binary, binary_sha, pool=PRIVATE / "sealed")
        report["binary_sha256"] = binary_sha
        deadline = time.monotonic() + 300

        def run(name, args, input_bytes=None, *, success=True):
            nonlocal stage
            stage = name
            assert time.monotonic() < deadline, "Operator rehearsal total deadline"
            input_path, output_path = PRIVATE / (name + ".input"), PRIVATE / (name + ".output")
            data = b"" if input_bytes is None else input_bytes
            assert len(data) <= 65536
            input_path.write_bytes(data)
            started = time.monotonic()
            with input_path.open("rb") as source, output_path.open("wb") as output, (PRIVATE / (name + ".stderr")).open("wb") as errors:
                child = subprocess.Popen([str(binary), *map(str, args)], cwd=ROOT, env=env,
                    stdin=source, stdout=output, stderr=errors, start_new_session=True)
                joined = False
                try:
                    code = child.wait(timeout=min(30, max(.1, deadline - time.monotonic())))
                    joined = True
                finally:
                    if not joined:
                        try:
                            os.killpg(child.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                        child.wait()
            outcome = dict(stage=name, verb=args[0], exit_code=code,
                elapsed_seconds=round(time.monotonic() - started, 3))
            if code != 0:
                try:
                    error_code = json.loads(output_path.read_bytes())["result"]["Err"]["code"]
                    outcome["error_code"] = error_code if error_code in {
                        "forbidden", "unauthorized", "invalid-request", "transport-uncertain",
                        "conflict", "unsupported-schema", "configuration", "protocol",
                        "reconciliation-required", "authority-busy", "authority-unavailable",
                        "storage-unavailable"} else "unclassified"
                except (ValueError, KeyError, TypeError):
                    outcome["error_code"] = None
            report["commands"].append(outcome)
            assert (code == 0) == success, "Operator command outcome differs from required result"
            return output_path.read_bytes()

        def check(name):
            report["checks"].append(name)

        def request(name, config, template, *, success=True):
            value = json.loads(run(name, ["request", "--config", config, "--envelope"],
                (TEMPLATES / template).read_bytes(), success=success))
            assert value["profile"] == "connect-control/1"
            if not success:
                assert set(value["result"]) == {"Err"}
                return value["result"]["Err"]
            assert set(value["result"]) == {"Ok"} and value["result"]["Ok"]["kind"] == "canonical_reply"
            return value["result"]["Ok"]["data"]["frames"]

        def rooms(name, config):
            frames = request(name, config, "conversation-list.json")
            acknowledgements = [f for f in frames if f["type"] == "ack"]
            assert len(acknowledgements) == 1
            return acknowledgements[0]["payload"]["details"]["page"]["conversations"]

        def original(name, config):
            frames = request(name, config, "operation-get.json")
            updates = [f["payload"] for f in frames if f["type"] == "operation.updated"]
            assert len(updates) == 1 and updates[0]["operation_id"] == "operations-guide-create-1"
            assert updates[0]["state"] == "completed"
            return updates[0]

        def start(name, state):
            nonlocal server, stage
            stage = name
            log_path = PRIVATE / (name + ".log")
            stream = log_path.open("wb")
            streams.append(stream)
            server = subprocess.Popen([str(binary), "serve", "--config", str(state / "server.toml")],
                cwd=ROOT, env=env, stdin=subprocess.DEVNULL, stdout=stream,
                stderr=subprocess.STDOUT, start_new_session=True)
            until = time.monotonic() + 15
            while True:
                assert server.poll() is None, "Owned server exited before readiness"
                if b"ready transport=quic" in log_path.read_bytes() and b"profile=connect/2" in log_path.read_bytes():
                    break
                assert time.monotonic() < until, "Owned server readiness deadline"
                time.sleep(.05)
            assert sha(Path(f"/proc/{server.pid}/exe")) == binary_sha
            report["servers"].append(dict(stage=name, binary_sha256=binary_sha, original_joined=False))

        def stop():
            nonlocal server
            server.send_signal(signal.SIGTERM)
            assert server.wait(timeout=10) == 0, "Original server did not join graceful shutdown"
            try:
                os.killpg(server.pid, 0)
            except ProcessLookupError:
                pass
            else:
                raise AssertionError("Original server process group survived")
            report["servers"][-1]["original_joined"] = True
            server = None

        native, http = port(socket.SOCK_DGRAM), port(socket.SOCK_STREAM)
        state = PRIVATE / "current"
        init_args = ["init", "--directory", state, "--bind", f"127.0.0.1:{native}", "--http-bind", f"127.0.0.1:{http}"]
        run("fresh-init", init_args)
        prior = inventory(state)
        run("occupied-init-refusal", init_args, success=False)
        assert inventory(state) == prior
        check("fresh-current-init-and-occupied-destination-refusal-without-overwrite")

        # Replace only the generated policy tables; no database or credential edits.
        config_path = state / "server.toml"
        text = re.sub(r"(?ms)^\[record_retention(?:\.[a-z_]+)?\]\n.*?(?=^\[|\Z)", "", config_path.read_text())
        text += '\n[record_retention]\nname = "operations-guide-metadata-60s-v1"\nmetadata = { kind = "after", seconds = 60 }\ntranscript = { kind = "until-deleted" }\nrecorded_media = { kind = "until-deleted" }\nprovider_reference = { kind = "until-deleted" }\n'
        config_path.write_text(text)
        start("current-serve", state)
        health = json.loads(run("current-health", ["health", "--config", state / "operator.application.toml"]))
        assert health["result"]["kind"] == "health" and health["result"]["data"]["readiness"]["state"] == "ready"
        check("production-server-ready-with-documented-retention-policy-and-public-health")
        assert rooms("empty-current-list", state / "operator.toml") == []
        request("create-original", state / "operator.toml", "conversation-create.json")
        initial_rooms = rooms("created-list", state / "operator.toml")
        assert len(initial_rooms) == 1
        first = original("original-status", state / "operator.toml")
        request("replay-original", state / "operator.toml", "conversation-create.json")
        assert rooms("replayed-list", state / "operator.toml") == initial_rooms
        assert original("replayed-status", state / "operator.toml") == first
        check("exact-documented-native-templates-recover-one-original-effect")

        identity = json.loads(run("create-identity", ["provision", "--config", state / "operator.application.toml"], (TEMPLATES / "identity.json").read_bytes()))["result"]["Ok"]
        assert identity["kind"] == "identity"
        child_ids = {}
        expiry = int(time.time()) + 86400
        for label in ("a", "b"):
            intent_path = PRIVATE / ("credential-" + label + ".json")
            intent_path.write_text(json.dumps(dict(operation_id="operations-guide-issue-" + label + "-1",
                identity_id=identity["identity_id"], operations=["health", "conversation.list", "send", "history", "operation.get"], recipients=[], expires_at=expiry)))
            output = PRIVATE / ("client-" + label)
            run("issue-" + label, ["issue", "--config", state / "operator.application.toml", "--native-template", state / "operator.toml", "--request", intent_path, "--output", output])
            issued = json.loads((output / "issued.json").read_text())
            assert issued["identity_id"] == identity["identity_id"] and issued["expires_at"] == expiry
            child_ids[label] = issued["credential_id"]
            if label == "a":
                installed = inventory(output)
                run("resume-original-issue", ["issue-resume", "--config", state / "operator.application.toml", "--installation", output])
                assert inventory(output) == installed
        assert child_ids["a"] != child_ids["b"]
        assert rooms("new-client-read", PRIVATE / "client-b/client.toml") == []
        child_health = json.loads(run("replacement-health", ["health", "--config", PRIVATE / "client-b/application.toml"]))
        assert child_health["result"]["data"].get("operational") is None
        forbidden = json.loads(run("replacement-cannot-provision", ["provision", "--config", PRIVATE / "client-b/application.toml"],
            (TEMPLATES / "identity.json").read_bytes(), success=False))
        assert forbidden["result"]["Err"]["code"] == "forbidden"
        check("scoped-independent-replacement-and-original-issuance-resume")
        revoke_args = ["revoke", "--config", state / "operator.application.toml", "--operation-id", "operations-guide-revoke-a-1", child_ids["a"]]
        revoked = json.loads(run("revoke-old-client", revoke_args))
        assert revoked["result"]["Ok"]["revoked"] is True
        assert json.loads(run("replay-original-revocation", revoke_args)) == revoked
        changed = json.loads(run("changed-revocation-target-conflicts", [*revoke_args[:-1], child_ids["b"]], success=False))
        assert changed["result"]["Err"]["code"] == "conflict"
        old = request("old-client-refused", PRIVATE / "client-a/client.toml", "conversation-list.json", success=False)
        assert old["code"] == "unauthorized"
        assert rooms("replacement-survives-revocation", PRIVATE / "client-b/client.toml") == []
        check("original-id-revocation-replay-old-client-refused-new-client-preserved")

        backup = PRIVATE / "before-change"
        backup_args = ["backup", "--server-config", state / "server.toml", "--config", state / "operator.toml", "--output", backup]
        run("live-writer-backup-refusal", backup_args, success=False)
        assert not backup.exists()
        stop()
        run("stopped-backup", backup_args)
        manifest = json.loads((backup / "manifest.json").read_text())
        schema = int(re.search(r"pub const SCHEMA_VERSION: u32 = (\d+);", (ROOT / "crates/connect-store/src/lib.rs").read_text())[1])
        assert manifest["snapshot"]["schema"] == schema
        report["schema"] = schema
        stopped = inventory(state)
        run("occupied-restore-refusal", ["restore", "--backup", backup, "--directory", state], success=False)
        assert inventory(state) == stopped
        bad = PRIVATE / "unsupported-backup"
        shutil.copytree(backup, bad)
        altered = json.loads((bad / "manifest.json").read_text())
        altered["snapshot"]["schema"] = schema - 1
        (bad / "manifest.json").write_text(json.dumps(altered))
        run("unsupported-backup-refusal", ["restore", "--backup", bad, "--directory", PRIVATE / "must-not-publish-unsupported"], success=False)
        assert not (PRIVATE / "must-not-publish-unsupported").exists()
        corrupt = PRIVATE / "corrupt-backup"
        shutil.copytree(backup, corrupt)
        path = corrupt / "state.sqlite3"
        with path.open("r+b") as stream:
            stream.write(b"corrupt-fixture")
        run("corrupt-backup-refusal", ["restore", "--backup", corrupt, "--directory", PRIVATE / "must-not-publish-corrupt"], success=False)
        assert not (PRIVATE / "must-not-publish-corrupt").exists()
        check("live-writer-occupied-corrupt-and-unsupported-backup-refusals")

        # Keep the old installation intact, while making its original path absent.
        preserved = PRIVATE / "preserved-original"
        state.rename(preserved)
        restored = PRIVATE / "restored"
        run("current-restore", ["restore", "--backup", backup, "--directory", restored])
        assert not (restored / "operator.application.toml").exists()
        start("restored-serve", restored)
        assert rooms("restored-original-conversation", restored / "operator.toml") == initial_rooms
        assert original("restored-original-status", restored / "operator.toml") == first
        assert rooms("restored-current-child", PRIVATE / "client-b/client.toml") == []
        assert request("restored-revoked-child-refusal", PRIVATE / "client-a/client.toml", "conversation-list.json", success=False)["code"] == "unauthorized"
        stop()
        check("restore-without-original-path-retains-original-operation-state-and-revocation")
        assert inventory(preserved) == stopped
        restored_before = inventory(restored)
        reprovisioned = PRIVATE / "reprovisioned"
        run("explicit-new-path-reprovision", ["init", "--directory", reprovisioned, "--bind", f"127.0.0.1:{native}", "--http-bind", f"127.0.0.1:{http}"])
        assert inventory(preserved) == stopped and inventory(restored) == restored_before
        start("reprovisioned-serve", reprovisioned)
        assert rooms("reprovisioned-empty-state", reprovisioned / "operator.toml") == []
        request("old-client-cannot-adopt-fresh-installation", PRIVATE / "client-b/client.toml", "conversation-list.json", success=False)
        assert tomllib.loads((reprovisioned / "operator.toml").read_text())["identity_id"] != tomllib.loads((restored / "operator.toml").read_text())["identity_id"]
        stop()
        check("explicit-fresh-path-reset-preserves-all-original-state")
        assert sha(binary) == binary_sha
        assert len(report["servers"]) == 3 and all(value["original_joined"] for value in report["servers"])
        assert {c["verb"] for c in report["commands"]} == {"init", "health", "request", "provision", "issue", "issue-resume", "revoke", "backup", "restore"}
        check("all-original-server-and-command-processes-joined")
        report["passed"] = True
    except BaseException as error:
        (PRIVATE / "failure.log").write_text(traceback.format_exc())
        report["failure"] = dict(stage=stage, name=type(error).__name__, line=traceback.extract_tb(error.__traceback__)[-1].lineno)
    finally:
        if server is not None and server.poll() is None:
            os.killpg(server.pid, signal.SIGKILL)
            server.wait()
        for stream in streams:
            stream.close()
        report["source_inputs_unchanged"] = before == sources()
        report["passed"] = report["passed"] and report["source_inputs_unchanged"]
        (EVIDENCE / "rehearsal.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ("passed", "checks")}, sort_keys=True))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
