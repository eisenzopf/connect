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
import uuid

from lib.browser_matrix_actors import browser_actor
from lib.package_environment import fixture_environment
from lib.sealed_executable import seal_executable
from lib.transport_actors import CONTROLLERS, DEVICES, enroll_actors
from lib.transport_fixture import materialize, publish, toml_bytes
from lib.transport_matrix import digest
from lib.transport_matrix_execution import browser_cycle_session_owners_verified
from lib.matrix_browser_capture import BrowserCapture
from lib.matrix_idle import MatrixIdle
from lib.matrix_storage import MetadataReserve, GUARD_RESERVE_BYTES, write_component, atomic_json
from lib.matrix_plateau import POLICY as PLATEAU_POLICY, evaluate as evaluate_plateau, runtime_capacity_configuration
from lib.matrix_authority import MatrixAuthority
from lib.mac_linux_fixture import MacLinuxFixture, MacLinuxIdle, remote_browser_owner_joined

ROOT = Path(__file__).resolve().parents[1]


def record_server_custody(server, receipt):
    """Observe the already waited original group separately from server success."""
    receipt.update(original_pid=server.pid, original_exit_code=server.returncode,
                   original_process_group_joined=False)
    if server.returncode is None:
        return
    try:
        os.killpg(server.pid, 0)
    except ProcessLookupError:
        receipt["original_process_group_joined"] = True
    except OSError as error:
        receipt["process_group_observation_error"] = type(error).__name__


def runtime_spec(args, linux):
    browser = "firefox" if args.transport.startswith("firefox") else "chrome"
    retained = Path.home() / ".codex/worktrees/private-sip-lifetime/Connect/.local/gate-6"
    if linux:
        binary = (str(args.firefox or ROOT / ".local/matrix-browser-runtime/firefox/firefox") if browser == "firefox"
                  else str(args.chrome or "/ms-playwright/chromium-1248/chrome-linux-arm64/chrome"))
        playwright = "/opt/connect-browser-tools/node_modules/playwright/index.mjs"
        puppeteer = str(ROOT / ".local/matrix-browser-runtime/node_modules/puppeteer-core/lib/puppeteer/puppeteer-core.js")
    else:
        binary = (str(args.firefox or retained / "browser-runtimes/Firefox-158.0.1.app/Contents/MacOS/firefox") if browser == "firefox"
                  else str(args.chrome or "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"))
        playwright = str(ROOT / "scripts/video/observer-tools/node_modules/playwright/index.mjs")
        puppeteer = str(retained / "browser-bidi-tools/node_modules/puppeteer-core/lib/puppeteer/puppeteer-core.js")
    contract = json.loads((ROOT / "contracts/gate6-transport-matrix.json").read_text())
    client_platform = "ubuntu24-arm64" if linux else "macos-arm64"
    version = contract["expected_browsers_by_client_platform"][client_platform][browser]
    product = "Mozilla Firefox" if browser == "firefox" else (
        "Google Chrome for Testing" if linux else "Google Chrome")
    expected = product + " " + version
    return dict(browserBinary=binary, playwrightModule=playwright, puppeteerModule=puppeteer, expectedBrowserVersion=expected)


def read_browser_observation(browser_output, exit_code, command, browser_log):
    """A missing result must not replace the already joined owner's failure."""
    path = browser_output / "browser-observation.json"
    try:
        return json.loads(path.read_text())
    except FileNotFoundError:
        if exit_code == 0:
            raise
        error = subprocess.CalledProcessError(exit_code, command)
        try:
            # Private diagnostic only. Bound the tail; do not load an arbitrary
            # original browser log into memory or a public verdict field.
            with browser_log.open("rb") as source:
                source.seek(0, os.SEEK_END)
                source.seek(max(0, source.tell() - 8192))
                tail = source.read(8192).decode("utf8", errors="replace")
            error.add_note("Original joined browser log (last 8192 bytes):\n" + tail)
        except OSError as log_error:
            error.add_note("Original browser log unavailable: " + type(log_error).__name__)
        raise error from None


def port(kind):
    with socket.socket(socket.AF_INET, kind) as reservation:
        reservation.bind(("127.0.0.1", 0))
        return reservation.getsockname()[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stage", choices=("admission", "programme", "quality"), required=True)
    parser.add_argument("--resource-cycles", type=int, choices=(1, 3), default=1)
    parser.add_argument("--transport", choices=("chrome-webtransport", "chrome-wss-webrtc", "firefox-wss-webrtc"), required=True)
    parser.add_argument("--signing", choices=("EdDSA", "ES256"), required=True)
    parser.add_argument("--image", help="Exact already installed local ARM browser image ID")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--network", choices=("local", "remote-profile-fixture"), default="local")
    parser.add_argument("--chrome", type=Path)
    parser.add_argument("--firefox", type=Path)
    parser.add_argument("--rtc-audio-facts", action="store_true",
                        help="Opt in to bounded private WSS audio diagnostics; default is off")
    parser.add_argument("--original-completion-deferral-ms", type=int, choices=(0, 250),
                        help="Diagnostic-only first actual WSS completion delivery barrier; default off")
    args = parser.parse_args()
    if args.original_completion_deferral_ms is not None and not (
            args.stage == "quality" and args.resource_cycles == 1 and args.network == "local"
            and args.transport.endswith("wss-webrtc")):
        parser.error("Original completion counter requires one local WSS quality programme")
    rtc_audio_facts = args.rtc_audio_facts and args.transport.endswith("wss-webrtc")
    if args.resource_cycles == 3 and args.stage != "quality":
        parser.error("Three resource cycles require the full quality workload")
    if os.environ.get("GITHUB_ACTIONS") or os.environ.get("CI"):
        parser.error("Use the owner's local ARM resources")
    linux = platform.system() == "Linux"
    assert (platform.system(), platform.machine()) in {("Linux", "aarch64"), ("Darwin", "arm64")}
    server_platform = "ubuntu24-arm64" if linux else "macos-arm64"
    if linux:
        release = platform.freedesktop_os_release()
        assert release["ID"] == "ubuntu" and release["VERSION_ID"] == "24.04"
        assert os.getuid() != 0
        assert args.image and re.fullmatch(r"sha256:[0-9a-f]{64}", args.image)
        image = json.loads(subprocess.check_output(["docker", "image", "inspect", args.image], text=True))[0]
        assert image["Id"] == args.image and image["Architecture"] == "arm64" and image["Os"] == "linux"
    assert not subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).strip()
    os.umask(0o077)
    output = args.output.resolve()
    assert output.is_relative_to((ROOT / ".local").resolve())
    output.mkdir(parents=True, exist_ok=False)
    private = output / "private"
    private.mkdir()
    component_reserve = MetadataReserve(output / "component-metadata.reserve", GUARD_RESERVE_BYTES)
    env = fixture_environment()
    env["CARGO_PROFILE_RELEASE_DEBUG"] = "0"
    if not linux:
        env.update(MACOSX_DEPLOYMENT_TARGET="15.0", OPUS_STATIC="1", LIBOPUS_NO_PKG="1", CMAKE_POLICY_VERSION_MINIMUM="3.5")
    # These bounded counters are already collected on the media path and emit
    # only when each owned audio member retires. Preserve loss attribution
    # without adding per-packet logging to the measured programme.
    env["RUST_LOG"] = "warn,connect_service::canonical_replacement=debug,rvoip_core::replacement_commit=debug,rvoip_core::audio_summary=debug,connect_service::packet_refusal=debug,connect_service::runtime_retention=debug" + ",quinn_proto::datagram_queue=debug"
    if rtc_audio_facts:
        env["RUST_LOG"] += ",rvoip_webrtc_stack::audio_facts=debug,rvoip_webrtc::audio_facts=debug,rvoip_webrtc::canonical::media=debug"
    sources = runpy.run_path(str(ROOT / "scripts/check-gate6-linux-hosting.py"))["sources"]
    bounded = runpy.run_path(str(ROOT / "scripts/check-linux-package-build.py"))["run_bounded"]
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    cell_id = "/".join(("G6-058", server_platform, args.network, args.transport, args.signing))
    report = dict(format="connect/browser-matrix-component/1", stage=args.stage, cell_id=cell_id,
        source_revision=revision, server_platform=server_platform, client_platform="ubuntu24-arm64" if args.network != "local" else server_platform,
        network_profile=args.network, diagnostic_options=dict(rtc_audio_facts_enabled=rtc_audio_facts,
            original_completion_deferral_ms=args.original_completion_deferral_ms),
        diagnostic_counter_only=args.original_completion_deferral_ms is not None,
        signing_algorithm=args.signing, transport=args.transport, browser_image_id=args.image,
        passed=False, cell_qualified=False, task_complete=False, whole_gate_qualified=False,
        full_media_programme_executed=False, commands=[], servers=[], browser_owner_started=False,
        scope="Actual fresh production CLI/normal TLS/enrolled browser-SDK " + args.stage + " component. Independent decoded media/capture/fallback/expiry/idle remain required.")
    state = dict(stage="preparing", controller_pid=os.getpid(), source_revision=revision,
                 cell_id=cell_id, source_guard_active=False)
    def save():
        atomic_json(output / "state.json", state, reserve=component_reserve)
    save()
    before = sdk_before = executable = server = browser_owner = None
    log_handles = []
    capture = None
    idle = None
    idle_observers, idle_results = [], []
    cycle_count = args.resource_cycles
    cycle_workloads = [dict(index=index, workload_id="browser-cycle-" + uuid.uuid4().hex)
                      for index in range(1, cycle_count + 1)]
    browser_timeout = 240 * cycle_count
    network_name = None
    remote = None
    server_ip = client_ip = "127.0.0.1"
    interface = "lo" if linux else "lo0"
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
        receipt = report["servers"][-1]
        try:
            code = server.wait(timeout=10)
        finally:
            record_server_custody(server, receipt)
        assert code == 0 and receipt["original_process_group_joined"] is True
        receipt["original_joined"] = True
        server = None

    def start_server(label, config, probe):
        nonlocal server
        handle = (private / (label + ".log")).open("xb")
        log_handles.append(handle)
        server = subprocess.Popen([str(executable), "serve", "--config", str(config)], cwd=ROOT, env=env,
                                  stdin=subprocess.DEVNULL, stdout=handle, stderr=subprocess.STDOUT, start_new_session=True)
        running = digest(Path(f"/proc/{server.pid}/exe")) if linux else digest(executable)
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
        if args.network != "local":
            phase = "owned-remote-browser-topology"
            if not linux:
                remote = MacLinuxFixture(ROOT, private / "remote-owner", env, resource_cycles=cycle_count)
                report["actual_topology"] = remote.start(revision, image=args.image)
                args.image = remote.image
                report["browser_image_id"] = remote.image
                server_ip, client_ip, interface = remote.server_ip, remote.client_ip, "lo0"
            else:
                network_name = "connect-g658-" + uuid.uuid4().hex[:12]
                network_id = command("client-network", ["docker", "network", "create", "--internal",
                    "--subnet", "10.203.0.0/29", "--gateway", "10.203.0.1", network_name]).decode().strip()
                assert re.fullmatch(r"[0-9a-f]{64}", network_id)
                server_ip, client_ip, interface = "10.203.0.1", "10.203.0.2", "br-" + network_id[:12]
                network = json.loads(command("inspect-client-network", ["docker", "network", "inspect", network_name]))[0]
                assert network["Id"] == network_id and network["Internal"] is True
                report["actual_topology"] = dict(kind="ubuntu-netns", internal_docker_network=True,
                    client_platform="ubuntu24-arm64", server_platform=server_platform, client_emulated=False)
        else:
            report["actual_topology"] = dict(kind="local-loopback", server_platform=server_platform,
                client_platform=server_platform, client_emulated=False)
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
        origin = f"https://{client_ip}:{asset}"
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
        (tls / "leaf.ext").write_text("basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:localhost,IP:127.0.0.1,IP:" + server_ip + ",IP:" + client_ip + "\n")
        command("signed-leaf", ["openssl", "x509", "-req", "-in", tls / "leaf.csr", "-CA", tls / "root.pem",
            "-CAkey", tls / "root-key.pem", "-CAcreateserial", "-days", "1", "-sha256", "-extfile", tls / "leaf.ext", "-out", tls / "leaf.pem"])
        command("leaf-der", ["openssl", "x509", "-in", tls / "leaf.pem", "-outform", "DER", "-out", initial / "certificate.der"])
        command("leaf-private-der", ["openssl", "pkcs8", "-topk8", "-nocrypt", "-in", tls / "leaf-key.pem", "-outform", "DER", "-out", initial / "private-key.der"])
        command("root-der", ["openssl", "x509", "-in", tls / "root.pem", "-outform", "DER", "-out", initial / "browser-root.der"])
        server_config = tomllib.loads((initial / "server.toml").read_text())
        operator = tomllib.loads((initial / "operator.toml").read_text())
        operator["trust_certificate"] = "browser-root.der"
        server_config["limits"]["message_bytes"] = 65536
        media_start = 40000
        server_config["bind"] = f"{server_ip}:{native}"
        operator["endpoint"] = f"{server_ip}:{native}"
        if args.network != "local":
            server_config["network_profile"] = operator["network_profile"] = "remote"
        server_config.update(webtransport=dict(mount_path="/connect", allowed_origins=[origin], advertised_url=f"https://{server_ip}:{native}/connect"),
            websocket=dict(bind=f"{server_ip}:{wss}", mount_path="/connect", allowed_origins=[origin], advertised_url=f"wss://{server_ip}:{wss}/connect"),
            webrtc=dict(profile="opus-vp8", max_peers=min(32, server_config["limits"]["connections"]),
                addressing=dict(bind_ip=server_ip, port_start=media_start, port_end=media_start+63, ice_policy="all", ice_services=[])),
            https=[dict(bind=f"{server_ip}:{application}", surface="application",
                        public_origin=f"https://{server_ip}:{application}", allowed_origins=[origin])])
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
            topology=dict(kind=report["actual_topology"]["kind"], server_platform=server_platform, client_platform=report["client_platform"],
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
                    address=f"https://{server_ip}:{native if transport == 'webtransport' else wss}/matrix/" + actor["device_id"],
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
        authorities = []
        for cycle in cycle_workloads:
            folder = browser_output if cycle_count == 1 else browser_output / "cycles" / ("cycle-" + str(cycle["index"]))
            folder.mkdir(mode=0o700, parents=True, exist_ok=True)
            authority_actors = actors_root if cycle_count == 1 else actors_root / ("cycle-authority-" + str(cycle["index"]))
            authority_actors.mkdir(mode=0o700, exist_ok=True)
            authorities.append(MatrixAuthority(folder, authority_actors, records, instance,
                initial / "operator.application.toml", cli, args.signing))
        # Docker cannot create a nested destination beneath the read-only
        # workspace mount. This empty ignored host mountpoint supplies the
        # short private alias required by the Unix audio socket path limit.
        audio_mount = ROOT / ".local/matrix-browser"
        audio_mount.mkdir(mode=0o700, exist_ok=True)
        assert not audio_mount.is_symlink() and not any(audio_mount.iterdir())
        browser_input = browser_output / "input.json"
        client_runtime = runtime_spec(args, linux or remote is not None)
        if remote is not None:
            for name in ("browserBinary", "puppeteerModule"):
                client_runtime[name] = client_runtime[name].replace(str(ROOT), "/workspace")
        publish(browser_input, (json.dumps(dict(format="connect/browser-matrix-admission-input/1", sourceRevision=revision,
            transport=args.transport, signingAlgorithm=args.signing, origin=origin, stage=args.stage, resourceCycles=cycle_count, cycleWorkloads=cycle_workloads,
            originalCompletionDeferral=(dict(holdMs=args.original_completion_deferral_ms, source=0, ordinal=0)
                if args.original_completion_deferral_ms is not None else None),
            rtcAudioFacts=rtc_audio_facts, fixtureAddresses=sorted({server_ip, client_ip}),
            endpoints={"webtransport": f"https://{server_ip}:{native}/connect", "wss-webrtc": f"wss://{server_ip}:{wss}/connect"},
            actors=exported, output=str(browser_output),
            **client_runtime,
            tls=dict(certificatePem=str(tls / "leaf.pem"), privateKeyPem=str(tls / "leaf-key.pem"), rootDer=str(initial / "browser-root.der"),
                     leafSha256=digest(initial / "certificate.der"), webtransportTrust="certificate-sha256"))) + "\n").encode())
        command_args = ["docker", "run", "--rm", "--cidfile", str(cidfile), "--network", network_name or "host", "--read-only",
            "--user", f"{os.getuid()}:{os.getgid()}", "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
            "--shm-size", "512m", "--tmpfs", "/tmp:rw,size=512m,mode=1777",
            "--mount", f"type=bind,source={ROOT},target={ROOT},readonly",
            "--mount", f"type=bind,source={browser_output},target={browser_output}",
            "--mount", f"type=bind,source={browser_output},target=/workspace/.local/matrix-browser",
            "--workdir", str(ROOT), "--env", "HOME=/tmp", "--env", "CONNECT_GATE6_OWNED_BROWSER_CONTAINER=1",
            args.image, "python3", "scripts/video/owned_pulse_audio.py", "--output", "/workspace/.local/matrix-browser/audio",
            "--", "node", "scripts/video/browser_matrix_admission.mjs", str(browser_input)]
        if network_name:
            command_args[command_args.index(args.image):command_args.index(args.image)] = ["--ip", client_ip]
        if remote is not None:
            remote.stage_paths([ROOT / "sdk/typescript/dist", tls / "leaf.pem", tls / "leaf-key.pem", initial / "browser-root.der"])
            remote.stage_json(browser_input, json.loads(browser_input.read_text()))
            remote.call(["mkdir", "-p", str(remote.remote_root / ".local/matrix-browser")])
            guest_output, guest_input = remote.mapped(browser_output), remote.mapped(browser_input)
            container = "connect-g658-" + remote.run_id
            command_args = ["docker", "run", "--rm", "--name", container, "--label", "connect.gate6.owner=" + remote.run_id,
                "--network", "host", "--read-only", "--user", f"{remote.uid}:{remote.gid}",
                "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--shm-size", "512m", "--tmpfs", "/tmp:rw,size=512m,mode=1777",
                "--mount", "type=bind,source=/workspace,target=/workspace,readonly",
                "--mount", f"type=bind,source={remote.remote_root},target={remote.remote_root},readonly",
                "--mount", f"type=bind,source={guest_output},target={guest_output}",
                "--mount", f"type=bind,source={guest_output},target={remote.remote_root}/.local/matrix-browser",
                "--workdir", str(remote.remote_root), "--env", "HOME=/tmp", "--env", "CONNECT_GATE6_OWNED_BROWSER_CONTAINER=1",
                remote.image, "python3", "scripts/video/owned_pulse_audio.py", "--output", str(remote.remote_root / ".local/matrix-browser/audio"),
                "--", "node", "scripts/video/browser_matrix_admission.mjs", str(guest_input)]
        elif not linux:
            command_args = ["node", "scripts/video/browser_matrix_admission.mjs", str(browser_input)]
        handle = (private / "browser.log").open("xb")
        log_handles.append(handle)
        if args.stage == "quality" and remote is None:
            capture = BrowserCapture(private / "packet-capture", interface=interface, server_ip=server_ip,
                native=native, wss=wss, media_start=media_start, transport=args.transport, env=env, resource_cycles=cycle_count)
            try:
                capture.start()
            except (OSError, RuntimeError):
                report["packet_capture"] = capture.finish()
                capture = None
        browser_owner = (remote.launch(command_args, browser_output, private / "guest-supervisor.log", timeout=browser_timeout, resource_cycles=cycle_count, container=container,
                            capture=dict(kind="browser", interface=remote.topology["guest"]["route"]["dev"], server_ip=server_ip, client_ip=client_ip,
                                         native=native, wss=wss, media_start=media_start, transport=args.transport, resource_cycles=cycle_count) if args.stage == "quality" else None)
                         if remote is not None else subprocess.Popen(command_args, cwd=ROOT, env=env, stdin=subprocess.DEVNULL,
                                         stdout=handle, stderr=subprocess.STDOUT, start_new_session=True))
        report["browser_owner_started"] = True
        browser_cutoff = time.monotonic() + browser_timeout
        while browser_owner.poll() is None:
            if remote is not None:
                remote.poll()
            for authority in authorities:
                authority.poll()
            if args.stage == "quality":
                next_cycle = len(idle_results) + 1
                idle_folder = browser_output if cycle_count == 1 else browser_output / "cycles" / ("cycle-" + str(next_cycle))
                if idle is None and next_cycle <= cycle_count and (idle_folder / "resources-baseline.json").exists() and (remote is None or (remote.latest.get("state") or {}).get("resources")):
                    if linux:
                        cid = cidfile.read_text().strip()
                        assert re.fullmatch("[0-9a-f]{64}", cid)
                        container = json.loads(subprocess.check_output(["docker", "inspect", cid], text=True, timeout=3))[0]
                        assert container["Id"] == cid and container["State"]["Running"]
                        browser_pid = container["State"]["Pid"]
                    else:
                        browser_pid = browser_owner.pid
                    credential = tomllib.loads((initial / "operator.application.toml").read_text())["credential"]
                    original_baseline = idle_results[0]["baseline"] if idle_results else None
                    shared_sampler = idle_observers[0].sampler if idle_observers else None
                    idle = (MacLinuxIdle(idle_folder, server.pid, remote, f"http://127.0.0.1:{http}/v1/operations", credential,
                                cycle_mode=cycle_count == 3, original_baseline=original_baseline,
                                sampler=shared_sampler, resource_cycles=cycle_count)
                            if remote is not None else MatrixIdle(idle_folder, server.pid, browser_pid,
                                f"http://127.0.0.1:{http}/v1/operations", credential,
                                cycle_mode=cycle_count == 3, original_baseline=original_baseline,
                                sampler=shared_sampler, resource_cycles=cycle_count))
                    idle_observers.append(idle)
                if idle is not None:
                    idle.poll()
                    if idle.done:
                        idle_results.append(idle.finish())
                        idle = None
            if time.monotonic() >= browser_cutoff:
                raise subprocess.TimeoutExpired(command_args, browser_timeout)
            time.sleep(.05)
        report["browser_exit_code"] = browser_owner.wait(timeout=1)
        report["browser_owner_joined"] = True
        if remote is not None:
            report["remote_client_owner"] = remote.close()
            report["browser_owner_joined"] = remote_browser_owner_joined(report["remote_client_owner"])
        browser_owner = None
        report["browser_observation"] = read_browser_observation(
            browser_output, report["browser_exit_code"], command_args,
            browser_output / "guest-job.log" if remote is not None else private / "browser.log")
        if remote is not None and args.stage == "quality":
            report["packet_capture"] = json.loads((browser_output / "browser-capture-observation.json").read_text())
        report["browser_observation_sha256"] = digest(browser_output / "browser-observation.json")
        report["runtime_versions"] = dict(passed=report["browser_observation"].get("browser") == client_runtime["expectedBrowserVersion"],
            browser=report["browser_observation"].get("browser"), node=report["browser_observation"].get("client_runtime", {}).get("node"),
            rust=subprocess.check_output(["rustc", "--version"], text=True).strip(), server_binary_sha256=report["binary_sha256"],
            client_platform=report["browser_observation"].get("client_runtime", {}).get("platform"), server_platform=server_platform)
        report["runtime_versions"]["passed"] &= report["runtime_versions"]["client_platform"] == report["client_platform"]
        report["owned_audio"] = json.loads((browser_output / "audio/summary.json").read_text()) if linux or remote is not None else dict(
            passed=True, scope="Native macOS AudioContext owned by the browser renderers; no PulseAudio process", original_process_joined=True)
        report["passed"] = report["browser_exit_code"] == 0 and report["browser_observation"]["passed"] and report["owned_audio"]["passed"] and report["browser_owner_joined"]
        if not report["passed"] and report["browser_observation"].get("failure"):
            report["failure"] = dict(phase=phase, browser=report["browser_observation"]["failure"])
    except BaseException as error:
        (private / "failure.log").write_text(traceback.format_exc())
        report["failure"] = dict(phase=phase, name=type(error).__name__, line=traceback.extract_tb(error.__traceback__)[-1].lineno)
    finally:
        if remote is not None:
            try:
                report["remote_client_owner"] = remote.close()
                report["browser_owner_joined"] = remote_browser_owner_joined(report["remote_client_owner"])
                report["passed"] &= report["remote_client_owner"]["passed"]
            except Exception:
                report["remote_client_owner"] = dict(passed=False, failure="original-guest-cleanup-unconfirmed")
                report["browser_owner_joined"] = False
                report["passed"] = False
        if remote is None and browser_owner is not None:
            if browser_owner.poll() is None:
                if cidfile.exists():
                    cid = cidfile.read_text().strip()
                    if re.fullmatch("[0-9a-f]{64}", cid):
                        subprocess.run(["docker", "stop", "--time", "5", cid], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15)
                report["browser_forced_cleanup"] = True
                report["passed"] = False
            report["browser_exit_code"] = browser_owner.wait(timeout=10)
            # A failed programme can still return its original process custody.
            # Record the real join even when it exited before this cleanup ran.
            report["browser_owner_joined"] = True
        # The traffic owner is joined above. Keep its interface alive until
        # tcpdump flushes and joins; removing the bridge first aborts capture.
        if capture is not None:
            report["packet_capture"] = capture.finish()
            report["packet_capture"]["transport"] = args.transport
            report["passed"] &= report["packet_capture"]["observed"]
        if network_name is not None:
            cleanup = subprocess.run(["docker", "network", "rm", network_name], capture_output=True, timeout=15)
            report["client_network_removed"] = cleanup.returncode == 0
        if args.stage == "quality" and cycle_count == 3:
            if idle is not None:
                idle_results.append(idle.finish())
            cycles = []
            report["cycle_observations"] = []
            archives = report.get("browser_observation", {}).get("resource_cycles", [])
            for cycle, observed_idle, archive in zip(cycle_workloads, idle_results, archives):
                folder = browser_output / "cycles" / ("cycle-" + str(cycle["index"]))
                try:
                    if archive["index"] != cycle["index"] or archive["workload_id"] != cycle["workload_id"]:
                        raise ValueError("Original cycle archive binding changed")
                    for name, key in (("cycle-observation.json", "observation_sha256"),
                                      ("workload.json", "workload_evidence_sha256"),
                                      ("original-joins.json", "join_evidence_sha256")):
                        if digest(folder / name) != archive[key]:
                            raise ValueError("Original cycle evidence digest changed")
                    observed = json.loads((folder / "cycle-observation.json").read_text())
                    original_joins = json.loads((folder / "original-joins.json").read_text())
                    if observed.get("cycle") != cycle or original_joins.get("cycle") != cycle:
                        raise ValueError("Original logical cycle identity changed")
                    report["cycle_observations"].append(observed)
                    retirement = observed.get("retirement", {})
                    local = observed.get("idle_resources", {})
                    # Session ownership is distinct from six required functional
                    # Connection.end receipts. The final verifier retains both.
                    session_joined = browser_cycle_session_owners_verified(observed)
                    helper_joined = (local.get("localOwnersJoined") is True
                        and local.get("renderer", {}).get("joined") is True
                        and local.get("diagnostic_hooks", {}).get("joined") is True
                        and local.get("fixture_reference_retirement", {}).get("released") is True
                        and observed.get("diagnostic_cycle_retirement", {}).get("released") is True)
                    join_evidence = dict(cycle=cycle, persistent_owners=observed_idle.get("persistent_owners"),
                                         original_join_receipt=original_joins)
                    join_path = folder / "cycle-join-evidence.json"
                    publish(join_path, (json.dumps(join_evidence, sort_keys=True) + "\n").encode())
                    cycles.append(dict(**cycle, workload_started=observed.get("workload_started") is True,
                        workload_complete=observed.get("workload_complete") is True
                            and observed.get("programme", {}).get("source_programme", {}).get("durationMs") == 62000
                            and 62000 <= observed.get("programme", {}).get("finished_ms", -1) < 67000,
                        workload_evidence_sha256=archive["workload_evidence_sha256"],
                        persistent_owners=observed_idle.get("persistent_owners"), idle_resources=observed_idle,
                        joins=dict(session_owners_joined=session_joined, helper_work_joined=helper_joined,
                                   functional_connection_retirement_passed=retirement.get("passed") is True,
                                   forced=False, evidence_sha256=digest(join_path))))
                except (OSError, ValueError, KeyError, TypeError) as error:
                    report.setdefault("cycle_evidence_failures", []).append(dict(index=cycle["index"], name=type(error).__name__))
            original = idle_results[0] if idle_results else {}
            plateau = dict(format="connect/matrix-plateau-observation/1", policy=PLATEAU_POLICY,
                baseline=original.get("baseline"), persistent_owners=original.get("persistent_owners"), cycles=cycles,
                original_cycle_ids=cycle_workloads,
                server_runtime_capacity=runtime_capacity_configuration((original.get("persistent_owners") or {}).get("server")))
            plateau["verdict"] = evaluate_plateau(plateau)
            plateau["passed"] = plateau["verdict"]["passed"]
            report["idle_resources"] = plateau
            if "browser_output" in locals():
                publish(browser_output / "resource-plateau.json", (json.dumps(plateau, indent=2) + "\n").encode())
            report["passed"] &= plateau["passed"]
            report["passed"] &= len(report["cycle_observations"]) == 3 and all(c.get("passed") is True for c in report["cycle_observations"])
        elif args.stage == "quality":
            if idle is not None:
                idle_results.append(idle.finish())
            report["idle_resources"] = idle_results[0] if idle_results else dict(passed=False, failure="idle-observer-not-started")
            report["passed"] &= report["idle_resources"]["passed"]
        if args.stage == "quality":
            report["passed"] &= report.get("packet_capture", {}).get("observed") is True
        try:
            stop_server()
        except BaseException:
            report["server_cleanup_failed"] = True
            report["passed"] = False
            if server is not None:
                receipt = report["servers"][-1]
                if server.poll() is None:
                    receipt["forced_cleanup"] = True
                    os.killpg(server.pid, signal.SIGKILL)
                    server.wait()
                record_server_custody(server, receipt)
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
        write_component(output / "component.json", report, reserve=component_reserve)
        state.update(stage="complete", source_guard_active=False, component_passed=report["passed"])
        save()
    print(json.dumps(dict(browser_admission_passed=report["passed"], cell_qualified=False, task_complete=False)))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
