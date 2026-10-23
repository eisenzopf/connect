#!/usr/bin/env python3
"""Qualify owned WebRTC DTMF IO; this is a component of unfinished G6-024."""
import datetime
import hashlib
import importlib.util
import json
import pathlib
import platform
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
REQUIRED = [
    'media_stream::track_local::owned_dtmf::socket_custody_tests::cancellation_wins_when_pending_socket_wakes_ready_in_same_turn',
    'media_stream::track_local::owned_dtmf::socket_custody_tests::driver_cancel_aborts_pending_socket_before_permission_release',
    'media_stream::track_local::owned_dtmf::socket_custody_tests::driver_deadline_aborts_pending_socket_before_permission_release',
    'media_stream::track_local::owned_dtmf::socket_custody_tests::driver_drop_aborts_pending_socket_before_permission_release',
    'media_stream::track_local::owned_dtmf::tests::concurrent_cancel_and_submit_have_one_monotonic_winner',
    'peer_connection::configuration::interceptor_registry::owned_dtmf_profile::tests::custom_wrappers_do_not_inherit_reviewed_delivery_contract',
    'media::outbound::tests::actual_driver_denial_does_not_reserve_primary_audio_sequence_or_time',
    'authorized_actual_srtp_48khz_exact_segmented_events_leave_primary_audio_live',
    'owned_authority::actual_srtp_requires_fresh_permission_for_every_exact_event_packet',
    'owned_authority::actual_revocation_between_packets_stops_suffix_and_preserves_uncertainty',
    'owned_authority::completed_first_digit_then_gap_revocation_remains_an_uncertain_whole_intent',
    'owned_authority::dropping_authorized_sequence_stops_actual_wire_and_later_digits',
    'nack::responder::tests::negotiated_audio_nack_does_not_replay_telephone_event_or_unknown_payloads_on_its_ssrc',
    'relay::controller::dtmf_transmitter::authority_tests::completed_first_digit_then_gap_revocation_is_an_uncertain_sequence',
    'actual_rfc4733_adapter_preserves_observed_duration_and_live_connection',
]


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def source_inputs():
    paths = [ROOT / n for n in ('scripts/check-gate6-webrtc-dtmf-authority.py',
             'scripts/check-gate6-media-feedback.py', 'scripts/prepare-dependencies.sh',
             'scripts/prepare-dependencies.py', 'scripts/lib/prepared_archive.py', 'scripts/format.sh',
             'dependencies/source-lock.json', 'contracts/native-signals-profile.md')]
    paths += [p for p in (ROOT / '.deps').rglob('*') if p.is_file() and
              (p.suffix == '.rs' or p.name in ('Cargo.toml', 'Cargo.lock'))]
    paths += list((ROOT / 'patches').rglob('*.patch'))
    return {str(p.relative_to(ROOT)): digest(p) for p in sorted(set(paths))}


def main():
    directory = ROOT / '.local/gate-6/webrtc-dtmf-authority' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    before = source_inputs()
    cargo = ['cargo', 'test', '--locked', '--offline', '-j1', '--target-dir', 'target', '--manifest-path', '.deps/rvoip/Cargo.toml']
    base = '.deps/rvoip/crates/webrtc/'
    formats = {
        '2024': [base + x for x in ('rvoip-rtc/src/peer_connection/configuration/owned_dtmf_profile.rs',
                 'rvoip-rtc/src/peer_connection/configuration/interceptor_registry.rs',
                 'rvoip-rtc/src/peer_connection/mod.rs', 'rvoip-rtc/src/rtp_transceiver/rtp_sender/mod.rs',
                 'rvoip-webrtc-stack/src/media_stream/track_local/owned_dtmf.rs',
                 'rvoip-webrtc-stack/src/media_stream/track_local/static_rtp.rs',
                 'rvoip-webrtc-stack/src/media_stream/track_local/mod.rs',
                 'rvoip-webrtc-stack/src/peer_connection/driver.rs')] + ['.deps/rtc-interceptor/src/lib.rs'],
        '2021': [base + x for x in ('rvoip-webrtc/src/media/dtmf.rs', 'rvoip-webrtc/src/media/outbound.rs',
                 'rvoip-webrtc/src/errors.rs', 'rvoip-webrtc/src/peer/builder.rs', 'rvoip-webrtc/tests/dtmf_wire.rs')]
                + ['.deps/rvoip/crates/media/media-core/src/dtmf_authority.rs'],
    }
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
    ]
    commands += [('upstream-format-' + edition, ['rustfmt', '--edition', edition, '--config', 'skip_children=true', '--check'] + paths, 0)
                 for edition, paths in formats.items()]
    commands += [
        ('stack-library', cargo + ['-p', 'rvoip-webrtc-stack', '--lib', '--', '--test-threads=1'], 18),
        ('rtc-profile', cargo + ['-p', 'rvoip-rtc', '--lib', 'owned_dtmf', '--', '--test-threads=1'], 1),
        ('interceptor-library', cargo + ['-p', 'rtc-interceptor', '--lib', '--', '--test-threads=1'], 135),
        ('shared-and-udp', cargo + ['-p', 'rvoip-media-core', '--lib', 'dtmf', '--', '--test-threads=1'], 24),
        ('webrtc-media', cargo + ['-p', 'rvoip-webrtc', '--lib', 'media::', '--', '--test-threads=1'], 1),
        ('webrtc-wire', cargo + ['-p', 'rvoip-webrtc', '--test', 'dtmf_wire', '--', '--test-threads=1'], 11),
        ('sip-network', cargo + ['-p', 'rvoip-sip', '--features', 'rvoip-core/opus', '--test', 'adapter_dtmf_network', '--', '--test-threads=1'], 1),
        ('progress', ['python3', 'scripts/check-gate6-progress.py'], 0),
        ('diff', ['git', 'diff', '--check'], 0),
    ]
    steps = []
    for name, command, minimum in commands:
        log = directory / (name + '.log')
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, stdout=output, stderr=subprocess.STDOUT)
        raw = log.read_text(errors='replace')
        tests = re.findall(r'^test (.+?) \.\.\. ok$', raw, re.M)
        binaries = {p: digest(ROOT / p) for p in re.findall(r'Running [^\n]+\((target/debug/deps/[^\)]+)\)', raw)}
        passed = result.returncode == 0 and len(tests) >= minimum
        steps.append(dict(name=name, command=command, passed=passed, exit_code=result.returncode,
                          tests_passed=tests, log=str(log.relative_to(ROOT)), log_sha256=digest(log),
                          test_binaries_sha256=binaries))
        print(json.dumps(dict(step=name, passed=passed, tests=len(tests))), flush=True)
        if not passed:
            break
    observed = {t for s in steps for t in s['tests_passed']}
    missing = sorted(set(REQUIRED) - observed)
    report = dict(task='G6-024', implementation_status='in-progress', full_task_qualification=False,
                  qualification='Owned per-write WebRTC DTMF permission through actual SRTP/UDP submission on the reviewed default interceptor profile, fresh real read guards, exact 121ms and negotiated 48kHz segmented events, retained primary audio, default denial, revocation, completed-prefix uncertainty, caller cancellation and pending-socket custody/drop ordering. All behavior is local macOS arm64 fixture evidence.',
                  not_qualified=['General Core/native/SIP gateway source, method, stable-operation and independent current source/recipient policy installation.',
                                 'Remaining frozen media, malformed/late/oversized/feedback/decoded profiles; full G6-024 closure.',
                                 'Production Store/HTTP/SDK/journal dispatch and Linux, remote/native/browser/provider/full Gate 6 acceptance.'],
                  executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
                  rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(),
                  source_inputs_sha256=before, sources_unchanged=before == source_inputs(), steps=steps,
                  runtime_tests=sum(len(s['tests_passed']) for s in steps), required_cases=REQUIRED,
                  missing_required_cases=missing, passed=len(steps) == len(commands) and before == source_inputs() and not missing and all(s['passed'] for s in steps))
    path = ROOT / 'docs/evidence/gate-6/webrtc-dtmf-authority.json'
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(passed=report['passed'], tests=report['runtime_tests'], report=str(path.relative_to(ROOT)))), flush=True)
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
