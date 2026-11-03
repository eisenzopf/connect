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
from lib.transport_actors import CONTROLLERS, DEVICES, enroll_actors, exact
from lib.transport_capture import NativeCapture
from lib.transport_av import prepare as prepare_av, decode_completed as decode_completed_av
from lib.transport_namespace import NativeNamespace
from lib.transport_fixture import FixtureError, RELIABLE_PAYLOAD_BYTES, materialize, publish, toml_bytes
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


def main(actors=False, conversation=False, data_workload=False, signing_algorithm="EdDSA", network_profile="local", media_workload=False, host_capture=True, native_port=None, arm64_component=False):
    global PRIVATE, OUTPUT
    assert type(host_capture) is bool and (host_capture or media_workload and network_profile == "local")
    assert native_port is None or (type(native_port) is int and 1024 <= native_port <= 65535 and media_workload and network_profile == "local")
    if media_workload:
        data_workload = actors = conversation = True
        PRIVATE, OUTPUT = BASE / "private/media", BASE / "evidence/media"
    elif data_workload:
        actors = conversation = True
        PRIVATE, OUTPUT = BASE / "private/data", BASE / "evidence/data"
    elif conversation:
        actors = True
        PRIVATE, OUTPUT = BASE / "private/conversation", BASE / "evidence/conversation"
    elif actors:
        PRIVATE, OUTPUT = BASE / "private/actors", BASE / "evidence/actors"
    assert signing_algorithm in {"EdDSA", "ES256"}
    os.umask(0o077)
    if arm64_component:
        assert platform.system() == "Linux" and platform.machine() == "aarch64"
        assert network_profile == "local"
    if platform.system() == "Linux":
        assert platform.machine() == ("aarch64" if arm64_component else "x86_64")
        release = platform.freedesktop_os_release()
        assert release["ID"] == "ubuntu" and release["VERSION_ID"] == "24.04"
        cpuinfo = Path("/proc/cpuinfo").read_text()
        assert re.search(r"^CPU architecture\s*:\s*8\s*$" if arm64_component else r"^vendor_id\s*:", cpuinfo, re.M)
        server_platform, target = (("ubuntu24-arm64", "aarch64-unknown-linux-gnu") if arm64_component else
            ("ubuntu24-x86_64", "x86_64-unknown-linux-gnu"))
        host = dict(os="linux", architecture=platform.machine(), kernel_release=platform.release(), distribution=release["PRETTY_NAME"])
    else:
        assert platform.system() == "Darwin" and platform.machine() == "arm64"
        assert subprocess.check_output(["/usr/sbin/sysctl", "-n", "hw.optional.arm64"], text=True).strip() == "1"
        server_platform, target = "macos-arm64", "aarch64-apple-darwin"
        host = dict(os="darwin", architecture="arm64", kernel_release=platform.release(), version=platform.mac_ver()[0])
    rust = subprocess.check_output(["rustc", "-Vv"], text=True)
    assert "rustc 1.97.1 " in rust and "host: " + target in rust
    host["rust"] = rust
    host["platform"] = server_platform
    assert network_profile in {"local", "remote"}
    assert network_profile == "local" or server_platform == "ubuntu24-x86_64"
    network_cell = "local" if network_profile == "local" else "remote-profile-fixture"
    assert os.environ.get("TRANSPORT_NETWORK", network_cell) == network_cell
    assert os.environ.get("TRANSPORT_TARGET", server_platform) == server_platform
    PRIVATE.mkdir(parents=True)
    OUTPUT.mkdir(parents=True)
    source = runpy.run_path(str(ROOT / "scripts/check-gate6-linux-hosting.py"))["sources"]
    bounded = runpy.run_path(str(ROOT / "scripts/check-linux-package-build.py"))["run_bounded"]
    env = fixture_environment()
    env["CARGO_PROFILE_RELEASE_DEBUG"] = "0"
    if target == "aarch64-apple-darwin":
        env.update(MACOSX_DEPLOYMENT_TARGET="15.0", OPUS_STATIC="1",
            LIBOPUS_NO_PKG="1", CMAKE_POLICY_VERSION_MINIMUM="3.5")
    report = dict(format="connect/transport-fixture-rehearsal/1", source_revision=subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(), passed=False, checks=[], commands=[],
        servers=[], signing_algorithm=signing_algorithm, network_profile=network_profile, host=host, task_complete=False, matrix_qualified=False, cell_qualified=False, whole_gate_qualified=False,
        scope="Native macOS-arm64 or Ubuntu24.04 x86_64/explicit ARM64 component production CLI configuration and public Node SDK native stages. Optional data workload holds six actual data Connections across four participants/two Sessions, transfers six64KiB payloads, observes the owned UDP listener and actual client/server topology and requires private packet capture. Optional media workload adds six Opus/VP8 sources with an independently decoded 60-second native component. Paired AV-sync, live upgrade/replacement media ownership, browser, complete remote transport cells, SIP, TURN and model/provider execution remain unqualified. All complete matrix cells remain unqualified.")
    report["arm64_component_only"] = arm64_component
    report["host_capture_requested"] = host_capture
    report["requested_native_port"] = native_port
    report["host_capture_required_for_complete_cell"] = True
    if not host_capture:
        report["packet_capture"] = dict(format="connect/native-private-packet-capture/1", observed=False,
            executed=False, skipped=True, reason="explicit-local-media-component-without-host-capture",
            scope="Full local media workload with public SDK packet/clock recordings; privileged host capture unavailable. No complete transport-cell qualification.")
    before = None
    server = None
    streams = []
    phase = "dependencies"
    binary = None
    peer = None
    sdk_before = None
    capture = None
    namespace = None
    server_pid = None
    media_sources = None
    try:
        code = bounded(["bash", "scripts/prepare-dependencies.sh"], PRIVATE / "dependencies.log", env, 600)
        assert code == 0
        before = source()
        source_path = OUTPUT / "source-inputs.json"
        source_path.write_text(json.dumps(before, sort_keys=True) + "\n")
        report.update(source_inputs_sha256=digest(source_path), source_count=len(before))
        if media_workload:
            phase = "synthetic-opus-vp8-source-preparation"
            media_sources = prepare_av(PRIVATE / "av-sources")
            report["native_av_source_manifest_sha256"] = digest(PRIVATE / "av-sources/manifest.json")
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
        if media_workload:
            phase = "compile-independent-physical-media-peer"
            peer_log = PRIVATE / "peer-compile.log"
            code = bounded(["cargo", "build", "--release", "--locked", "--message-format=json",
                "--manifest-path", "examples/rust-sdk-journey/Cargo.toml"], peer_log, env, 3600)
            report["peer_compile_exit_code"] = code
            assert code == 0
            peers = []
            for line in peer_log.read_text().splitlines():
                try:
                    value = json.loads(line)
                except ValueError:
                    continue
                if (value.get("reason") == "compiler-artifact" and value.get("executable")
                        and value["target"]["name"] == "connect-external-rust-journey"):
                    assert not value["profile"]["test"] and value["profile"]["opt_level"] == "3"
                    assert "test-support" not in value["features"]
                    peers.append(Path(value["executable"]))
            assert len(peers) == 1
            report["peer_binary_sha256"] = digest(peers[0])
            peer = PRIVATE / "physical-media-peer"
            seal_executable(peers[0], peer, report["peer_binary_sha256"], pool=PRIVATE / "sealed")

        if network_profile == "remote":
            phase = "owned-remote-network-setup"
            namespace = NativeNamespace(PRIVATE / "network", ROOT, env)
            namespace.start()

        def run(name, arguments, data=None, *, success=True):
            nonlocal phase
            phase = name
            path = PRIVATE / (name + ".output")
            source_path = PRIVATE / (name + ".input")
            source_path.write_bytes(b"" if data is None else json.dumps(data).encode())
            with source_path.open("rb") as incoming, path.open("wb") as out, (PRIVATE / (name + ".stderr")).open("wb") as errors:
                command = [str(binary), *map(str, arguments)]
                if namespace is not None and arguments[0] == "request":
                    command = namespace.client_command(command)
                child = subprocess.Popen(command, cwd=ROOT, env=env,
                    stdin=incoming, stdout=out, stderr=errors, start_new_session=True)
                joined = False
                try:
                    result = child.wait(timeout=30)
                    joined = True
                finally:
                    if not joined:
                        if namespace is not None:
                            namespace.client_forced = True
                        try:
                            os.killpg(child.pid, signal.SIGKILL)
                        except PermissionError:
                            subprocess.run(["sudo", "-n", "/bin/kill", "-KILL", "--", "-" + str(child.pid)],
                                env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL, timeout=5, check=False)
                        except ProcessLookupError:
                            pass
                        child.wait(timeout=5)
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
            nonlocal server, server_pid, phase
            phase = name
            path = PRIVATE / (name + ".log")
            stream = path.open("wb")
            streams.append(stream)
            server_env = dict(env, RUST_LOG="info,connect_service::canonical_native=debug,connect_service::canonical_native_data=debug,rvoip_uctp::native_subscription_preflight=debug,rvoip_core::native_catalog=debug") if data_workload else env
            command = ([str(binary), "serve", "--config", str(config)] if namespace is None else
                namespace.server_command(binary, config, server_env))
            server = subprocess.Popen(command, cwd=ROOT, env=server_env,
                stdin=subprocess.DEVNULL, stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
            if namespace is not None:
                namespace.owner = server
            until = time.monotonic() + 15
            while b"ready transport=quic" not in path.read_bytes():
                assert server.poll() is None and time.monotonic() < until
                time.sleep(.05)
            server_pid = server.pid if namespace is None else namespace.identify_server(server)
            actual_executable = (Path(f"/proc/{server_pid}/exe") if platform.system() == "Linux" else
                Path(subprocess.check_output(["/bin/ps", "-ww", "-p", str(server_pid), "-o", "comm="], text=True).strip()))
            assert actual_executable.is_absolute() and digest(actual_executable) == report["binary_sha256"]
            report["servers"].append(dict(name=name, running_binary_sha256=report["binary_sha256"], original_joined=False, managed_by_systemd=namespace is not None))

        def stop():
            nonlocal server
            if namespace is not None:
                namespace.stop_server(server)
            else:
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
        native, http = native_port or port(socket.SOCK_DGRAM), port(socket.SOCK_STREAM)
        run("initialize", ["init", "--directory", state, "--bind", f"127.0.0.1:{native}", "--http-bind", f"127.0.0.1:{http}", "--signing-algorithm", signing_algorithm])
        initialized = files(state)
        run("unsupported-init-signing", ["init", "--directory", PRIVATE / "unsupported-signing",
            "--signing-algorithm", "RS256"], success=False)
        assert not (PRIVATE / "unsupported-signing").exists()
        other_algorithm = "ES256" if signing_algorithm == "EdDSA" else "EdDSA"
        run("existing-init-signing-refusal", ["init", "--directory", state,
            "--signing-algorithm", other_algorithm], success=False)
        assert files(state) == initialized
        initial_server = tomllib.loads((state / "server.toml").read_text())
        initial_operator = tomllib.loads((state / "operator.toml").read_text())
        assert initial_server["signing_algorithm"] == initial_operator["signing_algorithm"] == initial_operator["server_algorithm"] == signing_algorithm
        public_key = (state / "server-signing.pub").read_bytes()
        assert len(public_key) == (32 if signing_algorithm == "EdDSA" else 65)
        if signing_algorithm == "ES256":
            assert public_key[0] == 4
        assert len({digest(state / name) for name in ("server-signing.der", "operator-signing.der", "private-key.der")}) == 3
        report["checks"].append("selected-signing-cohort-generated-with-independent-keys-and-no-existing-installation-replacement")
        # The production default is a smaller ordinary-message installation.
        # Configure this fresh synthetic installation for its required 64KiB
        # decoded native workload before starting or sealing the stopped seed.
        # Existing seeds remain immutable and must refuse insufficient limits.
        server_config = initial_server
        initialized_message_bytes = server_config["limits"]["message_bytes"]
        server_config["limits"]["message_bytes"] = max(initialized_message_bytes, RELIABLE_PAYLOAD_BYTES)
        report["workload_message_limits"] = dict(initialized_message_bytes=initialized_message_bytes,
            configured_message_bytes=server_config["limits"]["message_bytes"],
            required_decoded_payload_bytes=RELIABLE_PAYLOAD_BYTES)
        if namespace is not None:
            server_config.update(network_profile="remote", bind=f"{namespace.server_ip}:{native}")
            operator_config = tomllib.loads((state / "operator.toml").read_text())
            operator_config.update(network_profile="remote", endpoint=f"{namespace.server_ip}:{native}")
            (state / "operator.toml").write_bytes(toml_bytes(operator_config))
        (state / "server.toml").write_bytes(toml_bytes(server_config))
        report["checks"].append("fresh-fixture-message-limit-covers-required-full-size-native-reliable-payload")
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
        identity_prefix = "G6-058-component/" if arm64_component else "G6-058/"
        parameters = dict(cell_id=identity_prefix + server_platform + "/" + network_cell + "/native-quic/" + signing_algorithm,
            candidate_source_revision=report["source_revision"], candidate_binary_sha256=report["binary_sha256"],
            seed_manifest_sha256=digest(seed / "manifest.json"), browser_origin=None,
            topology=dict(kind="local-loopback" if namespace is None else "ubuntu-netns", server_platform=server_platform, client_platform=server_platform,
                client_emulated=False, native_endpoint=original_operator["endpoint"], server_name=original_operator["server_name"]))
        phase = "materialize"
        summary = materialize(ROOT, seed, output, parameters, binary, env, arm64_component=arm64_component)
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
            materialize(ROOT, seed, output, parameters, binary, env, arm64_component=arm64_component)
        except FileExistsError:
            pass
        else:
            raise AssertionError("Existing fixture was replaced")
        assert files(output) == before_retry
        report["checks"].append("occupied-instance-refuses-without-overwrite")
        wrong = dict(parameters, cell_id=identity_prefix + server_platform + "/" + network_cell + "/native-quic/" + other_algorithm)
        try:
            materialize(ROOT, seed, PRIVATE / "wrong-signing", wrong, binary, env, arm64_component=arm64_component)
        except FixtureError as error:
            assert str(error) == "fixture-signing-profile"
        else:
            raise AssertionError("Wrong signing cohort accepted")
        assert not (PRIVATE / "wrong-signing").exists()
        report["checks"].append("wrong-signing-seed-refuses-before-state-publication")
        browser = dict(parameters, cell_id=identity_prefix + server_platform + "/" + network_cell + "/chrome-wss-webrtc/" + signing_algorithm, browser_origin="https://browser.fixture.invalid")
        try:
            materialize(ROOT, seed, PRIVATE / "missing-browser", browser, binary, env, arm64_component=arm64_component)
        except FixtureError as error:
            assert str(error) == ("fixture-arm64-component" if arm64_component else "fixture-browser-transports")
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
            enrolled = enroll_actors(ROOT, output, state / "operator.application.toml", binary, directory, run,
                movement_controllers=media_workload)
            report["actor_enrollment"] = enrolled
            report["checks"].append("four-identities-" + ("seven" if media_workload else "five") + "-independent-scoped-devices-enrolled-through-production-cli")
            installed = files(directory)
            phase = "actor-original-resume"
            resumed = enroll_actors(ROOT, output, state / "operator.application.toml", binary, directory,
                lambda name, args, data=None: run("resume-" + name, args, data), resume=True,
                movement_controllers=media_workload)
            assert resumed == enrolled and files(directory) == installed
            report["checks"].append("explicit-original-enrollment-resume-preserves-all-ids-keys-credentials-and-expiry")
            records = json.loads((directory / "actors.json").read_text())
            assert set(records) == set(DEVICES + CONTROLLERS if media_workload else DEVICES)
            for name, actor in records.items():
                assert rooms(name + "-scoped-native-read", Path(actor["config"])) == []
            report["checks"].append("all-enrolled-devices-perform-signed-native-read-under-their-own-authority")
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
                enroll_actors(ROOT, output, changed, binary, directory, must_not_execute, resume=True,
                    movement_controllers=media_workload)
            except FixtureError as error:
                assert str(error) == "fixture-actor-plan"
            else:
                raise AssertionError("Changed issuer configuration adopted an old plan")
            assert files(directory) == installed
            report["checks"].append("changed-bound-issuer-refuses-before-public-calls-and-preserves-original-plan")
            before_revocation = {name: [] for name in records.keys() - {"participant-1"}}
            if data_workload:
                phase = "trusted-native-endpoint-verification"
                verification = []
                for name, actor in records.items():
                    # Public issuance stdout exposes its existing Endpoint receipt;
                    # never derive IDs or inspect the installer's private intent.
                    issued_output = json.loads((PRIVATE / (name + "-enrollment.output")).read_bytes())
                    endpoint = issued_output["endpoint"]
                    assert endpoint["kind"] == "endpoint" and endpoint["identity_id"] == actor["identity_id"]
                    assert endpoint["device_id"] == actor["device_id"] and endpoint["transport"] == "quic"
                    assert endpoint["capabilities"] == ["data"] and endpoint["verification_state"] == "unverified"
                    assert type(endpoint["revision"]) is int
                    proof = PRIVATE / (name + "-scoped-native-read.output")
                    evidence = "native-signed-control-sha256-" + digest(proof)
                    verification.append(dict(type="verify_endpoint", operation_id="transport-verify-" + name + "-1",
                        endpoint_id=endpoint["endpoint_id"], expected_revision=endpoint["revision"], evidence_ref=evidence))
                if media_workload:
                    # Verify the separate full-capability receipts. The original
                    # data-only issuance receipts stay unverified, as in app setup.
                    verification = []
                    media_endpoints = {}
                    for name, actor in records.items():
                        if name in CONTROLLERS:
                            continue
                        issued = tomllib.loads(Path(actor["config"]).read_text())
                        command = dict(type="endpoint", operation_id="transport-media-endpoint-" + name + "-1",
                            identity_id=actor["identity_id"], device=dict(id=actor["device_id"],
                                kind=issued["device_kind"], platform=issued["device_platform"]), transport="quic",
                            address="quic://" + original_operator["endpoint"] + "/transport-media/" + actor["device_id"],
                            capabilities=["audio", "data", "video"], secret_ref=None)
                        result = json.loads(run("media-endpoint-" + name, ["provision", "--config", state / "operator.application.toml"], command))["result"]["Ok"]
                        assert result["device_id"] == actor["device_id"] and result["capabilities"] == command["capabilities"]
                        assert result["verification_state"] == "unverified" and result["transport"] == "quic"
                        media_endpoints[name] = result["endpoint_id"]
                        replay = json.loads(run("media-endpoint-original-" + name, ["provision", "--config", state / "operator.application.toml"], command))["result"]["Ok"]
                        assert replay == result
                        verification.append(dict(type="verify_endpoint", operation_id="transport-media-verify-" + name + "-1",
                            endpoint_id=result["endpoint_id"], expected_revision=result["revision"],
                            evidence_ref="native-signed-control-sha256-" + digest(PRIVATE / (name + "-scoped-native-read.output"))))
                assert len({v["endpoint_id"] for v in verification}) == 5
                plan_path = PRIVATE / "endpoint-verifications.json"
                exact(plan_path, (json.dumps(verification, sort_keys=True, indent=2) + "\n").encode())
                self_verify = dict(verification[0], operation_id="transport-self-verify-refusal-1")
                refused = json.loads(run("participant-cannot-verify-endpoint", ["provision", "--config", directory / "participant-1/application.toml"], self_verify, success=False))
                assert refused["result"]["Err"]["code"] == "forbidden"
                for index, command in enumerate(verification):
                    response = json.loads(run("trusted-endpoint-verify-" + str(index), ["provision", "--config", state / "operator.application.toml"], command))
                    current = response["result"]["Ok"]
                    assert current["kind"] == "endpoint" and current["endpoint_id"] == command["endpoint_id"]
                    assert current["verification_state"] == "verified" and current["revision"] == command["expected_revision"] + 1
                    replay = json.loads(run("trusted-endpoint-verify-original-" + str(index), ["provision", "--config", state / "operator.application.toml"], command))
                    assert replay == response
                for name, actor in records.items():
                    resumed_output = json.loads(run("post-verification-resume-" + name,
                        ["issue-resume", "--config", state / "operator.application.toml", "--installation", Path(actor["config"]).parent]))
                    assert resumed_output["endpoint"]["verification_state"] == ("unverified" if media_workload else "verified")
                    assert resumed_output["endpoint"]["device_id"] == actor["device_id"]
                assert files(directory) == installed
                report["endpoint_verification"] = dict(initial_unverified=len(verification), signed_control_proofs=len(records),
                    endpoints_verified=len(verification), original_verification_replays=len(verification), verified_endpoint_issuance_resumes=0 if media_workload else 5,
                    unchanged_unverified_issuance_resumes=len(records) if media_workload else 0,
                    full_media_endpoints_verified=5 if media_workload else 0,
                    control_only_unverified_devices=len(CONTROLLERS) if media_workload else 0,
                    participant_self_verification_refused=True,
                    private_plan_sha256=digest(plan_path), public_administration_only=True)
                report["checks"].append("native-endpoints-remain-unverified-until-trusted-backend-signed-control-proof-and-original-verification")
                if host_capture:
                    phase = "owned-native-packet-capture-start"
                    capture = NativeCapture(PRIVATE / "packet-capture", native, env,
                        interface=None if namespace is None else namespace.interface,
                        server_ip="127.0.0.1" if namespace is None else namespace.server_ip,
                        peer_ip="127.0.0.1" if namespace is None else namespace.client_ip,
                        media_workload=media_workload)
                    capture.start()
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
                    if media_workload:
                        configured_actors[name].update(device_id=actor["device_id"], native_config=str(native_path),
                            media_endpoint_id=media_endpoints.get(name))
                input_path = PRIVATE / "conversation-input.json"
                publish(input_path, (json.dumps(dict(format="connect/transport-conversation-input/1", actors=configured_actors,
                    executable=str(binary), binary_sha256=report["binary_sha256"], source_revision=report["source_revision"],
                    physical_peer_executable=str(peer) if media_workload else None,
                    physical_peer_sha256=report.get("peer_binary_sha256"),
                    server_pid=server_pid, native_endpoint=original_operator["endpoint"], signing_algorithm=signing_algorithm,
                    server_platform=server_platform, network_profile=network_profile,
                    client_ip=None if namespace is None else namespace.client_ip,
                    media_source_directory=str(PRIVATE / "av-sources") if media_workload else None,
                    media_output_directory=str(PRIVATE / "av-received") if media_workload else None), indent=2) + "\n").encode())
                sdk_inputs = runpy.run_path(str(ROOT / "scripts/check-gate-6-transports.py"))["sdk_inputs"]
                sdk_before = sdk_inputs()
                report["sdk_dist_inputs"] = sdk_before
                command = ["node", "scripts/check-transport-conversation.mjs", str(input_path), str(OUTPUT / "conversation.json")]
                if media_workload:
                    command.append("--media-workload")
                elif data_workload:
                    command.append("--data-workload")
                code = (bounded(command, PRIVATE / "conversation-control.log", env, 300 if media_workload else 180) if namespace is None else
                    namespace.run_client(command, PRIVATE / "conversation-control.log", 300 if media_workload else 180))
                report["conversation_exit_code"] = code
                if (OUTPUT / "conversation.json").is_file():
                    report["conversation_report_sha256"] = digest(OUTPUT / "conversation.json")
                observed = json.loads((OUTPUT / "conversation.json").read_text())
                # A later control/recovery failure must not discard the already
                # completed native media observation. Preserve both results.
                decoded = None
                if media_workload:
                    try:
                        decoded = decode_completed_av(observed, PRIVATE / "av-received", media_sources)
                        if decoded is not None:
                            report["native_av_decoded"] = decoded
                    except Exception as decode_error:
                        (PRIVATE / "native-av-decoder-failure.log").write_text(traceback.format_exc())
                        report["native_av_decoder_failure"] = dict(name=type(decode_error).__name__,
                            line=traceback.extract_tb(decode_error.__traceback__)[-1].lineno)
                assert code == 0
                assert observed["passed"] and observed["all_observed_helpers_joined"]
                assert observed["maximum_active_members_observed"] == 4 and observed["concurrent_session_intents_observed"] == 2
                assert observed["signing_algorithm"] == signing_algorithm and observed["host"]["platform"] == server_platform
                assert observed["source_revision"] == report["source_revision"] and observed["binary_sha256"] == report["binary_sha256"]
                if data_workload:
                    assert observed["actual_connection_workload_observed"] and observed["actual_topology_verified"]
                    workload = observed["data_workload"]
                    assert workload["connections"] == 6 and workload["participants"] == 4 and workload["sessions"] == 2
                    assert workload["verified_payloads"] == workload["original_connections_retired"] == 6
                    assert workload["total_received_bytes"] == 6 * 65536
                if media_workload:
                    phase = "independent-native-opus-vp8-decode"
                    assert decoded is not None and decoded["passed"]
                    report["checks"].append("six-native-opus-vp8-routes-have-independently-decoded-source-identity-and-moving-360p-content")
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
        if capture is not None:
            phase = "owned-native-packet-capture-finish"
            report["packet_capture"] = capture.finish()
            assert report["packet_capture"]["observed"]
            assert report["packet_capture"]["counts"]["to_server_udp_bytes"] >= 6 * 65536
            assert report["packet_capture"]["counts"]["from_server_udp_bytes"] >= 6 * 65536
            report["checks"].append("private-owned-" + ("loopback" if namespace is None else "veth") + "-packet-capture-validates-both-native-directions-with-zero-kernel-drops-and-original-owner-join")
        stop()
        if namespace is not None:
            report["network_namespace"] = namespace.close()
            assert report["network_namespace"]["passed"] and namespace.servers_verified == 2
            report["checks"].append("owned-remote-profile-systemd-server-and-separate-client-namespace-originals-retired")
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
        if capture is not None:
            report["packet_capture"] = capture.finish()
            report["passed"] = bool(report["passed"] and report["packet_capture"]["observed"])
        if namespace is not None and server is not None:
            try:
                namespace.stop_server(server)
                if report["servers"]:
                    report["servers"][-1]["original_joined"] = True
                server = None
            except (AssertionError, OSError, ValueError, subprocess.SubprocessError) as error:
                report["failure_cleanup"] = dict(systemd_stop_confirmed=False, name=type(error).__name__)
        if namespace is None and server is not None and server.poll() is None:
            server.send_signal(signal.SIGTERM)
            forced = False
            try:
                code = server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                forced = True
                os.killpg(server.pid, signal.SIGKILL)
                code = server.wait()
            retired = False
            try:
                os.killpg(server.pid, 0)
            except ProcessLookupError:
                retired = True
            report["failure_cleanup"] = dict(server_exit_code=code, forced=forced, original_group_retired=retired)
            if code == 0 and not forced and retired and report["servers"]:
                report["servers"][-1]["original_joined"] = True
        if namespace is not None:
            report["network_namespace"] = namespace.close()
            report["passed"] = bool(report["passed"] and report["network_namespace"]["passed"])
        for stream in streams:
            stream.close()
        if data_workload:
            stages, data_stages, data_checkpoints, cleanup_stages, custodians = [], [], [], [], []
            for path in (PRIVATE / "seed-server.log", PRIVATE / "materialized-server.log"):
                if not path.exists():
                    continue
                for line in path.read_text().splitlines():
                    line = re.sub(r"\x1b\[[0-9;]*m", "", line)
                    if "Native custodian observed terminal lifetime" in line:
                        facts = {key: re.search(r'\b' + key + r'=(true|false)(?:\s|$)', line)
                            for key in ("stopped", "physical_closed", "route_live")}
                        if all(facts.values()):
                            custodians.append({key: match[1] == "true" for key, match in facts.items()})
                    if "Native offer cleanup stage" in line:
                        stage = re.search(r'\bstage="?([a-z-]{1,64})"?(?:\s|$)', line)
                        code = re.search(r'\bcode="?([a-z-]{1,64})"?(?:\s|$)', line)
                        if stage:
                            cleanup_stages.append(dict(stage=stage[1], code=code[1] if code else None))
                        continue
                    is_data_failure = "Native data did not complete" in line
                    is_data_checkpoint = "Native data stage reached" in line
                    if "Native offer did not complete" not in line and not is_data_failure and not is_data_checkpoint:
                        continue
                    stage = re.search(r'\bstage="?([a-z-]{1,64})"?(?:\s|$)', line)
                    if stage and is_data_checkpoint:
                        data_checkpoints.append(stage[1])
                        continue
                    code = re.search(r'\bcode="?([a-z-]{1,64})"?(?:\s|$)', line)
                    if stage and code:
                        if is_data_failure:
                            origin = re.search(r'\borigin="?([a-z-]{1,64})"?(?:\s|$)', line)
                            if origin:
                                data_stages.append(dict(stage=stage[1], code=code[1], origin=origin[1]))
                        else:
                            stages.append(dict(stage=stage[1], code=code[1]))
            report["native_offer_failure_stages"] = stages[:16]
            report["native_data_failure_stages"] = data_stages[:32]
            report["native_data_checkpoints"] = data_checkpoints[:64]
            report["native_offer_cleanup_stages"] = cleanup_stages[:64]
            report["native_offer_cleanup_stage_counts"] = {stage: sum(item["stage"] == stage for item in cleanup_stages)
                for stage in ("durable-retirement-recorded", "durable-retirement-retry", "durable-retirement-join")}
            report["native_custodian_terminal_observations"] = custodians[:32]
        report["source_inputs_unchanged"] = before is not None and source() == before
        report["executables_unchanged"] = (binary is not None and digest(binary) == report["binary_sha256"]
            and (not media_workload or peer is not None and digest(peer) == report["peer_binary_sha256"]))
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
    parser.add_argument("--arm64-component", action="store_true", help="Native Ubuntu24.04 ARM64 local component workload; never a required x86_64 matrix cell")
    parser.add_argument("--actors", action="store_true", help="Include scoped enrollment, original resume and current-authority checks")
    parser.add_argument("--conversation", action="store_true", help="Include actual public Node SDK membership and Session control")
    parser.add_argument("--data-workload", action="store_true", help="Include six actual native data Connections, four participants/two Sessions and observed loopback topology")
    parser.add_argument("--media-workload", action="store_true", help="Include six native Opus/VP8/data Connections with independent decoded 60-second observations; not complete matrix qualification")
    parser.add_argument("--without-host-capture", action="store_true", help="Run the same local media workload while explicitly recording privileged host capture unavailable; never complete cell qualification")
    parser.add_argument("--native-port", type=int, help="Explicit local media fixture UDP port for separately owned packet capture")
    parser.add_argument("--signing-algorithm", choices=("EdDSA", "ES256"), default="EdDSA", help="Generate and exercise the exact server/operator/actor signing cohort")
    parser.add_argument("--network-profile", choices=("local", "remote"), default="local", help="Owned native loopback or isolated Ubuntu systemd/network-namespace fixture")
    args = parser.parse_args()
    if args.arm64_component and args.network_profile != "local":
        parser.error("--arm64-component requires the local component profile")
    if args.without_host_capture and (not args.media_workload or args.network_profile != "local"):
        parser.error("--without-host-capture requires the explicit local --media-workload component")
    if args.native_port is not None and (not args.media_workload or args.network_profile != "local" or not 1024 <= args.native_port <= 65535):
        parser.error("--native-port requires an explicit local media fixture port in1024..65535")
    raise SystemExit(main(args.actors, args.conversation, args.data_workload, args.signing_algorithm, args.network_profile, args.media_workload, not args.without_host_capture, args.native_port, args.arm64_component))
