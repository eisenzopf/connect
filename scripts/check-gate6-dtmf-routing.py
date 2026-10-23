#!/usr/bin/env python3
"""Qualify primary audio admission and decoded G.711 routing for open G6-024."""
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
PACKAGES = ['rvoip-rtp-core', 'rvoip-media-core', 'rvoip-core', 'rvoip-sip',
            'rvoip-webrtc', 'rvoip-webrtc-stack', 'rvoip-rtc', 'rtc-interceptor']
REQUIRED = [
    'raw_dtmf_event_does_not_borrow_bridge_authority_for_a_signal_effect',
    'raw_dtmf_does_not_create_a_background_schedule_or_block_lifecycle_drain',
    'dtmf_does_not_forward_when_connection_is_not_bridged',
    'explicit_route_sends_actual_negotiated_rtp_with_exact_source_recipient_and_original_intent',
    'independent_denial_info_or_cancelled_expired_and_foreign_route_never_fall_back_to_raw_effects',
    'current_policy_revoke_between_packets_stops_suffix_without_an_idle_reader_or_logical_retry',
    'source_retirement_cancel_drop_deadline_and_target_capacity_bound_the_owned_schedule',
    'unrelated_conversation_and_missing_source_metadata_cannot_create_a_route_or_signal_grant',
    'dtmf_authority::poll_fence_tests::synchronous_io_poll_fence_releases_on_pending_and_prevents_ready_io_after_retirement',
    'bridge::frame_pump::tests::unnegotiated_pt101_and_four_byte_codec_failures_cannot_escape_audio_admission',
    'bridge::frame_pump::tests::negotiated_primary_pt101_remains_opaque_audio_on_an_equal_codec_route',
    'bridge::frame_pump::tests::g711_transcoding_relabels_primary_payload_and_preserves_decodable_marker',
    'media_graph::tests::unnegotiated_telephone_payload_does_not_enter_primary_graph_or_prove_activity',
    'media_graph::tests::primary_g711_pt101_transcodes_as_negotiated_audio_with_decoded_marker',
    'primary_audio_continues_around_refused_unnegotiated_event_payloads',
    'unsupported_codec_and_four_byte_payload_have_no_control_fallback',
    'session::negotiated_telephone_tests::mixed_primary_and_auxiliary_udp_tracks_one_sequence_without_event_clock_jitter',
    'session::negotiated_telephone_tests::auxiliary_only_and_old_primary_payloads_do_not_prove_current_primary_flow',
    'session::negotiated_telephone_tests::actual_udp_requires_explicit_mapping_and_reports_exact_remote_source_and_segment_clock',
    'telephone_event::negotiated_profile_tests::unsupported_packed_suffix_cannot_publish_or_consume_valid_prefix',
    'telephone_event::negotiated_profile_tests::renegotiation_removal_and_readdition_preserve_physical_replay_history',
    'relay::controller::rtp_management::dtmf_configuration_tests::renegotiation_during_permission_acquisition_cannot_submit_old_pt_or_clock',
    'relay::controller::rtp_management::dtmf_configuration_tests::changed_configuration_between_packets_stops_old_suffix_and_preserves_whole_intent_uncertainty',
    'adapters::media_adapter::dtmf_transaction_tests::pending_sip_transaction_refuses_tone_without_waiting_or_retaining_idle_host_permission',
    'adapters::media_adapter::telephone_event_negotiation_tests::changed_auxiliary_clock_and_invalid_event_lists_cannot_negotiate',
    'actual_rfc4733_adapter_preserves_observed_duration_and_live_connection',
    'negotiated::actual_sip_dynamic_pt110_8khz_event_subset_has_fresh_exact_write_authority',
    'negotiated::actual_sip_dynamic_pt126_48khz_segmented_event_retains_negotiated_clock',
    'rtp_transceiver::rtp_receiver::internal::repair_binding_tests::equivalent_telephone_event_preferences_preserve_each_negotiated_payload',
    'sdp::inspect::telephone_event_limits_tests::duplicate_mapping_or_invalid_event_suffix_never_picks_a_prefix_profile',
    'send_dtmf_emits_rfc4733_telephone_events',
    'authorized_actual_srtp_48khz_exact_segmented_events_leave_primary_audio_live',
    'owned_authority::actual_revocation_between_packets_stops_suffix_and_preserves_uncertainty',
]


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def source_inputs():
    spec = importlib.util.spec_from_file_location('previous_dtmf', ROOT / 'scripts/check-gate6-webrtc-dtmf-authority.py')
    previous = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(previous)
    inputs = previous.source_inputs()
    for name in ('scripts/check-gate6-dtmf-routing.py', 'Cargo.toml', 'Cargo.lock', 'docs/developer/telephone-events.md', 'docs/developer/dtmf-routing.md', 'scripts/check-gate6-audio-admission.py'):
        inputs[name] = digest(ROOT / name)
    for path in (ROOT / 'crates').rglob('*'):
        if path.is_file() and (path.suffix == '.rs' or path.name == 'Cargo.toml'):
            inputs[str(path.relative_to(ROOT))] = digest(path)
    return dict(sorted(inputs.items()))


def main():
    directory = ROOT / '.local/gate-6/dtmf-routing' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    before = source_inputs()
    steps = []

    def run(name, command, minimum=0, manifest=None):
        log = directory / (name + '.log')
        env = dict(os.environ)
        if manifest:
            env['CARGO_MANIFEST_DIR'] = str(pathlib.Path(manifest).parent)
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT)
        tests = re.findall(r'^test (.+?) \.\.\. ok$', log.read_text(errors='replace'), re.M)
        passed = result.returncode == 0 and len(tests) >= minimum
        binaries = {str(pathlib.Path(command[0]).relative_to(ROOT)): digest(pathlib.Path(command[0]))} if manifest else {}
        steps.append(dict(name=name, command=command, environment={'CARGO_MANIFEST_DIR': env['CARGO_MANIFEST_DIR']} if manifest else {},
                          passed=passed, exit_code=result.returncode, tests_passed=tests,
                          log=str(log.relative_to(ROOT)), log_sha256=digest(log), test_binaries_sha256=binaries))
        print(json.dumps(dict(step=name, passed=passed, tests=len(tests))), flush=True)
        return passed

    ok = run('dependencies', ['bash', 'scripts/prepare-dependencies.sh'])
    if ok:
        ok = run('format', ['bash', 'scripts/format.sh', '--check'])
    # Check precisely the Rust files recorded in the three active media patches.
    formats = {'2021': set(), '2024': set()}
    for patch, prefix in [('patches/rvoip/gate6-dtmf-routing.patch', '.deps/rvoip/'), ('patches/rvoip/gate6-media-primitives.patch', '.deps/rvoip/'),
                          ('patches/rtc-interceptor/gate6-auxiliary-nack.patch', '.deps/rtc-interceptor/'),
                          ('patches/rtc-interceptor/gate6-owned-dtmf-profile.patch', '.deps/rtc-interceptor/')]:
        for name in re.findall(r'^\+\+\+ b/(.+\.rs)$', (ROOT / patch).read_text(), re.M):
            edition = '2024' if prefix.endswith('rtc-interceptor/') or any('/' + c + '/' in '/' + name for c in ('rvoip-rtc', 'rvoip-webrtc-stack')) else '2021'
            formats[edition].add(prefix + name)
    for edition, files in formats.items():
        if ok and files:
            ok = run('upstream-format-' + edition, ['rustfmt', '--edition', edition, '--config', 'skip_children=true', '--check'] + sorted(files))
    artifacts = {}
    if ok:
        command = ['cargo', 'test', '--locked', '--offline', '-j1', '--target-dir', 'target', '--manifest-path', '.deps/rvoip/Cargo.toml']
        command += [v for package in PACKAGES for v in ('-p', package)]
        command += ['--features', 'rvoip-core/opus,rvoip-core/vcon-signing', '--lib', '--test', 'dtmf_auto_route', '--test', 'orchestrator_dispatch', '--test', 'adapter_dtmf_network', '--test', 'dtmf_wire', '--no-run', '--message-format=json']
        ok = run('joint-build', command)
        for line in (directory / 'joint-build.log').read_text(errors='replace').splitlines():
            try:
                item = json.loads(line)
            except ValueError:
                continue
            if item.get('reason') == 'compiler-artifact' and item.get('executable'):
                artifacts[item['target']['name']] = dict(executable=item['executable'], manifest=item['manifest_path'],
                                                        sha256=digest(pathlib.Path(item['executable'])))
        ok = ok and len(artifacts) == 12
        steps[-1]['passed'] = ok
    suites = [
        ('core-route', 'dtmf_auto_route', '', 8),
        ('core-dispatch', 'orchestrator_dispatch', '', 1),
        ('io-poll-fence', 'rvoip_media_core', 'dtmf_authority::poll_fence_tests::', 1),
        ('rtp-telephone', 'rvoip_rtp_core', 'telephone', 11),
        ('rtp-udp', 'rvoip_rtp_core', 'transport::udp::', 29),
        ('rtp-session', 'rvoip_rtp_core', 'session::tests::', 20),
        ('rtp-stream', 'rvoip_rtp_core', 'session::stream::', 1),
        ('rtp-loss', 'rvoip_rtp_core', 'stats::loss::', 1),
        ('rtp-jitter', 'rvoip_rtp_core', 'stats::jitter::', 1),
        ('media-statistics', 'rvoip_media_core', 'relay::controller::statistics::', 1),
        ('media-dtmf', 'rvoip_media_core', 'dtmf', 26),
        ('media-codec', 'rvoip_media_core', 'relay::controller::codec_runtime::', 4),
        ('sip-transaction', 'rvoip_sip', 'dtmf_transaction_tests', 1),
        ('sip-negotiation', 'rvoip_sip', 'telephone_event_negotiation_tests', 2),
        ('sip-sdp', 'rvoip_sip', 'adapters::media_adapter::sdp_format_tests::', 63),
        ('sip-dtmf', 'rvoip_sip', 'dtmf', 5),
        ('sip-wire', 'adapter_dtmf_network', '', 3),
        ('webrtc-media', 'rvoip_webrtc', 'media::', 40),
        ('webrtc-sdp', 'rvoip_webrtc', 'sdp::', 18),
        ('webrtc-wire', 'dtmf_wire', '', 11),
        ('stack-library', 'webrtc', '', 18),
        ('rtc-profile', 'rtc', 'owned_dtmf', 1),
        ('rtc-codec', 'rtc', 'rtp_codec::', 4),
        ('rtc-receiver', 'rtc', 'repair_binding_tests', 3),
        ('interceptor-library', 'rtc_interceptor', '', 135),
        ('core-library', 'rvoip_core', '', 398),
    ]
    for name, target, selected, minimum in suites:
        if not ok:
            break
        artifact = artifacts[target]
        ok = run(name, [artifact['executable']] + ([selected] if selected else []) + ['--test-threads=1'], minimum, artifact['manifest'])
        ok = ok and digest(pathlib.Path(artifact['executable'])) == artifact['sha256']
        steps[-1]['passed'] = ok
    if ok:
        command = ['cargo', 'test', '--locked', '--offline', '-j1', '--target-dir', 'target', '--manifest-path', '.deps/rvoip/Cargo.toml', '-p', 'rvoip-uctp', '--test', 'dtmf_bridge', '--no-run', '--message-format=json']
        ok = run('public-audio-build', command)
        extra = None
        for line in (directory / 'public-audio-build.log').read_text(errors='replace').splitlines():
            try:
                item = json.loads(line)
            except ValueError:
                continue
            if item.get('reason') == 'compiler-artifact' and item.get('executable') and item['target']['name'] == 'dtmf_bridge':
                extra = dict(executable=item['executable'], manifest=item['manifest_path'], sha256=digest(pathlib.Path(item['executable'])))
        ok = ok and extra is not None
        steps[-1]['passed'] = ok
        if ok:
            artifacts['dtmf_bridge'] = extra
            ok = run('public-audio-boundary', [extra['executable'], '--test-threads=1'], 2, extra['manifest'])
            ok = ok and digest(pathlib.Path(extra['executable'])) == extra['sha256']
            steps[-1]['passed'] = ok
    if ok:
        ok = run('server-integration', ['cargo', 'check', '--locked', '--offline', '-j1', '-p', 'connect-server'])
    if ok:
        ok = run('progress', ['python3', 'scripts/check-gate6-progress.py'])
    if ok:
        ok = run('diff', ['git', 'diff', '--check'])
    observed = {test for step in steps for test in step['tests_passed']}
    missing = sorted(set(REQUIRED) - observed)
    unchanged = before == source_inputs()
    report = dict(task='G6-024', implementation_status='in-progress', full_task_qualification=False,
                  qualification='Core-owned source/recipient and original-operation RFC4733 routing under independent fresh per-write signal permission, exact synchronous engine fences through actual IO polls, bounded caller-owned schedules and explicit prefix uncertainty. Raw bridge observations have no automatic effect and target-only Core dispatch is removed. Real local UDP/engine ownership, deny/revoke/retire/cancel/drop/deadline/contention/isolation cases, controlled pending-then-ready IO fence, affected current RTP/SIP/WebRTC/audio regressions and locked server compile check. Local macOS arm64 only.',
                  not_qualified=['Actual native sealed-source/egress and SIP/WebRTC source composition, plus real SIP INFO per-write and retransmission authority. INFO refuses explicitly in this Core route and cannot fall back to RTP.',
                                 'Decoded primary readiness and remaining malformed/late/oversized/feedback/codec profiles; full G6-024 closure.',
                                 'Production Store/HTTP/SDK/journal dispatch; Linux, remote/native/browser/provider and full Gate 6 acceptance.'],
                  executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
                  rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(), source_inputs_sha256=before,
                  sources_unchanged=unchanged, artifacts=artifacts, steps=steps,
                  runtime_tests=sum(len(step['tests_passed']) for step in steps), runtime_unique_tests=len(observed), required_cases=REQUIRED,
                  missing_required_cases=missing, passed=ok and unchanged and not missing and all(step['passed'] for step in steps))
    path = ROOT / 'docs/evidence/gate-6/dtmf-routing.json'
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(passed=report['passed'], tests=report['runtime_tests'], missing=missing,
                          report=str(path.relative_to(ROOT)))), flush=True)
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
