#!/usr/bin/env python3
"""Rehearse isolated canonical HTTP3, discovery and optional installed Chrome."""
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
INDEPENDENT = [
    'independent_http3_signed_control_and_reconnect_share_native_receipt_eddsa',
    'independent_http3_signed_control_and_reconnect_share_native_receipt_es256',
    'wrong_missing_duplicate_origin_and_wrong_mount_refuse_before_authority',
    *['canonical_webtransport_media::independent_http3_actual_decoded_duplex_uses_webtransport_bindings_' + algorithm for algorithm in ('eddsa', 'es256')],
    *['canonical_webtransport_discovery::' + name for name in (
        'configured_webtransport_discovery_is_bounded_authenticated_current_and_read_only',
        'disabled_webtransport_and_unrelated_http_listener_do_not_invent_transport_discovery',
        'discovery_sdk_refuses_incompatible_and_oversized_responses_during_collection')],
]
INDEPENDENT += [
    'canonical_webtransport_lifecycle::missing_quic_datagram_capability_is_refused_before_auth_and_keeps_other_peers_alive',
    'canonical_webtransport_media::server_shutdown_with_live_decoded_webtransport_media_retires_all_physical_bindings',
    'canonical_webtransport_media::server_drop_with_live_decoded_webtransport_media_retires_all_physical_bindings',
    'canonical_webtransport_lifecycle::shared_native_and_http3_physical_budget_refuses_excess_and_reuses_only_retired_slots',
    'canonical_webtransport_lifecycle::current_credential_and_key_revocation_retire_http3_without_replaying_durable_mutation',
    'canonical_webtransport_lifecycle::server_signing_expiry_closes_http3_and_native_intake_and_releases_owned_runtime',
    'canonical_webtransport_lifecycle::owned_http3_shutdown_and_drop_restart_preserve_original_receipt_and_fence_old_peers',
    'canonical_webtransport_lifecycle::malformed_and_unallocated_datagrams_are_observed_rejected_before_and_after_auth_without_allocation',
    'canonical_webtransport_lifecycle::authenticated_control_only_endpoint_keeps_commands_but_refuses_audio_without_slots',
    'canonical_webtransport_lifecycle::http3_session_budget_is_shared_with_native_commands_and_refusal_leaves_no_connection',
    'canonical_webtransport_lifecycle::client_credential_and_signing_key_expiry_retire_http3_without_extending_each_other',
    'canonical_webtransport_lifecycle::disabled_webtransport_retains_native_tls_commands_and_refuses_h3_without_authority',
    *['canonical_webtransport_mixed::actual_mixed_webtransport_and_native_duplex_survives_foreign_physical_handle_rejection_' + algorithm for algorithm in ('eddsa', 'es256')],
]

BROWSER = ['canonical_webtransport_browser::installed_chrome_signed_control_reconnect_and_independently_decoded_duplex_' + algorithm for algorithm in ('eddsa', 'es256')]
CONFIG = ['canonical_webtransport::tests::' + name for name in (
    'exact_origins_mount_and_remote_contact_are_required_before_startup',
    'advertised_contact_refuses_parser_normalization_of_whitespace_and_controls')]

CONFIG += ['canonical_webtransport::tests::derived_local_contact_preserves_bound_ipv4_and_ipv6_address_family', 'tls::tests::generated_local_certificate_verifies_loopback_names_and_refuses_other_hosts']


def digest(path):
    with Path(path).open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def sources():
    result = BASE['sources']()
    # The native fifty-Participant helper excludes JS. Both the embedded peer
    # and the external CDP runner are executable inputs to this rehearsal.
    for path in (ROOT / 'crates/connect-server/tests/fixtures/gate6-webtransport-browser').glob('*.mjs'):
        result[str(path.relative_to(ROOT))] = digest(path)
    for name in ('canonical-http.md', 'application-sdk.md', 'canonical-webtransport.md'):
        path = ROOT / 'docs/developer' / name
        if path.is_file():
            result[str(path.relative_to(ROOT))] = digest(path)
    return dict(sorted(result.items()))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target-dir', type=Path, required=True)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--jobs', type=int, default=4, choices=range(1, 9))
    parser.add_argument('--installed-browser', action='store_true', help='Explicitly run owned installed Chrome; no personal profile or certificate bypass.')
    args = parser.parse_args()
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    directory = (args.output or ROOT / '.local/gate-6/webtransport' / stamp).resolve()
    if not directory.is_relative_to((ROOT / '.local').resolve()):
        parser.error('Raw evidence must remain under ignored .local/')
    directory.mkdir(parents=True, exist_ok=False)
    (directory / 'tmp').mkdir()
    env = dict(os.environ, CARGO_INCREMENTAL='0', TMPDIR=str(directory / 'tmp'), CONNECT_GATE6_BROWSER_ARTIFACTS=str(directory / 'browser-artifacts'))
    before = sources()
    report = dict(passed=False, source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                  source_inputs_sha256=before, source_lock_sha256=digest(ROOT / 'dependencies/source-lock.json'),
                  scope='Local isolated actual canonical HTTP3/control/receipt parity, subscribed independently decoded duplex, transport start facts, exact Origin/mount refusal, authenticated bounded SDK discovery, derived Local address-family/TLS identity, shared physical/Session limits, current credential/key lifetimes, actual rejected datagrams, owned shutdown/drop/restart and mixed WT/native isolation. Installed Chrome is explicit. No complete G6-049, browser SDK G6-050, remote/provider, acceptance or publication claim.',
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
