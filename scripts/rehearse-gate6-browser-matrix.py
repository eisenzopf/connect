#!/usr/bin/env python3
"""Fresh production-CLI/browser matrix admission on the owner's local ARM VM.

The admission stage retains the full matrix checklist as incomplete. It does
not substitute for the subsequent original media/movement/expiry/idle stages.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import runpy
import signal
import socket
import subprocess
import sys
import time
import tomllib
import traceback

from lib.browser_matrix_actors import browser_actor
from lib.package_environment import fixture_environment
from lib.sealed_executable import seal_executable
from lib.transport_actors import CONTROLLERS, DEVICES, enroll_actors
from lib.transport_fixture import materialize, publish, toml_bytes
from lib.transport_matrix import digest

ROOT = Path(__file__).resolve().parents[1]


def port(kind):
    with socket.socket(socket.AF_INET, kind) as reservation:
        reservation.bind(("127.0.0.1", 0))
        return reservation.getsockname()[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stage", choices=("admission", "programme"), required=True)
    parser.add_argument("--transport", choices=("chrome-webtransport", "chrome-wss-webrtc"), required=True)
    parser.add_argument("--signing", choices=("EdDSA", "ES256"), required=True)
    parser.add_argument("--image", required=True, help="Exact already installed local ARM browser image ID")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.stage == "programme" and args.transport != "chrome-webtransport":
        parser.error("Actual browser replacement programme currently requires Chrome WebTransport")
    if os.environ.get("GITHUB_ACTIONS") or os.environ.get("CI"):
        parser.error("Use the owner's local ARM resources")
    assert platform.system() == "Linux" and platform.machine() == "aarch64"
    release = platform.freedesktop_os_release()
    assert release["ID"] == "ubuntu" and release["VERSION_ID"] == "24.04"
    assert os.getuid() != 0
    assert re.fullmatch(r"sha256:[0-9a-f]{64}", args.image)
    image = json.loads(subprocess.check_output(["docker", "image", "inspect", args.image], text=True))[0]
    assert image["Id"] == args.image and image["Architecture"] == "arm64" and image["Os"] == "linux"
    assert not subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).strip()
    os.umask(0o077)
    output = args.output.resolve()
    assert output.is_relative_to((ROOT / ".local").resolve())
    output.mkdir(parents=True, exist_ok=False)
    private = output / "private"
    private.mkdir()
    env = fixture_environment()
    env["CARGO_PROFILE_RELEASE_DEBUG"] = "0"
    env["RUST_LOG"] = "warn,connect_service::canonical_replacement=debug,rvoip_core::replacement_commit=debug"
    sources = runpy.run_path(str(ROOT / "scripts/check-gate6-linux-hosting.py"))["sources"]
    bounded = runpy.run_path(str(ROOT / "scripts/check-linux-package-build.py"))["run_bounded"]
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    cell_id = "/".join(("G6-058", "ubuntu24-arm64", "local", args.transport, args.signing))
    report = dict(format="connect/browser-matrix-component/1", stage=args.stage, cell_id=cell_id,
        source_revision=revision, server_platform="ubuntu24-arm64", client_platform="ubuntu24-arm64",
        signing_algorithm=args.signing, transport=args.transport, browser_image_id=args.image,
        passed=False, cell_qualified=False, task_complete=False, whole_gate_qualified=False,
        full_media_programme_executed=False, commands=[], servers=[],
        scope="Actual fresh production CLI/normal TLS/enrolled browser-SDK " + args.stage + " component. Independent decoded media/capture/fallback/expiry/idle remain required.")
    state = dict(stage="preparing", controller_pid=os.getpid(), source_revision=revision,
                 cell_id=cell_id, source_guard_active=False)
    def save():
        (output / "state.json").write_text(json.dumps(state, indent=2) + "\n")
    save()
    before = sdk_before = executable = server = browser_owner = None
    log_handles = []
    cidfile = private / "browser-container-id"
    phase = "dependencies"
    sequence = 0

    def command(label, arguments, data=None, timeout=30):
        nonlocal sequence
        sequence += 1
        prefix = private / (f"{sequence:03d}-" + label)
        incoming = prefix.with_suffix(".input")
        incoming.write_bytes(b"" if data is None else json.dumps(data).encode())
        with incoming.open("rb") as source, prefix.with_suffix(".stdout").open("xb") as stdout, prefix.with_suffix(".stderr").open("xb") as stderr:
            subprocess.run([str(v) for v in arguments], cwd=ROOT, env=env, stdin=source,
                           stdout=stdout, stderr=stderr, check=True, timeout=timeout)
        report["commands"].append(dict(name=label, exit_code=0))
        return prefix.with_suffix(".stdout").read_bytes()

    def cli(label, arguments, data=None):
        return command(label, [executable, *arguments], data)

    def stop_server():
        nonlocal server
        if server is None:
            return
        server.send_signal(signal.SIGTERM)
        assert server.wait(timeout=10) == 0
        report["servers"][-1]["original_joined"] = True
        server = None

    def start_server(label, config, probe):
        nonlocal server
        handle = (private / (label + ".log")).open("xb")
        log_handles.append(handle)
        server = subprocess.Popen([str(executable), "serve", "--config", str(config)], cwd=ROOT, env=env,
                                  stdin=subprocess.DEVNULL, stdout=handle, stderr=subprocess.STDOUT, start_new_session=True)
        running = digest(Path(f"/proc/{server.pid}/exe"))
        assert running == report["binary_sha256"]
        report["servers"].append(dict(name=label, original_joined=False, running_binary_sha256=running))
        until = time.monotonic() + 10
        address = probe.split(":")
        while True:
            assert server.poll() is None, "Server exited before readiness"
            try:
                with socket.create_connection((address[0], int(address[1])), timeout=.1):
                    break
            except OSError:
                assert time.monotonic() < until, "Server readiness deadline"
                time.sleep(.02)

    try:
        command("dependencies", ["bash", "scripts/prepare-dependencies.sh"], timeout=600)
        command("sdk-build", ["npm", "run", "build", "--prefix", "sdk/typescript"], timeout=180)
        before = sources()
        (private / "source-inputs.json").write_text(json.dumps(before, sort_keys=True) + "\n")
        sdk_inputs = runpy.run_path(str(ROOT / "scripts/check-gate-6-transports.py"))["sdk_inputs"]
        sdk_before = sdk_inputs()
        report["source_inputs_sha256"] = digest(private / "source-inputs.json")
        report["sdk_dist_inputs"] = sdk_before
        state.update(stage="build-and-browser-admission", source_guard_active=True)
        save()
        phase = "production-build"
        build_log = private / "compile.log"
        code = bounded(["cargo", "build", "--release", "--locked", "--message-format=json",
                        "-p", "connect-server", "--bin", "connect"], build_log, env, 3600)
        assert code == 0
        artifacts = []
        for line in build_log.read_text().splitlines():
            try:
                value = json.loads(line)
            except ValueError:
                continue
            if value.get("reason") == "compiler-artifact" and value["target"]["name"] == "connect" and value.get("executable"):
                assert not value["profile"]["test"] and value["profile"]["opt_level"] == "3" and "test-support" not in value["features"]
                artifacts.append(Path(value["executable"]))
        assert len(artifacts) == 1
        report["binary_sha256"] = digest(artifacts[0])
        executable = private / "connect"
        seal_executable(artifacts[0], executable, report["binary_sha256"], pool=private / "sealed")
        phase = "fresh-configured-fixture"
        initial, seed, instance = private / "initial", private / "seed", private / "instance"
        native, http, wss, application, asset = (port(kind) for kind in
            (socket.SOCK_DGRAM, socket.SOCK_STREAM, socket.SOCK_STREAM, socket.SOCK_STREAM, socket.SOCK_STREAM))
        assert len({http, wss, application, asset}) == 4
        origin = f"https://127.0.0.1:{asset}"
        cli("initialize", ["init", "--directory", initial, "--bind", f"127.0.0.1:{native}",
                           "--http-bind", f"127.0.0.1:{http}", "--signing-algorithm", args.signing])
        tls = private / "tls"
        tls.mkdir()
        command("root-ca", ["openssl", "req", "-new", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256",
            "-nodes", "-x509", "-days", "1", "-subj", "/CN=Connect owned matrix root",
            "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign,cRLSign",
            "-keyout", tls / "root-key.pem", "-out", tls / "root.pem"])
        command("leaf-request", ["openssl", "req", "-new", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256",
            "-nodes", "-subj", "/CN=localhost", "-keyout", tls / "leaf-key.pem", "-out", tls / "leaf.csr"])
        (tls / "leaf.ext").write_text("basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:localhost,IP:127.0.0.1\n")
        command("signed-leaf", ["openssl", "x509", "-req", "-in", tls / "leaf.csr", "-CA", tls / "root.pem",
            "-CAkey", tls / "root-key.pem", "-CAcreateserial", "-days", "1", "-sha256", "-extfile", tls / "leaf.ext", "-out", tls / "leaf.pem"])
        command("leaf-der", ["openssl", "x509", "-in", tls / "leaf.pem", "-outform", "DER", "-out", initial / "certificate.der"])
        command("leaf-private-der", ["openssl", "pkcs8", "-topk8", "-nocrypt", "-in", tls / "leaf-key.pem", "-outform", "DER", "-out", initial / "private-key.der"])
        command("root-der", ["openssl", "x509", "-in", tls / "root.pem", "-outform", "DER", "-out", initial / "browser-root.der"])
        server_config = tomllib.loads((initial / "server.toml").read_text())
        operator = tomllib.loads((initial / "operator.toml").read_text())
        operator["trust_certificate"] = "browser-root.der"
        server_config["limits"]["message_bytes"] = 65536
        server_config.update(webtransport=dict(mount_path="/connect", allowed_origins=[origin], advertised_url=f"https://127.0.0.1:{native}/connect"),
            websocket=dict(bind=f"127.0.0.1:{wss}", mount_path="/connect", allowed_origins=[origin], advertised_url=f"wss://127.0.0.1:{wss}/connect"),
            webrtc=dict(profile="opus-vp8", max_peers=min(32, server_config["limits"]["connections"])),
            https=[dict(bind=f"127.0.0.1:{application}", surface="application", allowed_origins=[origin])])
        (initial / "server.toml").write_bytes(toml_bytes(server_config))
        (initial / "operator.toml").write_bytes(toml_bytes(operator))
        start_server("seed-server", initial / "server.toml", f"127.0.0.1:{http}")
        cli("seed-original", ["request", "--config", initial / "operator.toml", "--envelope"],
            dict(command=dict(type="conversation.create", payload=dict(tenant_id="tnt_default", policy="persistent", initial_participants=[], metadata={})),
                 scope=dict(operation_id="browser-matrix-seed-original")))
        stop_server()
        cli("stopped-backup", ["backup", "--server-config", initial / "server.toml", "--config", initial / "operator.toml", "--output", seed])
        parameters = dict(cell_id=cell_id, candidate_source_revision=revision, candidate_binary_sha256=report["binary_sha256"],
            seed_manifest_sha256=digest(seed / "manifest.json"), browser_origin=origin,
            topology=dict(kind="local-loopback", server_platform="ubuntu24-arm64", client_platform="ubuntu24-arm64",
                          client_emulated=False, native_endpoint=operator["endpoint"], server_name=operator["server_name"]))
        report["materialization"] = materialize(ROOT, seed, instance, parameters, executable, env)
        start_server("materialized-server", instance / "server.toml", f"127.0.0.1:{http}")
        phase = "public-actor-enrollment"
        actors_root = instance / "actors"
        report["actor_enrollment"] = enroll_actors(ROOT, instance, initial / "operator.application.toml", executable,
            actors_root, cli, movement_controllers=True)
        records = json.loads((actors_root / "actors.json").read_text())
        assert set(records) == set(DEVICES + CONTROLLERS)
        exported, bindings = {}, {}
        transport = "webtransport" if args.transport == "chrome-webtransport" else "webrtc"
        for name, actor in records.items():
            proof = cli(name + "-current-read", ["request", "--config", actor["config"], "--envelope"],
                dict(command=dict(type="conversation.list", payload=dict(filter={}, limit=20)), scope={}))
            assert json.loads(proof)["result"].get("Ok") is not None
            exported[name], bindings[name] = browser_actor(Path(actor["config"]).parent, scope=actors_root, algorithm=args.signing)
            if name in CONTROLLERS:
                continue
            config = exported[name]
            result = json.loads(cli(name + "-media-endpoint", ["provision", "--config", initial / "operator.application.toml"],
                dict(type="endpoint", operation_id="browser-media-" + name, identity_id=actor["identity_id"],
                    device=config["device"], transport=transport,
                    address=f"https://127.0.0.1:{native if transport == 'webtransport' else wss}/matrix/" + actor["device_id"],
                    capabilities=["audio", "video"], secret_ref=None)))["result"]["Ok"]
            assert result["verification_state"] == "unverified" and result["transport"] == transport
            verified = json.loads(cli(name + "-verify-media", ["provision", "--config", initial / "operator.application.toml"],
                dict(type="verify_endpoint", operation_id="browser-media-verify-" + name, endpoint_id=result["endpoint_id"],
                    expected_revision=result["revision"], evidence_ref="native-signed-device-proof-" + hashlib.sha256(proof).hexdigest())))["result"]["Ok"]
            assert verified["verification_state"] == "verified"
            config["endpointId"] = result["endpoint_id"]
        report["actor_bindings"] = bindings
        phase = "actual-browser-" + args.stage
        browser_output = private / "browser"
        browser_output.mkdir()
        # Docker cannot create a nested destination beneath the read-only
        # workspace mount. This empty ignored host mountpoint supplies the
        # short private alias required by the Unix audio socket path limit.
        audio_mount = ROOT / ".local/matrix-browser"
        audio_mount.mkdir(mode=0o700, exist_ok=True)
        assert not audio_mount.is_symlink() and not any(audio_mount.iterdir())
        browser_input = browser_output / "input.json"
        publish(browser_input, (json.dumps(dict(format="connect/browser-matrix-admission-input/1", sourceRevision=revision,
            transport=args.transport, signingAlgorithm=args.signing, origin=origin, stage=args.stage,
            endpoints={"webtransport": f"https://127.0.0.1:{native}/connect", "wss-webrtc": f"wss://127.0.0.1:{wss}/connect"},
            actors=exported, output=str(browser_output),
            tls=dict(certificatePem=str(tls / "leaf.pem"), privateKeyPem=str(tls / "leaf-key.pem"), rootDer=str(initial / "browser-root.der"),
                     leafSha256=digest(initial / "certificate.der"), webtransportTrust="certificate-sha256"))) + "\n").encode())
        command_args = ["docker", "run", "--rm", "--cidfile", str(cidfile), "--network", "host", "--read-only",
            "--user", f"{os.getuid()}:{os.getgid()}", "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
            "--shm-size", "512m", "--tmpfs", "/tmp:rw,size=512m,mode=1777",
            "--mount", f"type=bind,source={ROOT},target={ROOT},readonly",
            "--mount", f"type=bind,source={browser_output},target={browser_output}",
            "--mount", f"type=bind,source={browser_output},target=/workspace/.local/matrix-browser",
            "--workdir", str(ROOT), "--env", "HOME=/tmp", "--env", "CONNECT_GATE6_OWNED_BROWSER_CONTAINER=1",
            args.image, "python3", "scripts/video/owned_pulse_audio.py", "--output", "/workspace/.local/matrix-browser/audio",
            "--", "node", "scripts/video/browser_matrix_admission.mjs", str(browser_input)]
        handle = (private / "browser.log").open("xb")
        log_handles.append(handle)
        browser_owner = subprocess.Popen(command_args, cwd=ROOT, env=env, stdin=subprocess.DEVNULL,
                                         stdout=handle, stderr=subprocess.STDOUT, start_new_session=True)
        report["browser_exit_code"] = browser_owner.wait(timeout=240)
        report["browser_owner_joined"] = True
        browser_owner = None
        report["browser_observation"] = json.loads((browser_output / "browser-observation.json").read_text())
        report["browser_observation_sha256"] = digest(browser_output / "browser-observation.json")
        report["owned_audio"] = json.loads((browser_output / "audio/summary.json").read_text())
        report["passed"] = report["browser_exit_code"] == 0 and report["browser_observation"]["passed"] and report["owned_audio"]["passed"]
        if not report["passed"] and report["browser_observation"].get("failure"):
            report["failure"] = dict(phase=phase, browser=report["browser_observation"]["failure"])
    except BaseException as error:
        (private / "failure.log").write_text(traceback.format_exc())
        report["failure"] = dict(phase=phase, name=type(error).__name__, line=traceback.extract_tb(error.__traceback__)[-1].lineno)
    finally:
        if browser_owner is not None and browser_owner.poll() is None:
            if cidfile.exists():
                cid = cidfile.read_text().strip()
                if re.fullmatch("[0-9a-f]{64}", cid):
                    subprocess.run(["docker", "stop", "--time", "5", cid], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15)
            browser_owner.wait(timeout=10)
            report["browser_forced_cleanup"] = True
            report["passed"] = False
        try:
            stop_server()
        except BaseException:
            report["server_cleanup_failed"] = True
            report["passed"] = False
            if server is not None and server.poll() is None:
                os.killpg(server.pid, signal.SIGKILL)
                server.wait()
        for handle in log_handles:
            handle.close()
        try:
            report["source_inputs_unchanged"] = before is not None and sources() == before
        except Exception:
            report["source_inputs_unchanged"] = False
        try:
            report["sdk_dist_inputs_unchanged"] = sdk_before is not None and sdk_inputs() == sdk_before
        except Exception:
            report["sdk_dist_inputs_unchanged"] = False
        try:
            report["executable_unchanged"] = executable is not None and digest(executable) == report["binary_sha256"]
        except Exception:
            report["executable_unchanged"] = False
        report["passed"] &= report["source_inputs_unchanged"] and report["sdk_dist_inputs_unchanged"] and report["executable_unchanged"]
        (output / "component.json").write_text(json.dumps(report, indent=2) + "\n")
        state.update(stage="complete", source_guard_active=False, component_passed=report["passed"])
        save()
    print(json.dumps(dict(browser_admission_passed=report["passed"], cell_qualified=False, task_complete=False)))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
