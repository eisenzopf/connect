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
spec = importlib.util.spec_from_file_location('received_profile', ROOT / 'scripts/check-gate6-received-dtmf.py')
NATIVE = importlib.util.module_from_spec(spec)
spec.loader.exec_module(NATIVE)
REQUIRED = sorted(set(NATIVE.REQUIRED) | set(['peer_native::canonical_media::native_offers::media_profiles::actual_signed_native_three_audio_codecs_decode_across_unequal_mtu_peers_and_sequence_wrap', 'peer_native::canonical_media::native_offers::media_profiles::actual_signed_native_camera_and_screen_refragment_to_physical_mtu_and_independently_decode_vp8', 'peer_native::canonical_media::native_offers::media_profiles::actual_signed_native_malformed_oversized_and_expired_packets_are_isolated_and_live_audio_recovers', 'peer_native::canonical_media::reliable_data::actual_signed_reliable_text_json_and_binary_profiles_validate_decode_and_recover_on_all_native_wires', 'cross_runtime_media_vectors_match_checked_rust_and_browser_boundaries', 'peer_ids_are_nonzero_monotonic_and_never_reused', 'route_session_and_connection_indexes_remove_exact_bindings', 'reservation_commit_is_atomic_and_owner_checks_are_explicit', 'shutdown_cancels_active_and_reserved_routes_and_rejects_new_work', 'nack::responder::tests::negotiated_audio_nack_does_not_replay_telephone_event_or_unknown_payloads_on_its_ssrc', 'peer_connection::handler::endpoint::rtx_feedback_tests::declared_rtx_first_packet_reaches_primary_track_without_feedback_reset', 'peer_connection::handler::endpoint::rtx_feedback_tests::declared_rtx_source_clock_registry_preserves_transport_receipts_and_primary_recovery']))



def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def source_inputs():
    inputs = NATIVE.source_inputs()
    for name in ('scripts/check-gate6-media-profiles.py', 'docs/developer/media-profiles.md'):
        inputs[name] = digest(ROOT / name)
    for name in ('crates/uctp/rvoip-uctp/tests/fixtures/cross_runtime_media.json','crates/uctp/rvoip-uctp/tests/fixtures/uctp_full_rtp.pcap.hex','tests/browser-smoke/fixtures/uctp_wt_acceptance.mjs','tests/browser-smoke/tests/media_framing.test.mjs','docs/CONVERSATION_PROTOCOL.md'):
        inputs['.deps/rvoip/'+name]=digest(ROOT/'.deps/rvoip'/name)
    return dict(sorted(inputs.items()))


def main():
    directory = ROOT / '.local/gate-6/media-profiles' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    before = source_inputs()
    steps = []
    generated=directory/'generated';generated.mkdir()
    vectors=json.loads((ROOT/'.deps/rvoip/crates/uctp/rvoip-uctp/tests/fixtures/cross_runtime_media.json').read_text())
    required_node={v['name'] for group in ('valid','invalid') for v in vectors[group]}

    def run(name, command, minimum=0, artifact=None):
        log = directory / (name + '.log')
        env = dict(os.environ)
        env['CONNECT_G6_MEDIA_PROFILE_ARTIFACTS']=str(generated)
        if artifact:
            env['CARGO_MANIFEST_DIR'] = str(pathlib.Path(artifact['manifest']).parent)
            assert digest(pathlib.Path(command[0])) == artifact['sha256']
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT)
        raw = log.read_text(errors='replace')
        tests = re.findall(r'^test (.+?) \.\.\. ok$', raw, re.M)
        if name=='browser-vectors':tests=re.findall(r'^ok \d+ - (.+)$',raw,re.M)
        ignored = re.findall(r'^test (.+?) \.\.\. ignored[^\n]*$', raw, re.M)
        passed = result.returncode == 0 and len(tests) >= minimum
        binaries = {str(pathlib.Path(command[0]).relative_to(ROOT)): digest(pathlib.Path(command[0]))} if artifact else {}
        if artifact:
            passed = passed and artifact['sha256'] in binaries.values()
        steps.append(dict(name=name, command=command, environment={'CARGO_MANIFEST_DIR': env['CARGO_MANIFEST_DIR'],'CONNECT_G6_MEDIA_PROFILE_ARTIFACTS':str(generated)} if artifact else {},
                          passed=passed, exit_code=result.returncode, tests_passed=tests, tests_ignored=ignored,
                          log=str(log.relative_to(ROOT)), log_sha256=digest(log), test_binaries_sha256=binaries))
        print(json.dumps(dict(step=name, passed=passed, tests=len(tests), ignored=len(ignored))), flush=True)
        return passed

    ok = run('dependencies', ['bash', 'scripts/prepare-dependencies.sh'])
    if ok:
        ok = run('format', ['bash', 'scripts/format.sh', '--check'])
    files = re.findall(r'^\+\+\+ b/(.+\.rs)$', (ROOT / 'patches/rvoip/gate6-media-profiles.patch').read_text(), re.M)
    if ok:
        ok = run('upstream-format', ['rustfmt', '--edition', '2021', '--config', 'skip_children=true', '--check'] + ['.deps/rvoip/' + name for name in files])
    artifacts = {}
    if ok:
        ok = run('build', ['cargo', 'test', '--locked', '--offline', '-j2', '--target-dir', 'target', '--manifest-path', '.deps/rvoip/Cargo.toml', '-p', 'rvoip-core', '-p', 'rvoip-media-core', '-p', 'rvoip-sip', '-p', 'rvoip-sip-dialog', '-p', 'rvoip-uctp', '-p', 'rvoip-webrtc', '-p', 'rvoip-rtp-core', '-p','rvoip-webrtc-stack','-p','rvoip-rtc','-p','rtc-interceptor', '--features', 'rvoip-core/opus,rvoip-core/vcon-signing', '--lib', '--test', 'dtmf_info_route', '--test', 'dtmf_auto_route', '--test', 'adapter_dtmf_network', '--test', 'owned_info', '--test', 'v2_peer_admission', '--test', 'dtmf_wire','--test','datagram_conformance','--test','peer_media_router', '--no-run', '--message-format=json'])
        for line in (directory / 'build.log').read_text(errors='replace').splitlines():
            try:
                item = json.loads(line)
            except ValueError:
                continue
            if item.get('reason') == 'compiler-artifact' and item.get('executable'):
                artifacts[item['target']['name']] = dict(executable=item['executable'], manifest=item['manifest_path'], sha256=digest(pathlib.Path(item['executable'])))
        ok = ok and set(artifacts) == {'rvoip_core','rvoip_media_core','rvoip_sip','rvoip_sip_dialog','dtmf_info_route','dtmf_auto_route','adapter_dtmf_network','owned_info','rvoip_uctp','v2_peer_admission','rvoip_webrtc','rvoip_rtp_core','dtmf_wire','datagram_conformance','peer_media_router','webrtc','rtc','rtc_interceptor'}
        steps[-1]['passed'] = ok
    suites = [
        ('decoded-native','v2_peer_admission','peer_native::canonical_media::native_offers::media_profiles::',3),
        ('reliable-data','v2_peer_admission','peer_native::canonical_media::reliable_data::',14),
        ('framing','datagram_conformance','',6),
        ('peer-router','peer_media_router','',4),
        ('feedback','rtc_interceptor','',135),
        ('rtc-repair','rtc','rtx_feedback_tests',2),
        ('rtc-owned-profile','rtc','owned_dtmf',1),
        ('owned-srtp-io','webrtc','',18),
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
        ok = run(name, [artifact['executable']] + ([selected] if selected else []) + (['--skip', '::dtmf_route::','--skip','::media_profiles::'] if name == 'native-offers' else []) + ['--test-threads=1'], minimum, artifact)
    if ok:
        ok=run('browser-vectors',['node','--test','--test-reporter=tap','.deps/rvoip/tests/browser-smoke/tests/media_framing.test.mjs'],len(required_node))
    artifacts_generated={str(p.relative_to(ROOT)):digest(p) for p in sorted(generated.glob('*')) if p.is_file()}
    if ok:
        assert len(list(generated.glob('*.ivf')))==4
        assert len(list(generated.glob('*.json')))==12
    if ok:
        ok = run('server-integration', ['cargo', 'check', '--locked', '--offline', '-j2', '-p', 'connect-server'])
    if ok:
        ok = run('progress', ['python3', 'scripts/check-gate6-progress.py'])
    if ok:
        ok = run('diff', ['git', 'diff', '--check'])
    observed = {test for step in steps if step['name']!='browser-vectors' for test in step['tests_passed']}
    node_observed={test for step in steps if step['name']=='browser-vectors' for test in step['tests_passed']}
    missing_node=sorted(required_node-node_observed)
    missing = sorted(set(REQUIRED) - observed)
    unchanged = before == source_inputs()
    report = dict(task='G6-024', implementation_status='in-progress', full_task_qualification=False,
                  qualification='Approved G6-024 local engine profile matrix: checked Rust/browser-module framing, exact peer-scoped stream IDs, decoded PCMU/PCMA/Opus and independently decoded VP8 camera/screen on real signed QUIC/WebTransport peers with unequal fixed physical MTUs, visible loss gaps and recovery, malformed/oversized/expired refusal, reliable text/JSON/binary on QUIC/WT/WSS, feedback/RTC repair, owned current DTMF IO and measured native quality. Controlled host-policy fixtures; no production application or remote/browser/hosting acceptance claim.',
                  not_qualified=['Full G6-024: WebSocket/WebRTC fallback media wiring and decoded-path audit remain open.',
                                 'Production Store/journal/service/HTTP/SDK installation and downstream G6-028 combined engine qualification.',
                                 'Linux x86_64, live browser/remote/provider profiles, G6-093 named rehearsals, full Gate 6 acceptance and publication.'],
                  executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
                  ffmpeg=subprocess.check_output(['ffmpeg','-version'],text=True).splitlines()[0],
                  node=subprocess.check_output(['node','--version'],text=True).strip(),
                  generated_artifacts_sha256=artifacts_generated,node_checks=len(node_observed),missing_node_vectors=missing_node,
                  rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(),
                  source_inputs_sha256=before, sources_unchanged=unchanged, steps=steps,
                  runtime_tests=sum(len(step['tests_passed']) for step in steps if step['name']!='browser-vectors'),
                  runtime_checks=sum(len(step['tests_passed']) for step in steps), runtime_unique_tests=len(observed),
                  required_cases=REQUIRED, missing_required_cases=missing,
                  passed=ok and unchanged and not missing and not missing_node and all(step['passed'] for step in steps))
    path = ROOT / 'docs/evidence/gate-6/media-profiles.json'
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: report[k] for k in ('passed', 'runtime_tests', 'runtime_unique_tests', 'sources_unchanged', 'missing_required_cases')}))
    raise SystemExit(0 if report['passed'] else 1)


if __name__ == '__main__':
    main()
