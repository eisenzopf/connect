#!/usr/bin/env python3
"""Qualify sealed native sources through actual Core and SIP writes for the open G6-024 task."""
import datetime
import hashlib
import importlib.util
import json
import os
import pathlib
import platform
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('info_profile', ROOT / 'scripts/check-gate6-dtmf-info-route.py')
INFO = importlib.util.module_from_spec(spec)
spec.loader.exec_module(INFO)
REQUIRED = INFO.REQUIRED + ['source_metadata_hint_without_a_physical_poll_fence_cannot_authorize_signal_io'] + ['peer_native::canonical_media::native_offers::dtmf_route::actual_signed_quic_and_webtransport_dtmf_reaches_core_and_sip_info_with_original_operation', 'peer_native::canonical_media::native_offers::dtmf_route::actual_signed_native_dtmf_reaches_negotiated_rfc4733_without_info_fallback', 'peer_native::canonical_media::native_offers::dtmf_route::native_observation_and_media_permission_do_not_supply_independent_signal_authority', 'peer_native::canonical_media::native_offers::dtmf_route::native_source_recipient_and_signing_key_revocation_stop_info_suffix_without_idle_readers', 'peer_native::canonical_media::native_offers::dtmf_route::native_source_stream_retirement_stops_core_info_suffix_while_signaling_stays_live', 'peer_native::canonical_media::native_offers::dtmf_route::new_native_offer_epoch_invalidates_old_dtmf_metadata_before_next_info_write', 'peer_native::canonical_media::native_offers::dtmf_route::sealed_native_observation_cannot_bind_to_a_foreign_core_or_recipient_alias', 'peer_native::canonical_media::native_offers::dtmf_route::native_source_retirement_after_signal_grant_but_before_socket_poll_prevents_all_effects', 'peer_native::canonical_media::native_offers::dtmf_route::native_source_pending_poll_releases_controller_credential_and_port_metadata_custody', 'peer_native::canonical_media::native_offers::dtmf_route::actual_native_physical_peer_close_stops_info_suffix_under_original_operation', 'peer_native::canonical_media::native_offers::dtmf_route::native_core_metadata_without_sealed_source_fence_cannot_schedule_signal_io']


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def source_inputs():
    inputs = INFO.source_inputs()
    for name in ('scripts/check-gate6-native-dtmf-route.py', 'docs/developer/native-dtmf-route.md'):
        inputs[name] = digest(ROOT / name)
    return dict(sorted(inputs.items()))


def main():
    directory = ROOT / '.local/gate-6/native-dtmf-route' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    before = source_inputs()
    steps = []

    def run(name, command, minimum=0, artifact=None):
        log = directory / (name + '.log')
        env = dict(os.environ)
        if artifact:
            env['CARGO_MANIFEST_DIR'] = str(pathlib.Path(artifact['manifest']).parent)
            assert digest(pathlib.Path(command[0])) == artifact['sha256']
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT)
        raw = log.read_text(errors='replace')
        tests = re.findall(r'^test (.+?) \.\.\. ok$', raw, re.M)
        ignored = re.findall(r'^test (.+?) \.\.\. ignored[^\n]*$', raw, re.M)
        passed = result.returncode == 0 and len(tests) >= minimum
        binaries = {str(pathlib.Path(command[0]).relative_to(ROOT)): digest(pathlib.Path(command[0]))} if artifact else {}
        if artifact:
            passed = passed and artifact['sha256'] in binaries.values()
        steps.append(dict(name=name, command=command, environment={'CARGO_MANIFEST_DIR': env['CARGO_MANIFEST_DIR']} if artifact else {},
                          passed=passed, exit_code=result.returncode, tests_passed=tests, tests_ignored=ignored,
                          log=str(log.relative_to(ROOT)), log_sha256=digest(log), test_binaries_sha256=binaries))
        print(json.dumps(dict(step=name, passed=passed, tests=len(tests), ignored=len(ignored))), flush=True)
        return passed

    ok = run('dependencies', ['bash', 'scripts/prepare-dependencies.sh'])
    if ok:
        ok = run('format', ['bash', 'scripts/format.sh', '--check'])
    files = re.findall(r'^\+\+\+ b/(.+\.rs)$', (ROOT / 'patches/rvoip/gate6-native-dtmf-route.patch').read_text(), re.M)
    if ok:
        ok = run('upstream-format', ['rustfmt', '--edition', '2021', '--config', 'skip_children=true', '--check'] + ['.deps/rvoip/' + name for name in files])
    artifacts = {}
    if ok:
        ok = run('build', ['cargo', 'test', '--locked', '--offline', '-j2', '--target-dir', 'target', '--manifest-path', '.deps/rvoip/Cargo.toml', '-p', 'rvoip-core', '-p', 'rvoip-media-core', '-p', 'rvoip-sip', '-p', 'rvoip-sip-dialog', '-p', 'rvoip-uctp', '--features', 'rvoip-core/opus,rvoip-core/vcon-signing', '--lib', '--test', 'dtmf_info_route', '--test', 'dtmf_auto_route', '--test', 'adapter_dtmf_network', '--test', 'owned_info', '--test', 'v2_peer_admission', '--no-run', '--message-format=json'])
        for line in (directory / 'build.log').read_text(errors='replace').splitlines():
            try:
                item = json.loads(line)
            except ValueError:
                continue
            if item.get('reason') == 'compiler-artifact' and item.get('executable'):
                artifacts[item['target']['name']] = dict(executable=item['executable'], manifest=item['manifest_path'], sha256=digest(pathlib.Path(item['executable'])))
        ok = ok and set(artifacts) == {'rvoip_core','rvoip_media_core','rvoip_sip','rvoip_sip_dialog','dtmf_info_route','dtmf_auto_route','adapter_dtmf_network','owned_info','rvoip_uctp','v2_peer_admission'}
        steps[-1]['passed'] = ok
    suites = [
        ('native-sources', 'v2_peer_admission', 'peer_native::canonical_media::native_offers::dtmf_route::', 11),
        ('native-offers', 'v2_peer_admission', 'peer_native::canonical_media::native_offers::', 49),
        ('native-credential-rebind', 'v2_peer_admission', 'peer_native::canonical_media::media_control::credential_rebind_wire_tests::', 2),
        ('native-library', 'rvoip_uctp', '', 30),
        ('core-info-wire', 'dtmf_info_route', '', 8),
        ('core-routing', 'dtmf_auto_route', '', 10),
        ('core-library', 'rvoip_core', '', 420),
        ('shared-dtmf', 'rvoip_media_core', 'dtmf', 27),
        ('sip-dtmf', 'rvoip_sip', 'dtmf', 5),
        ('sip-network', 'adapter_dtmf_network', '', 3),
        ('owned-io', 'rvoip_sip_dialog', 'transaction::egress::', 2),
        ('transaction-manager', 'rvoip_sip_dialog', 'transaction::manager::', 137),
        ('client-transactions', 'rvoip_sip_dialog', 'transaction::client::', 31),
        ('exact-completions', 'rvoip_sip_dialog', 'transaction::completion::', 9),
        ('canonical-materializers', 'rvoip_sip_dialog', 'manager::transaction_integration::', 17),
        ('public-udp-info', 'owned_info', '', 4),
    ]
    for name, target, selected, minimum in suites:
        if not ok:
            break
        artifact = artifacts[target]
        ok = run(name, [artifact['executable']] + ([selected] if selected else []) + (['--skip', '::dtmf_route::'] if name == 'native-offers' else []) + ['--test-threads=1'], minimum, artifact)
    if ok:
        ok = run('server-integration', ['cargo', 'check', '--locked', '--offline', '-j2', '-p', 'connect-server'])
    if ok:
        ok = run('progress', ['python3', 'scripts/check-gate6-progress.py'])
    if ok:
        ok = run('diff', ['git', 'diff', '--check'])
    observed = {test for step in steps for test in step['tests_passed']}
    missing = sorted(set(REQUIRED) - observed)
    unchanged = before == source_inputs()
    report = dict(task='G6-024', implementation_status='in-progress', full_task_qualification=False,
                  qualification='Actual signed TLS QUIC/WebTransport native source to Core and actual negotiated SIP INFO/RFC4733 UDP writes, immutable original operation and verified signing-key metadata, exact registered adapter and per-physical-poll flow/controller/credential/audio-port source fences. Independent fresh source/recipient signal policy with no retained ingress invocation; revocation/retirement/reoffer/physical close and post-grant pre-IO races; existing Core/SIP owned write regressions repeated.',
                  not_qualified=['Full G6-024: general native recipient scheduling, richer SIP/WebRTC received-source composition and remaining approved media profiles.',
                                 'Controlled local policy/journal fixtures do not qualify production Store/service/HTTP/SDK installation.',
                                 'Local submission and SIP acceptance do not prove remote playback; no Linux x86_64, remote/browser/provider or Gate 6 acceptance claim.'],
                  executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
                  rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(),
                  source_inputs_sha256=before, sources_unchanged=unchanged, steps=steps,
                  runtime_tests=sum(len(step['tests_passed']) for step in steps), runtime_unique_tests=len(observed),
                  required_cases=REQUIRED, missing_required_cases=missing,
                  passed=ok and unchanged and not missing and all(step['passed'] for step in steps))
    path = ROOT / 'docs/evidence/gate-6/native-dtmf-route.json'
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: report[k] for k in ('passed', 'runtime_tests', 'runtime_unique_tests', 'sources_unchanged', 'missing_required_cases')}))
    raise SystemExit(0 if report['passed'] else 1)


if __name__ == '__main__':
    main()
