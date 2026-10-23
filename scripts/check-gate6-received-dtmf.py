#!/usr/bin/env python3
"""Qualify Received SIP/WebRTC source composition through Core routing for the open G6-024 task."""
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
spec = importlib.util.spec_from_file_location('core_native_profile', ROOT / 'scripts/check-gate6-core-native-dtmf.py')
NATIVE = importlib.util.module_from_spec(spec)
spec.loader.exec_module(NATIVE)
REQUIRED = sorted({'relay::controller::dtmf_receive::tests::received_dtmf_start_before_media_update_cannot_borrow_new_generation_when_end_arrives_after_update','relay::controller::dtmf_receive::tests::received_dtmf_media_update_invalidates_old_observation_and_queued_packet_without_successor_permission','relay::controller::dtmf_receive::tests::received_dtmf_observation_does_not_keep_an_removed_media_registry_or_udp_session_alive'} | set(NATIVE.REQUIRED) | set(['received_source_registration_preserves_original_facts_and_requires_fresh_independent_signal_policy', 'received_source_replacement_drop_and_retirement_invalidate_queued_metadata_without_idle_permission', 'received_source_queue_is_bounded_and_old_or_foreign_inputs_do_not_acquire_current_registration', 'received_rtp_incomplete_and_fractional_durations_remain_observations_without_fabricated_effects', 'peer_native::canonical_media::native_offers::dtmf_route::actual_received_sip_info_and_rtp_reach_signed_native_with_original_source_method_and_timing', 'peer_native::canonical_media::native_offers::dtmf_route::received_sip_subscription_replacement_and_source_end_prevent_native_delivery', 'actual_srtp_received_source_routes_through_core_with_exact_timing_and_fresh_signal_authority', 'peer::source_clock::received_signal_tests::signal_fence_rejects_same_ssrc_track_aba_peer_retirement_and_finished_pump_without_primary_clock', 'peer::source_clock::received_signal_tests::pending_signal_poll_releases_track_metadata_and_cannot_keep_a_retired_source_alive', 'orchestrator::dtmf_routing::received::binding_tests::binding_epoch_does_not_revive_after_session_aba_and_dead_sources_do_not_accumulate']))



def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def source_inputs():
    inputs = NATIVE.source_inputs()
    for name in ('scripts/check-gate6-received-dtmf.py', 'docs/developer/received-dtmf.md'):
        inputs[name] = digest(ROOT / name)
    return dict(sorted(inputs.items()))


def main():
    directory = ROOT / '.local/gate-6/received-dtmf' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
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
    files = re.findall(r'^\+\+\+ b/(.+\.rs)$', (ROOT / 'patches/rvoip/gate6-received-dtmf.patch').read_text(), re.M)
    if ok:
        ok = run('upstream-format', ['rustfmt', '--edition', '2021', '--config', 'skip_children=true', '--check'] + ['.deps/rvoip/' + name for name in files])
    artifacts = {}
    if ok:
        ok = run('build', ['cargo', 'test', '--locked', '--offline', '-j2', '--target-dir', 'target', '--manifest-path', '.deps/rvoip/Cargo.toml', '-p', 'rvoip-core', '-p', 'rvoip-media-core', '-p', 'rvoip-sip', '-p', 'rvoip-sip-dialog', '-p', 'rvoip-uctp', '-p', 'rvoip-webrtc', '-p', 'rvoip-rtp-core', '--features', 'rvoip-core/opus,rvoip-core/vcon-signing', '--lib', '--test', 'dtmf_info_route', '--test', 'dtmf_auto_route', '--test', 'adapter_dtmf_network', '--test', 'owned_info', '--test', 'v2_peer_admission', '--test', 'dtmf_wire', '--no-run', '--message-format=json'])
        for line in (directory / 'build.log').read_text(errors='replace').splitlines():
            try:
                item = json.loads(line)
            except ValueError:
                continue
            if item.get('reason') == 'compiler-artifact' and item.get('executable'):
                artifacts[item['target']['name']] = dict(executable=item['executable'], manifest=item['manifest_path'], sha256=digest(pathlib.Path(item['executable'])))
        ok = ok and set(artifacts) == {'rvoip_core','rvoip_media_core','rvoip_sip','rvoip_sip_dialog','dtmf_info_route','dtmf_auto_route','adapter_dtmf_network','owned_info','rvoip_uctp','v2_peer_admission','rvoip_webrtc','rvoip_rtp_core','dtmf_wire'}
        steps[-1]['passed'] = ok
    suites = [
        ('native-sources', 'v2_peer_admission', 'peer_native::canonical_media::native_offers::dtmf_route::', 22),
        ('native-offers', 'v2_peer_admission', 'peer_native::canonical_media::native_offers::', 49),
        ('native-credential-rebind', 'v2_peer_admission', 'peer_native::canonical_media::media_control::credential_rebind_wire_tests::', 2),
        ('native-library', 'rvoip_uctp', '', 95),
        ('core-info-wire', 'dtmf_info_route', '', 8),
        ('core-routing', 'dtmf_auto_route', '', 14),
        ('core-library', 'rvoip_core', '', 421),
        ('shared-dtmf', 'rvoip_media_core', 'dtmf', 30),
        ('sip-dtmf', 'rvoip_sip', 'dtmf', 5),
        ('sip-network', 'adapter_dtmf_network', '', 3),
        ('owned-io', 'rvoip_sip_dialog', 'transaction::egress::', 2),
        ('transaction-manager', 'rvoip_sip_dialog', 'transaction::manager::', 137),
        ('client-transactions', 'rvoip_sip_dialog', 'transaction::client::', 31),
        ('exact-completions', 'rvoip_sip_dialog', 'transaction::completion::', 9),
        ('canonical-materializers', 'rvoip_sip_dialog', 'manager::transaction_integration::', 17),
        ('received-webrtc-clock', 'rvoip_webrtc', 'peer::source_clock::', 2),
        ('webrtc-media', 'rvoip_webrtc', 'media::', 1),
        ('actual-srtp', 'dtmf_wire', '', 12),
        ('rtp-source-session', 'rvoip_rtp_core', 'session::negotiated_telephone_tests::', 3),
        ('rtp-decoder', 'rvoip_rtp_core', 'telephone_event::', 1),
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
                  qualification='Actual received SIP INFO/RFC4733 and WebRTC SRTP source composition retains original Core binding, method/provenance, original event-start timing across media changes and stable physical observation identity. Explicit bounded metadata-only source subscriptions invalidate replacement, drop, source retirement and Core binding ABA. Source lifecycle/media/track fences apply at actual target IO under fresh independent signal authority. Actual SIP input to signed native recipients and actual SRTP input through Core and its authorized WebRTC adapter dispatch to a second SRTP peer; previous native/SIP/Core regressions repeated.',
                  not_qualified=['Full G6-024 remaining approved media profiles and G6-028.',
                                 'Controlled local host journal and permission fixtures do not qualify production Store/service/HTTP/SDK installation.',
                                 'Success proves local writer submission; no Linux x86_64, remote/browser/provider or Gate 6 acceptance claim.'],
                  executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
                  rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(),
                  source_inputs_sha256=before, sources_unchanged=unchanged, steps=steps,
                  runtime_tests=sum(len(step['tests_passed']) for step in steps), runtime_unique_tests=len(observed),
                  required_cases=REQUIRED, missing_required_cases=missing,
                  passed=ok and unchanged and not missing and all(step['passed'] for step in steps))
    path = ROOT / 'docs/evidence/gate-6/received-dtmf.json'
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: report[k] for k in ('passed', 'runtime_tests', 'runtime_unique_tests', 'sources_unchanged', 'missing_required_cases')}))
    raise SystemExit(0 if report['passed'] else 1)


if __name__ == '__main__':
    main()
