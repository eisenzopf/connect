#!/usr/bin/env python3
"""Real stopped-backup materialization and native configuration probe; no media."""
import argparse
import json
import os
from pathlib import Path
import platform
import re
import runpy
import signal
import socket
import subprocess
import time
import tomllib
import traceback

from lib.package_environment import fixture_environment
from lib.sealed_executable import seal_executable
from lib.transport_actors import enroll_actors
from lib.transport_fixture import FixtureError, materialize, publish, toml_bytes
from lib.transport_matrix import digest

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / ".local/gate6-transports"
PRIVATE = BASE / "private/materializer"
OUTPUT = BASE / "evidence/materializer"


def files(directory):
    return {str(p.relative_to(directory)): dict(sha256=digest(p), mode=p.stat().st_mode & 0o777)
        for p in sorted(directory.rglob("*")) if p.is_file() and not p.is_symlink()}


def port(kind):
    with socket.socket(socket.AF_INET, kind) as reservation:
        reservation.bind(("127.0.0.1", 0))
        return reservation.getsockname()[1]


def main(actors=False, conversation=False):
    global PRIVATE, OUTPUT
    if conversation:
        actors = True
        PRIVATE, OUTPUT = BASE / "private/conversation", BASE / "evidence/conversation"
    elif actors:
        PRIVATE, OUTPUT = BASE / "private/actors", BASE / "evidence/actors"
    os.umask(0o077)
    assert platform.system() == "Linux" and platform.machine() == "x86_64"
    release = platform.freedesktop_os_release()
    assert release["ID"] == "ubuntu" and release["VERSION_ID"] == "24.04"
    assert re.search(r"^vendor_id\s*:", Path("/proc/cpuinfo").read_text(), re.M)
    PRIVATE.mkdir(parents=True)
    OUTPUT.mkdir(parents=True)
    source = runpy.run_path(str(ROOT / "scripts/check-gate6-linux-hosting.py"))["sources"]
    bounded = runpy.run_path(str(ROOT / "scripts/check-linux-package-build.py"))["run_bounded"]
    env = fixture_environment()
    env["CARGO_PROFILE_RELEASE_DEBUG"] = "0"
    report = dict(format="connect/transport-fixture-rehearsal/1", source_revision=subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(), passed=False, checks=[], commands=[],
        servers=[], task_complete=False, matrix_qualified=False, cell_qualified=False, whole_gate_qualified=False,
        scope="Actual Ubuntu24.04x86_64 production CLI stopped-backup/restore and private configuration. Optional actor and public Node SDK conversation stages verify scoped devices, membership/audiences and two accepted Session intents. No six-Connection workload, actual browser, packet capture, live media, SIP, TURN, model/provider or remote-host execution. All full matrix cells remain unqualified.")
    before = None
    server = None
    streams = []
    phase = "dependencies"
    binary = None
    sdk_before = None
    try:
        code = bounded(["bash", "scripts/prepare-dependencies.sh"], PRIVATE / "dependencies.log", env, 600)
        assert code == 0
        before = source()
        source_path = OUTPUT / "source-inputs.json"
        source_path.write_text(json.dumps(before, sort_keys=True) + "\n")
        report.update(source_inputs_sha256=digest(source_path), source_count=len(before))
        phase = "compile"
        log = PRIVATE / "compile.log"
        code = bounded(["cargo", "build", "--release", "--locked", "--message-format=json",
            "-p", "connect-server", "--bin", "connect"], log, env, 3600)
        report["compile_exit_code"] = code
        assert code == 0
        targets = []
        for line in log.read_text().splitlines():
            try:
                value = json.loads(line)
            except ValueError:
                continue
            if value.get("reason") == "compiler-artifact" and value["target"]["name"] == "connect" and value.get("executable"):
                assert not value["profile"]["test"] and value["profile"]["opt_level"] == "3"
                assert "test-support" not in value["features"]
                targets.append(Path(value["executable"]))
        assert len(targets) == 1
        report["binary_sha256"] = digest(targets[0])
        binary = PRIVATE / "connect"
        seal_executable(targets[0], binary, report["binary_sha256"], pool=PRIVATE / "sealed")

        def run(name, arguments, data=None, *, success=True):
            nonlocal phase
            phase = name
            path = PRIVATE / (name + ".output")
            source_path = PRIVATE / (name + ".input")
            source_path.write_bytes(b"" if data is None else json.dumps(data).encode())
            with source_path.open("rb") as incoming, path.open("wb") as out, (PRIVATE / (name + ".stderr")).open("wb") as errors:
                child = subprocess.Popen([str(binary), *map(str, arguments)], cwd=ROOT, env=env,
                    stdin=incoming, stdout=out, stderr=errors, start_new_session=True)
                joined = False
                try:
                    result = child.wait(timeout=30)
                    joined = True
                finally:
                    if not joined:
                        try:
                            os.killpg(child.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                        child.wait()
            report["commands"].append(dict(name=name, verb=arguments[0], exit_code=result))
            assert (result == 0) == success, "Public fixture command result differs"
            return path.read_bytes()

        def request(name, config, kind, payload, operation=None):
            scope = {} if operation is None else dict(operation_id=operation)
            result = json.loads(run(name, ["request", "--config", config, "--envelope"],
                dict(command=dict(type=kind, payload=payload), scope=scope)))
            assert result["profile"] == "connect-control/1" and set(result["result"]) == {"Ok"}
            return result["result"]["Ok"]["data"]["frames"]

        def rooms(name, config):
            frames = request(name, config, "conversation.list", dict(filter={}, limit=20))
            return next(v for v in frames if v["type"] == "ack")["payload"]["details"]["page"]["conversations"]

        def start(name, config):
            nonlocal server, phase
            phase = name
            path = PRIVATE / (name + ".log")
            stream = path.open("wb")
            streams.append(stream)
            server = subprocess.Popen([str(binary), "serve", "--config", str(config)], cwd=ROOT, env=env,
                stdin=subprocess.DEVNULL, stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
            until = time.monotonic() + 15
            while b"ready transport=quic" not in path.read_bytes():
                assert server.poll() is None and time.monotonic() < until
                time.sleep(.05)
            assert digest(Path(f"/proc/{server.pid}/exe")) == report["binary_sha256"]
            report["servers"].append(dict(name=name, running_binary_sha256=report["binary_sha256"], original_joined=False))

        def stop():
            nonlocal server
            server.send_signal(signal.SIGTERM)
            assert server.wait(timeout=10) == 0
            try:
                os.killpg(server.pid, 0)
            except ProcessLookupError:
                pass
            else:
                raise AssertionError("Original server group survived")
            report["servers"][-1]["original_joined"] = True
            server = None

        state, seed, output = PRIVATE / "initial", PRIVATE / "seed", PRIVATE / "materialized"
        native, http = port(socket.SOCK_DGRAM), port(socket.SOCK_STREAM)
        run("initialize", ["init", "--directory", state, "--bind", f"127.0.0.1:{native}", "--http-bind", f"127.0.0.1:{http}"])
        start("seed-server", state / "server.toml")
        request("seed-create", state / "operator.toml", "conversation.create",
            dict(tenant_id="tnt_default", policy="persistent", initial_participants=[], metadata=dict(fixture="transport-materialization")),
            "transport-fixture-original")
        expected = rooms("seed-list", state / "operator.toml")
        assert len(expected) == 1
        stop()
        run("stopped-backup", ["backup", "--server-config", state / "server.toml", "--config", state / "operator.toml", "--output", seed])
        original_seed = files(seed)
        original_server = tomllib.loads((seed / "server.toml").read_text())
        original_operator = tomllib.loads((seed / "operator.toml").read_text())
        parameters = dict(cell_id="G6-058/ubuntu24-x86_64/local/native-quic/EdDSA",
            candidate_source_revision=report["source_revision"], candidate_binary_sha256=report["binary_sha256"],
            seed_manifest_sha256=digest(seed / "manifest.json"), browser_origin=None,
            topology=dict(kind="local-loopback", server_platform="ubuntu24-x86_64", client_platform="ubuntu24-x86_64",
                client_emulated=False, native_endpoint=original_operator["endpoint"], server_name=original_operator["server_name"]))
        phase = "materialize"
        summary = materialize(ROOT, seed, output, parameters, binary, env)
        assert summary["server_configuration_materialized"] and summary["production_restore_joined"]
        assert not summary["cell_qualified"] and not summary["participant_setup_complete"]
        for name in ("server.toml", "operator.toml", "materialized.json"):
            assert (output / name).stat().st_mode & 0o777 == 0o600
        configured = tomllib.loads((output / "server.toml").read_text())
        operator = tomllib.loads((output / "operator.toml").read_text())
        assert configured["signing_key_expires_at"] == original_server["signing_key_expires_at"]
        assert operator["server_key_expires_at"] == original_operator["server_key_expires_at"]
        assert operator["credential"] == original_operator["credential"]
        report["checks"].append("production-restore-and-private-config-preserve-original-trust-validity-and-credential")
        before_retry = files(output)
        try:
            materialize(ROOT, seed, output, parameters, binary, env)
        except FileExistsError:
            pass
        else:
            raise AssertionError("Existing fixture was replaced")
        assert files(output) == before_retry
        report["checks"].append("occupied-instance-refuses-without-overwrite")
        wrong = dict(parameters, cell_id="G6-058/ubuntu24-x86_64/local/native-quic/ES256")
        try:
            materialize(ROOT, seed, PRIVATE / "wrong-signing", wrong, binary, env)
        except FixtureError as error:
            assert str(error) == "fixture-signing-profile"
        else:
            raise AssertionError("Wrong signing cohort accepted")
        assert not (PRIVATE / "wrong-signing").exists()
        report["checks"].append("wrong-signing-seed-refuses-before-state-publication")
        browser = dict(parameters, cell_id="G6-058/ubuntu24-x86_64/local/chrome-wss-webrtc/EdDSA", browser_origin="https://browser.fixture.invalid")
        try:
            materialize(ROOT, seed, PRIVATE / "missing-browser", browser, binary, env)
        except FixtureError as error:
            assert str(error) == "fixture-browser-transports"
        else:
            raise AssertionError("Native-only seed accepted as browser fixture")
        assert not (PRIVATE / "missing-browser").exists()
        report["checks"].append("incomplete-browser-seed-refuses-without-native-only-downgrade")
        start("materialized-server", output / "server.toml")
        assert rooms("materialized-list", output / "operator.toml") == expected
        frames = request("materialized-original", output / "operator.toml", "operation.get", dict(operation_id="transport-fixture-original"))
        record = next(v for v in frames if v["type"] == "operation.updated")["payload"]
        assert record["state"] == "completed" and record["operation_id"] == "transport-fixture-original"
        report["materialization"] = summary
        if actors:
            phase = "actor-enrollment"
            directory = output / "actors"
            enrolled = enroll_actors(ROOT, output, state / "operator.application.toml", binary, directory, run)
            report["actor_enrollment"] = enrolled
            report["checks"].append("four-identities-five-independent-scoped-devices-enrolled-through-production-cli")
            installed = files(directory)
            phase = "actor-original-resume"
            resumed = enroll_actors(ROOT, output, state / "operator.application.toml", binary, directory,
                lambda name, args, data=None: run("resume-" + name, args, data), resume=True)
            assert resumed == enrolled and files(directory) == installed
            report["checks"].append("explicit-original-enrollment-resume-preserves-all-ids-keys-credentials-and-expiry")
            records = json.loads((directory / "actors.json").read_text())
            assert len(records) == 5
            for name, actor in records.items():
                assert rooms(name + "-scoped-native-read", Path(actor["config"])) == []
            report["checks"].append("all-five-enrolled-devices-perform-signed-native-read-under-their-own-authority")
            forbidden = json.loads(run("actor-cannot-provision", ["provision", "--config", directory / "participant-1/application.toml"],
                dict(type="identity", operation_id="transport-actor-forbidden-identity", alias="transport-forbidden", kind="human"), success=False))
            assert forbidden["result"]["Err"]["code"] == "forbidden"
            report["checks"].append("participant-credential-cannot-provision-an-identity")
            changed = PRIVATE / "changed-admin.toml"
            changed.write_bytes((state / "operator.application.toml").read_bytes() + b"\n")
            phase = "changed-actor-issuer-refusal"
            def must_not_execute(*args):
                raise AssertionError("Changed bound issuer made a public call")
            try:
                enroll_actors(ROOT, output, changed, binary, directory, must_not_execute, resume=True)
            except FixtureError as error:
                assert str(error) == "fixture-actor-plan"
            else:
                raise AssertionError("Changed issuer configuration adopted an old plan")
            assert files(directory) == installed
            report["checks"].append("changed-bound-issuer-refuses-before-public-calls-and-preserves-original-plan")
            before_revocation = {name: [] for name in records.keys() - {"participant-1"}}
            if conversation:
                phase = "public-sdk-conversation-control"
                config_directory = PRIVATE / "sdk-configs"
                config_directory.mkdir(mode=0o700)
                configured_actors = {}
                for name, actor in records.items():
                    native_path = Path(actor["config"])
                    native_config = tomllib.loads(native_path.read_text())
                    for key in ("trust_certificate", "signing_private_key", "server_public_key"):
                        native_config[key] = str(native_path.parent / native_config[key])
                    application_config = tomllib.loads((native_path.parent / "application.toml").read_text())
                    sdk_config = config_directory / (name + ".toml")
                    publish(sdk_config, toml_bytes(dict(tenant_id="tnt_default", control=native_config, application=application_config)))
                    configured_actors[name] = dict(identity_id=actor["identity_id"], config=str(sdk_config))
                input_path = PRIVATE / "conversation-input.json"
                publish(input_path, (json.dumps(dict(format="connect/transport-conversation-input/1", actors=configured_actors,
                    executable=str(binary), binary_sha256=report["binary_sha256"], source_revision=report["source_revision"]), indent=2) + "\n").encode())
                sdk_inputs = runpy.run_path(str(ROOT / "scripts/check-gate-6-transports.py"))["sdk_inputs"]
                sdk_before = sdk_inputs()
                report["sdk_dist_inputs"] = sdk_before
                code = bounded(["node", "scripts/check-transport-conversation.mjs", str(input_path), str(OUTPUT / "conversation.json")],
                    PRIVATE / "conversation-control.log", env, 180)
                report["conversation_exit_code"] = code
                if (OUTPUT / "conversation.json").is_file():
                    report["conversation_report_sha256"] = digest(OUTPUT / "conversation.json")
                assert code == 0
                observed = json.loads((OUTPUT / "conversation.json").read_text())
                assert observed["passed"] and observed["all_observed_helpers_joined"]
                assert observed["maximum_active_members_observed"] == 4 and observed["concurrent_session_intents_observed"] == 2
                assert observed["source_revision"] == report["source_revision"] and observed["binary_sha256"] == report["binary_sha256"]
                report["checks"].append("public-sdk-membership-audiences-two-session-intents-and-independent-device-original-recovery")
                before_revocation = {name: rooms(name + "-before-revocation", Path(records[name]["config"])) for name in sorted(before_revocation)}
                assert all(len(value) == 1 for value in before_revocation.values())
                assert len({value[0]["id"] for value in before_revocation.values()}) == 1
            revoke = ["revoke", "--config", state / "operator.application.toml", "--operation-id", "transport-actor-revoke-primary-1", records["participant-1"]["credential_id"]]
            assert json.loads(run("revoke-primary-device", revoke))["result"]["Ok"]["revoked"] is True
            refusal = json.loads(run("revoked-primary-native-refusal", ["request", "--config", records["participant-1"]["config"], "--envelope"],
                dict(command=dict(type="conversation.list", payload=dict(filter={}, limit=20)), scope={}), success=False))
            assert refusal["result"]["Err"]["code"] == "unauthorized"
            for name in sorted(records.keys() - {"participant-1"}):
                assert rooms(name + "-after-revocation", Path(records[name]["config"])) == before_revocation[name]
            assert files(directory) == installed
            report["checks"].append("revoked-primary-device-refuses-while-independent-replacement-and-other-devices-remain-usable")
        stop()
        report["checks"].append("normal-server-and-native-client-read-original-state-through-materialized-config")
        assert files(seed) == original_seed
        assert len(report["servers"]) == 2 and all(v["original_joined"] for v in report["servers"])
        report["checks"].append("seed-unchanged-and-original-server-groups-joined")
        report["passed"] = True
    except BaseException as error:
        (PRIVATE / "failure.log").write_text(traceback.format_exc())
        report["failure"] = dict(phase=phase, name=type(error).__name__, line=traceback.extract_tb(error.__traceback__)[-1].lineno)
        if isinstance(error, FixtureError):
            report["failure"]["code"] = str(error)
    finally:
        if server is not None and server.poll() is None:
            os.killpg(server.pid, signal.SIGKILL)
            server.wait()
        for stream in streams:
            stream.close()
        report["source_inputs_unchanged"] = before is not None and source() == before
        report["executables_unchanged"] = binary is not None and digest(binary) == report["binary_sha256"]
        if sdk_before is not None:
            report["sdk_dist_inputs_unchanged"] = sdk_inputs() == sdk_before
            report["passed"] = bool(report["passed"] and report["sdk_dist_inputs_unchanged"])
        report["passed"] = bool(report["passed"] and report["source_inputs_unchanged"] and report["executables_unchanged"])
        report["cacheable_build"] = bool(report.get("compile_exit_code") == 0 and report["source_inputs_unchanged"] and report["executables_unchanged"])
        (OUTPUT / "rehearsal.json").write_text(json.dumps(report, indent=2) + "\n")
        if os.environ.get("GITHUB_OUTPUT"):
            with open(os.environ["GITHUB_OUTPUT"], "a") as stream:
                stream.write("cacheable_build=" + str(report["cacheable_build"]).lower() + "\n")
    print(json.dumps(dict(fixture_rehearsal_passed=report["passed"], matrix_qualified=False, task_complete=False)))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--actors", action="store_true", help="Include scoped enrollment, original resume and current-authority checks")
    parser.add_argument("--conversation", action="store_true", help="Include actual public Node SDK membership and Session control")
    args = parser.parse_args()
    raise SystemExit(main(args.actors, args.conversation))
