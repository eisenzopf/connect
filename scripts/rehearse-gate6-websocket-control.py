#!/usr/bin/env python3
"""Rehearse canonical TLS WebSocket control, discovery, shared budget and owned lifetimes."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import runpy
from lib.sealed_executable import seal_executable
import signal
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
BASE = runpy.run_path(str(ROOT / 'scripts/rehearse-gate6-fifty-participants.py'))
INDEPENDENT = ['canonical_websocket_control::cancelling_host_shutdown_keeps_wss_and_pending_tls_owned', 'canonical_webtransport_discovery::discovery_sdk_refuses_incompatible_and_oversized_responses_during_collection', 'canonical_websocket_control::tls_wss_signed_control_reconnect_and_native_original_receipt_eddsa', 'canonical_websocket_control::tls_wss_signed_control_reconnect_and_native_original_receipt_es256', 'canonical_websocket_control::tls_origin_mount_refusals_do_not_consume_canonical_slots', 'canonical_websocket_control::physical_budget_is_shared_across_wss_http3_and_native_and_reuses_retired_slots', 'canonical_websocket_control::dropping_host_closes_live_wss_and_releases_tcp_listener', 'canonical_websocket_control::signing_expiry_retires_wss_and_linked_native_and_http3', *['canonical_websocket_control::production_sdk_uses_actual_tls_wss_and_recovers_original_receipt_'+algorithm for algorithm in ('eddsa','es256')]]
BROWSER = []
CONFIG = ['canonical_websocket::tests::bind_policy_allows_remote_wildcards_but_never_multicast_or_broadcast', 'canonical_websocket::tests::derived_local_contact_preserves_bound_ipv4_and_ipv6_address_family', 'canonical_websocket::tests::exact_origins_mount_and_remote_contact_are_required_before_startup', 'canonical_websocket::tests::advertised_contact_refuses_parser_normalization_of_whitespace_and_controls']

def digest(path):
    with Path(path).open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def sources():
    result = BASE['sources']()
    result['scripts/rehearse-gate6-websocket-control.py'] = digest(ROOT / 'scripts/rehearse-gate6-websocket-control.py')
    for folder in ('sdk/typescript/src', 'sdk/typescript/dist', 'sdk/tests'):
        for path in (ROOT / folder).rglob('*'):
            if path.is_file() and path.suffix in ('.ts', '.js', '.mjs', '.py'):
                result[str(path.relative_to(ROOT))] = digest(path)
    # The native fifty-Participant helper excludes JS. Both the embedded peer
    # and the external CDP runner are executable inputs to this rehearsal.
    for path in (ROOT / 'crates/connect-server/tests/fixtures/gate6-webtransport-browser').glob('*.mjs'):
        result[str(path.relative_to(ROOT))] = digest(path)
    for name in ('canonical-http.md', 'application-sdk.md', 'canonical-webtransport.md', 'canonical-websocket.md'):
        path = ROOT / 'docs/developer' / name
        if path.is_file():
            result[str(path.relative_to(ROOT))] = digest(path)
    return dict(sorted(result.items()))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target-dir', type=Path, required=True)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--jobs', type=int, default=4, choices=range(1, 9))
    parser.set_defaults(installed_browser=False)
    args = parser.parse_args()
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    directory = (args.output or ROOT / '.local/gate-6/websocket-control' / stamp).resolve()
    if not directory.is_relative_to((ROOT / '.local').resolve()):
        parser.error('Raw evidence must remain under ignored .local/')
    directory.mkdir(parents=True, exist_ok=False)
    (directory / 'tmp').mkdir()
    env = dict(os.environ, CARGO_INCREMENTAL='0', TMPDIR=str(directory / 'tmp'), CONNECT_GATE6_BROWSER_ARTIFACTS=str(directory / 'browser-artifacts'))
    before = sources()
    report = dict(passed=False, source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                  source_inputs_sha256=before, source_lock_sha256=digest(ROOT / 'dependencies/source-lock.json'),
                  scope='Local isolated actual TLS WebSocket signed control for EdDSA/ES256, original durable receipt recovery via the public native SDK, Origin/mount/TLS refusal, configured authenticated discovery, shared QUIC/WebTransport/WSS physical budget, host drop and signing-key expiry. No browser network, WebRTC media, automatic fallback, remote/provider, complete G6-051, acceptance or publication qualification',
                  host=platform.platform(), installed_browser=args.installed_browser,
                  skipped=[] if args.installed_browser else [dict(case=name, reason='Explicit installed-browser lane not selected; not a pass.') for name in BROWSER],
                  steps=[], cases=[], executables=[], started_at=datetime.datetime.now(datetime.timezone.utc).isoformat())

    def step(name, command, artifacts=False):
        start = time.monotonic()
        with (directory / (name + '.log')).open('w') as log:
            if artifacts:
                with (directory / 'build.jsonl').open('w') as output:
                    code = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=log).returncode
            else:
                code = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT).returncode
        report['steps'].append(dict(name=name, command=command, exit_code=code, elapsed_seconds=time.monotonic() - start, log_sha256=digest(directory / (name + '.log'))))
        print(json.dumps(dict(step=name, exit_code=code)), flush=True)
        return code

    try:
        assert step('dependencies', ['bash', 'scripts/prepare-dependencies.sh']) == 0
        assert step('sdk-suite', ['npm', 'test', '--prefix', 'sdk/typescript']) == 0
        assert step('rust-format', ['bash', 'scripts/format.sh', '--check']) == 0
        assert step('compiler', ['rustc', '--version', '--verbose']) == 0
        assert step('openapi', ['python3', 'scripts/generate-canonical-http-openapi.py', '--check']) == 0
        assert step('build', ['cargo', 'test', '-p', 'connect-server', '--lib', '--test', 'gate6_canonical_webtransport', '--no-run', '--release', '--locked', '--offline', '-j' + str(args.jobs), '--target-dir', str(args.target_dir.resolve()), '--message-format=json'], True) == 0
        rows = [json.loads(line) for line in (directory / 'build.jsonl').read_text().splitlines()]
        artifacts = [r for r in rows if r.get('reason') == 'compiler-artifact' and r.get('executable') and r['target']['name'] in ('connect_server', 'gate6_canonical_webtransport') and r['profile']['test']]
        assert len(artifacts) == 2, 'Expected exact server unit and WebTransport integration executables'
        for artifact in artifacts:
            assert artifact['profile']['opt_level'] == '3'
            binary = Path(artifact['executable'])
            name = artifact['target']['name']
            sha = digest(binary)
            sealed = directory / ('qualified-' + name)
            seal_executable(binary, sealed, sha)
            assert digest(sealed) == sha
            report['executables'].append(dict(target=name, path=str(binary), sealed_path=str(sealed.relative_to(ROOT)), sha256=sha))
            listing = subprocess.check_output([str(sealed), '--list'], env=env, text=True)
            available = {line.removesuffix(': test') for line in listing.splitlines() if line.endswith(': test')}
            selected = CONFIG if name == 'connect_server' else INDEPENDENT + (BROWSER if args.installed_browser else [])
            assert set(selected).issubset(available), sorted(set(selected) - available)
            for case in selected:
                command = [str(sealed), case, '--exact', '--test-threads=1', '--nocapture']
                if case in BROWSER:
                    command.append('--ignored')
                logpath = directory / ('case-' + str(len(report['cases'])) + '.log')
                start = time.monotonic()
                with logpath.open('w') as log:
                    process = subprocess.Popen(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                    try:
                        code = process.wait(timeout=180)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait()
                        code = 124
                text = logpath.read_text()
                passed = code == 0 and bool(re.search(r'test result: ok\. 1 passed; 0 failed; 0 ignored;', text))
                row = dict(case=case, passed=passed, exit_code=code, elapsed_seconds=time.monotonic() - start, log=str(logpath.relative_to(ROOT)), log_sha256=digest(logpath))
                report['cases'].append(row)
                print(json.dumps(row), flush=True)
        report['executables_unchanged'] = all(digest(row['path']) == row['sha256'] and digest(ROOT / row['sealed_path']) == row['sha256'] for row in report['executables'])
    except Exception as error:
        report['failure'] = str(error)
    finally:
        report['source_inputs_unchanged'] = sources() == before
        required = len(CONFIG) + len(INDEPENDENT) + (len(BROWSER) if args.installed_browser else 0)
        report['passed'] = len(report['cases']) == required and all(row['passed'] for row in report['cases']) and report['source_inputs_unchanged'] and report.get('executables_unchanged') is True and 'failure' not in report
        report['finished_at'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        (directory / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(dict(passed=report['passed'], summary=str(directory / 'summary.json'))), flush=True)
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
