#!/usr/bin/env python3
"""Native Ubuntu x86 configuration/HTTPS/RTCP checks; no account/live lanes.

Raw compiler/test output and synthetic state stay private. Only source hashes,
tool versions, exit/result counts and artifact hashes enter uploadable evidence.
This is not installation, remote media or whole Gate 6 qualification.
"""
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
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / ".local/gate6-linux-ci"
EVIDENCE = BASE / "evidence"
PRIVATE = BASE / "private-logs"
sys.path.insert(0, str(ROOT / "scripts/lib"))
from source_build_cache import MANIFEST, snapshot


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def output(name, value):
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a") as stream:
            stream.write(f"{name}={value}\n")


def sources():
    files = {name: {k: v for k, v in value.items() if k != "mtime_ns"} for name, value in snapshot(ROOT).items()}
    for path in sorted((ROOT / ".deps").rglob("*")):
        if ".git" in path.parts or path.is_symlink() or not path.is_file():
            continue
        files[str(path.relative_to(ROOT))] = {"sha256": digest(path), "mode": path.stat().st_mode & 0o777}
    return files


def preflight():
    assert platform.system() == "Linux" and platform.machine() == "x86_64", "Native Linux x86_64 profile required"
    cpuinfo = Path("/proc/cpuinfo").read_text()
    assert re.search(r"^vendor_id\s*:", cpuinfo, re.M), "An x86 kernel is required; an emulated amd64 userland on an ARM kernel is insufficient"
    release = platform.freedesktop_os_release()
    assert release.get("ID") == "ubuntu" and release.get("VERSION_ID") == "24.04", "Ubuntu24.04 profile required"
    rust = subprocess.check_output(["rustc", "-Vv"], text=True)
    assert "rustc 1.97.1 " in rust and "host: x86_64-unknown-linux-gnu" in rust
    native = {
        "rust": rust,
        "clang": subprocess.check_output(["clang", "--version"], text=True),
        "packages": subprocess.check_output(["dpkg-query", "-W", "libopus-dev", "libssl-dev", "libc6-dev", "clang", "g++", "binutils", "pkg-config", "cmake"], text=True),
    }
    key = hashlib.sha256(json.dumps(native, sort_keys=True).encode()).hexdigest()
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    (EVIDENCE / "preflight.json").write_text(json.dumps({"os": release, "platform": platform.platform(),
        "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "runner_image_version": os.environ.get("ImageVersion"), "native_key": key, "toolchain": native,
        "scope": "Native Ubuntu24.04x86 CI fixtures only. No named remote/provider, model account, media/performance, installation or full acceptance qualification."}, indent=2) + "\n")
    output("native_key", key)


def run(node_sdk=False):
    preflight()
    PRIVATE.mkdir(parents=True, exist_ok=True, mode=0o700)
    temporary = BASE / "private-tmp"
    temporary.mkdir(exist_ok=True, mode=0o700)
    os.environ["TMPDIR"] = str(temporary)
    node_package = None
    if node_sdk:
        sys.path.insert(0, str(ROOT / 'sdk/tests'))
        from prepare_native_consumer import prepare
        node_package = prepare(PRIVATE / 'node-native', ROOT / 'dist/gate-6/node-linux-runtime')
        os.environ['CONNECT_GATE6_NODE_CONSUMER'] = str(ROOT / node_package['consumer'])
        os.environ['CONNECT_GATE6_NODE_CONSULT_CONSUMER'] = str(ROOT / node_package['consultation_consumer'])
        os.environ['CONNECT_GATE6_NODE_MOVEMENT_CONSUMER'] = str(ROOT / node_package['movement_consumer'])
    before = sources()
    (EVIDENCE / "source-inputs.json").write_text(json.dumps(before, indent=2) + "\n")
    report = {"passed": False, "started_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "steps": [], "runner_sha256": digest(__file__), "source_count": len(before),
              "source_inputs_sha256": digest(EVIDENCE / "source-inputs.json"),
              "scope": "Current snapshot and peer-authority Store regressions, bounded configuration/security, ICE/DTLS owned-close, negotiated RTCP UDP/adjacent allocator, local synthetic SIP readiness/startup/drain and actual HTTPS surface/CLI fixtures. No actual TURN allocation, remote SIP/NAT, browser/performance or installation qualification."}

    def execute(name, command, timeout, *, defer_failure=False, environment=None):
        log = PRIVATE / (name + ".log")
        started = time.monotonic()
        with log.open("wb") as stream:
            child = subprocess.Popen(command, cwd=ROOT, stdout=stream, stderr=subprocess.STDOUT, start_new_session=True,
                                     env=None if environment is None else dict(os.environ, **environment))
            timed = False
            forced = False
            try:
                child.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                timed = True
                os.killpg(child.pid, signal.SIGTERM)
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    forced = True
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait(timeout=10)
        text = log.read_text(errors="replace")
        summary = [line for line in text.splitlines() if line.startswith("test result:")]
        step = {"name": name, "exit_code": child.returncode, "timed_out": timed, "forced_kill": forced,
                "seconds": time.monotonic() - started, "log_sha256": digest(log), "test_results": summary}
        step["panic_locations"] = [
            {"file": location[0], "line": int(location[1]), "column": int(location[2])}
            for location in re.findall(r"thread '[^']+' panicked at ([^\n]+?):([0-9]+):([0-9]+):", text, re.M)
        ][:16]
        step["node_completed_phases"] = []
        for line in text.splitlines():
            if line.startswith('installed-typed-node-sdk-phase '):
                phase = json.loads(line.split(' ', 1)[1])
                assert set(phase) == {'algorithm','phase','passed','seconds','deadline_seconds'}
                assert phase['algorithm'] in {'EdDSA','ES256'} and phase['passed'] is True
                assert phase['phase'] in {'controls-ready','native-complete','initial','recovered','receipts-observed','completed'}
                assert type(phase['seconds']) in (int,float) and 0 <= phase['seconds'] <= 90 and phase['deadline_seconds'] == 15
                step['node_completed_phases'].append(phase)
        step["node_failures"] = []
        for line in text.splitlines():
            if line.startswith('installed-typed-node-sdk-failure '):
                failure = json.loads(line.split(' ', 1)[1])
                assert set(failure) == {'phase', 'checkpoint', 'kind', 'certainty', 'command', 'observedState', 'line', 'column'}
                assert failure['checkpoint'] in {'create', 'capabilities', 'session', 'native-offer', 'peer-offer',
                    'session-finish', 'native-movement-refusals', 'native-quality', 'native-gates', 'native-codecs', 'native-selection', 'native-end', 'native-recovery', 'native-end-http-recovery', 'native-end-status', 'native-end-wait', 'session-end-wait', 'session-end-refresh', 'message-receipts', 'current-revocation'}
                assert re.fullmatch(r'[a-z-]{1,64}', failure['kind'])
                assert failure['certainty'] is None or re.fullmatch(r'[a-z-]{1,32}', failure['certainty'])
                assert all(failure[k] is None or type(failure[k]) is int and 0 < failure[k] < 100000 for k in ('line', 'column'))
                assert failure['command'] is None or failure['command'] in {'conversation.create','message.send','operation.cancel','operation.reconcile','session.invite','session.accept','session.update','session.end','session.reject','session.cancel','session.handoff','connection.replace','connection.offer','connection.update','connection.end','stream.subscribe','stream.unsubscribe','operation.get','conversation.get','message.history','conversation.participant.add','conversation.participant.update','conversation.participant.remove'}
                assert failure['observedState'] is None or failure['observedState'] in {'accepted','preparing','ready','committing','completed','cancelled','failed','interrupted','unknown'}
                step['node_failures'].append(failure)
        step['node_consultation_phases'] = []
        step['node_consultation_failures'] = []
        for line in text.splitlines():
            if line.startswith('installed-node-consultation-phase '):
                phase = json.loads(line.split(' ', 1)[1])
                assert set(phase) == {'algorithm', 'phase', 'seconds', 'deadline_seconds'}
                assert phase['algorithm'] in {'EdDSA', 'ES256'}
                assert phase['phase'] in {'submitted', 'completed', 'returned', 'revoked', 'closed'}
                assert type(phase['seconds']) in (int, float) and 0 <= phase['seconds'] <= 90 and phase['deadline_seconds'] == 15
                step['node_consultation_phases'].append(phase)
            if line.startswith('installed-node-consultation-failure '):
                failure = json.loads(line.split(' ', 1)[1])
                assert set(failure) == {'phase', 'checkpoint', 'kind', 'certainty'} and failure['phase'] == 'failed'
                assert failure['checkpoint'] in {'submit', 'completed', 'revoked', 'close', 'invalid'}
                assert re.fullmatch(r'[a-z-]{1,64}', failure['kind'])
                assert failure['certainty'] is None or re.fullmatch(r'[a-z-]{1,32}', failure['certainty'])
                step['node_consultation_failures'].append(failure)
        step['node_movement_phases'] = []
        step['node_movement_failures'] = []
        for line in text.splitlines():
            if line.startswith('installed-node-movement-phase '):
                phase = json.loads(line.split(' ', 1)[1])
                assert set(phase) == {'algorithm', 'phase', 'seconds', 'deadline_seconds'}
                assert phase['algorithm'] in {'EdDSA', 'ES256'}
                assert phase['phase'] in {'registered', 'accepted', 'completed', 'stopped', 'closed'}
                assert type(phase['seconds']) in (int, float) and 0 <= phase['seconds'] <= 90 and phase['deadline_seconds'] == 15
                step['node_movement_phases'].append(phase)
            if line.startswith('installed-node-movement-failure '):
                failure = json.loads(line.split(' ', 1)[1])
                assert set(failure) == {'phase', 'checkpoint', 'kind', 'certainty', 'line', 'column'} and failure['phase'] == 'failed'
                assert failure['checkpoint'] in {'register', 'recover', 'begin', 'completed', 'stopped', 'close', 'invalid'}
                assert re.fullmatch(r'[a-z-]{1,64}', failure['kind'])
                assert failure['certainty'] is None or re.fullmatch(r'[a-z-]{1,32}', failure['certainty'])
                assert all(failure[k] is None or type(failure[k]) is int and 0 < failure[k] < 100000 for k in ('line', 'column'))
                step['node_movement_failures'].append(failure)
        if child.returncode != 0:
            diagnostics = []
            for line in text.splitlines():
                if not line.startswith("{"):
                    continue
                try:
                    item = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if item.get("reason") == "compiler-message" and item["message"]["level"] == "error":
                    message = item["message"]
                    diagnostics.append({"code": (message.get("code") or {}).get("code"), "message": message["message"],
                                        "locations": [{k: span[k] for k in ("file_name", "line_start", "column_start")} for span in message["spans"]]})
            step["compiler_diagnostics"] = diagnostics
        report["steps"].append(step)
        print(json.dumps(step), flush=True)
        if defer_failure and (child.returncode != 0 or timed or forced):
            return None
        assert child.returncode == 0 and not timed and not forced, f"Phase failed: {name}; raw output retained privately"
        return text

    artifacts = {}
    try:
        for name, wanted, arguments in (
            ("store", "connect_store", ["--offline", "-p", "connect-store", "--lib"]),
            ("library", "connect_server", ["--offline", "-p", "connect-server", "--features", "test-support", "--lib"]),
            ("hosting", "gate6_canonical_runtime", ["--offline", "-p", "connect-server", "--features", "test-support", "--test", "gate6_canonical_runtime"]),
            # Targeted locked upstream compilation may fetch its registry-only
            # test dependencies; it does not execute providers or remote lanes.
            ("rtp", "rvoip_rtp_core", ["--manifest-path", ".deps/rvoip/Cargo.toml", "-p", "rvoip-rtp-core", "--lib"]),
        ):
            text = execute("compile-" + name, ["cargo", "test", "--locked", "--jobs", "2",
                           *arguments, "--no-run", "--message-format=json"], 3600)
            rows = [json.loads(line) for line in text.splitlines() if line.startswith("{")]
            assert any(row.get("reason") == "build-finished" and row.get("success") for row in rows)
            executable = [row["executable"] for row in rows if row.get("reason") == "compiler-artifact"
                          and row["target"]["name"] == wanted and row["profile"]["test"] and row.get("executable")]
            assert len(executable) == 1
            artifacts[name] = executable[0]
            if node_sdk and name == 'hosting':
                cli = {row['executable'] for row in rows if row.get('reason') == 'compiler-artifact'
                       and row['target']['name'] == 'connect' and row.get('executable')
                       and not row['profile']['test']}
                assert len(cli) == 1, 'Missing exact compiled CLI for installed Node fixture'
                cli = Path(cli.pop())
                assert cli.stat().st_size > 1_000_000 and os.access(cli, os.X_OK)
                with cli.open('rb') as stream:
                    assert stream.read(4) == b'\x7fELF', 'Native Linux CLI header required'
                execute('node-cli-preflight', [str(cli), 'sdk-stdio', '--help'], 10)
                artifacts['cli'] = str(cli)
        assert sources() == before, "Sources changed during compilation"
        # A failed compile must never stamp source times as a completed build.
        MANIFEST.write_text(json.dumps({"format": 1, "files": snapshot(ROOT)}, sort_keys=True) + "\n")
        report["compiled_artifacts"] = {name: digest(path) for name, path in artifacts.items()}
        inventory = execute("snapshot-test-inventory", [artifacts['store'], '--list'], 10)
        for case in (
            'single_credential_snapshot_checks_current_projection_without_duplicate_source_captures',
            'multi_credential_snapshot_keeps_each_credentials_projection_revalidation',
            'snapshot_source_shortcut_cannot_replace_projection_coverage_or_current_key_checks',
        ):
            assert f'peer_authority::tests::{case}: test' in inventory.splitlines()
        for name, binary, pattern, expected in (
            ("snapshot-store", "store", "events::tests::", 1),
            ("snapshot-peer-authority", "store", "peer_authority::tests::", 3),
            ("rust-sdk-consultation-audio", "hosting", "canonical_native_connections::consultation::sdk_controls::production_public_rust_sdk_consultation_return_retains_actual_private_and_restored_audio", 1),
            ("rust-sdk-consultation-eof", "hosting", "canonical_native_connections::consultation::sdk_controls::production_public_rust_sdk_consultation_return_after_remote_eof_reports_no_continuing_call", 1),
            ("addressing", "library", "canonical_webrtc::addressing::tests::", 4),
            ("config", "library", "canonical_webrtc::config::tests::", 4),
            ("sip-config", "library", "canonical_telephone::tests::", 4),
            ("rtcp-udp", "rtp", "transport::udp::tests::", 32),
            ("rtcp-allocator", "rtp", "transport::allocator::tests::", 20),
            ("rtcp-session-route", "rtp", "session::tests::negotiated_rtcp_manual_reports_and_close_use_original_route", 1),
            ("owned-dtls", "library", "canonical_webrtc::tests::store_bound_peer_completes_actual_dtls_and_original_close_releases_capacity", 1),
            ("https-hosting", "hosting", "canonical_https_hosting::", 7),
            ("sip-unknown-account", "hosting", "canonical_native_connections::consultation::groups::replacement::telephone::configured::production_telephone_configured_unknown_account_refuses_before_binding_and_releases_installation", 1),
            ("sip-bind-failure", "hosting", "canonical_native_connections::consultation::groups::replacement::telephone::configured::production_telephone_configured_later_http_bind_failure_drains_started_sip", 1),
            ("sip-owner-drop", "hosting", "canonical_native_connections::consultation::groups::replacement::telephone::configured::production_telephone_configured_server_drop_fences_owner_and_drains_sip", 1),
            ("sip-readiness", "hosting", "canonical_native_connections::consultation::groups::replacement::telephone::initial::activation::production_mixed_session_activation_waits_for_actual_native_and_sip_readiness_with_exact_capabilities", 1),
        ):
            text = execute(name, [artifacts[binary], pattern, "--test-threads=1"], 90)
            counts = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;", text)
            assert len(counts) == 1 and int(counts[0][0]) >= expected and counts[0][1:] == ("0", "0"), "Missing/skipped fixture coverage"
        sdk_failures = []
        def sdk_case(name, case, environment=None):
            text = execute(name, [artifacts['hosting'], case, '--exact', '--ignored', '--nocapture', '--test-threads=1'],
                           90, defer_failure=True, environment=environment)
            if text is None:
                sdk_failures.append(name)
            return text
        if node_sdk:
            movement_results = []
            for scenario, name, algorithms, cases in (
                ('session-handoff', 'installed_node_session_handoff_retains_actual_audio_roles_and_recovery', ['EdDSA', 'ES256'], 4),
                ('conversation-handoff', 'installed_node_conversation_handoff_retains_actual_audio_roles_and_recovery', ['EdDSA', 'ES256'], 4),
                ('device-move-eddsa', 'installed_node_eddsa_device_move_preserves_audio_and_current_authority', ['EdDSA'], 4),
                ('device-move-es256', 'installed_node_es256_device_move_preserves_audio_and_current_authority', ['ES256'], 4),
                ('device-move-cancelled', 'installed_node_device_move_cancellation_remains_recorded_after_recovery', ['EdDSA', 'ES256'], 6),
                ('device-move-recording-failure', 'installed_node_device_move_recording_failure_recovers_one_known_commit', ['EdDSA', 'ES256'], 2),
            ):
                case = 'canonical_native_connections::consultation::groups::sdk_movements::' + name
                text = sdk_case('installed-node-movement-' + scenario, case, {'CONNECT_GATE6_NODE_MOVEMENTS': '1'})
                if text is None:
                    movement_results.append({'scenario': scenario, 'passed': False})
                    continue
                assert re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', text) == [('1', '0', '0')]
                witnesses = [json.loads(line.split('installed-node-movement-witness ', 1)[1])
                             for line in text.splitlines() if 'installed-node-movement-witness ' in line]
                assert len(witnesses) == 1
                witness = witnesses[0]
                assert witness['profile'] == 'node-movements/1' and witness['scenario'] == scenario
                assert witness['algorithms'] == algorithms and witness['cases'] == cases
                assert witness['helpers_joined'] == cases * 2 and witness['servers_joined'] == cases
                assert witness['physical_peers_drained'] and witness['media_readers_drained']
                movement_results.append(dict(witness, passed=True))
            report['installed_node_movements'] = {'passed': all(v['passed'] for v in movement_results),
                'profile': 'node-movements/1', 'results': movement_results, 'request_budget_ms': 4000,
                'phase_deadline_seconds': 15, 'whole_function_deadline_seconds': 90,
                'scope': 'Actual native peers/audio with installed public Node controller; topology/role/binding match host result, fresh original-record recovery, current revocation/cancellation/known-commit failure, original helpers/servers/drains. No model/provider/remote or full SDK qualification.'}
            consultation_results = []
            for scenario, name, helpers in (
                ('restored', 'installed_node_consultation_and_return_preserve_actual_private_and_restored_audio', 3),
                ('remote-ended', 'installed_node_consultation_return_after_remote_eof_cannot_claim_a_continuing_call', 2),
            ):
                case = 'canonical_native_connections::consultation::sdk_controls::node_controls::' + name
                text = sdk_case('installed-node-consultation-' + scenario, case)
                if text is None:
                    consultation_results.append({'scenario': scenario, 'passed': False})
                    continue
                assert re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', text) == [('1', '0', '0')]
                witnesses = [json.loads(line.split('installed-node-consultation-witness ', 1)[1])
                             for line in text.splitlines() if 'installed-node-consultation-witness ' in line]
                assert len(witnesses) == 2 and {w['algorithm'] for w in witnesses} == {'EdDSA', 'ES256'}
                for witness in witnesses:
                    assert witness['audio_case'] == scenario and witness['current_return_authority']
                    assert witness['original_record_recovered'] and witness['helpers_joined'] == helpers and witness['server_joined']
                    assert witness['physical_peers_drained'] and witness['media_readers_drained']
                consultation_results.append({'scenario': scenario, 'passed': True,
                    'helpers_joined': sum(w['helpers_joined'] for w in witnesses), 'servers_joined': len(witnesses)})
            report['installed_node_consultation_audio'] = {'passed': all(v['passed'] for v in consultation_results),
                'results': consultation_results, 'algorithms': ['EdDSA', 'ES256'],
                'whole_function_deadline_seconds': 90, 'request_budget_ms': 4000, 'phase_deadline_seconds': 15,
                'scenarios': ['restored', 'remote-ended'], 'required_original_helpers_joined': 10, 'required_servers_joined': 4,
                'verified_original_helpers_joined': sum(v.get('helpers_joined', 0) for v in consultation_results),
                'verified_servers_joined': sum(v.get('servers_joined', 0) for v in consultation_results),
                'scope': 'Public installed Node controls with actual native private/restored audio and remote EOF; no model/provider/remote lane or full SDK qualification.'}
            report['scope'] += ' Also installed Node private/restored decoded audio and remote-EOF controls.'
            case = 'canonical_stdio::node_sdk::installed_typed_node_sdk_recovers_typed_operations_and_current_authority'
            text = sdk_case('installed-node-native-control', case)
            if text is None:
                report['installed_node_native'] = {'passed': False, 'functional_phase_profile': 'node-controls/3'}
            else:
                counts = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', text)
                assert counts == [('1', '0', '0')]
                witnesses = [json.loads(line.split('installed-typed-node-sdk-witness ', 1)[1])
                             for line in text.splitlines() if 'installed-typed-node-sdk-witness ' in line]
                assert len(witnesses) == 2
                assert {w['completed']['algorithm'] for w in witnesses} == {'EdDSA', 'ES256'}
                for witness in witnesses:
                    assert witness['functional_phase_profile'] == 'node-controls/3'
                    assert witness['controls']['session_participants_accepted']
                    assert witness['native']['native_controls_completed']
                    assert witness['initial']['movement_current_authority_refused']
                    assert witness['initial']['node_capability_support_only']
                    assert witness['initial']['native_connection_controls'] and witness['initial']['native_selection']
                    assert witness['initial']['native_http_record'] and witness['initial']['observation_only_stream']
                    assert witness['recovered']['typed_connection_end_recovered']
                    assert witness['receipts']['current_delivery_fact'] and witness['completed']['typed_receipts_current_authority']
                    assert witness['completed']['helpers_joined'] == 3
                    assert witness['server_shutdown_joined'] and witness['physical_peers_drained'] and witness['media_readers_drained']
                report['installed_node_native'] = {'passed': True, 'package': node_package,
                    'functional_phase_profile': 'node-controls/3', 'request_budget_ms': 4000,
                    'scenario_deadline_seconds': 15, 'whole_function_deadline_seconds': 90,
                    'algorithms': ['EdDSA', 'ES256'], 'original_helpers_joined': 6, 'servers_joined': 2,
                    'physical_peers_drained': True, 'media_readers_drained': True,
                    'scope': 'Installed current SDK through actual Rust/native/HTTP control; no decoded media, named remote/provider/model or full SDK qualification.'}
                report['scope'] += ' Also one installed Node native control function for both algorithms, separate callers/peers and original drains; no decoded media claim.'
        report['sdk_failures'] = sdk_failures
        assert not sdk_failures, 'Required installed SDK behavior failed; all independent scenes retained'
        report["behavior_passed"] = True
    finally:
        report["source_inputs_unchanged"] = sources() == before
        report["passed"] = report.get("behavior_passed", False) and report["source_inputs_unchanged"]
        output("source_unchanged", str(report["source_inputs_unchanged"]).lower())
        report["finished_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        (EVIDENCE / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    assert report["passed"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preflight", action="store_true")
    parser.add_argument("--node-sdk", action="store_true", help="Include installed synthetic caller/peer Node native control journey")
    args = parser.parse_args()
    preflight() if args.preflight else run(args.node_sdk)


if __name__ == "__main__":
    main()
