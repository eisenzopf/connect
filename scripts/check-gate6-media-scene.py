#!/usr/bin/env python3
"""Source-bound local owned general media scene qualification."""
import datetime
import importlib.util
import json
import pathlib
import platform
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('previous', ROOT / 'scripts/check-gate6-direct-subscriptions.py')
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
digest = base.digest
UPSTREAM = ROOT / '.deps/rvoip'
FILES = [
    'crates/foundation/rvoip-core/src/staged_streams/media_scene.rs',
    'crates/foundation/rvoip-core/src/staged_streams/scene_consumer.rs',
    'crates/foundation/rvoip-core/src/staged_streams/tests/media_scene.rs',
    'crates/foundation/rvoip-core/src/adapter.rs',
    'crates/foundation/rvoip-core/src/conference.rs',
    'crates/foundation/rvoip-core/src/orchestrator/participant_control.rs',
    'crates/foundation/rvoip-core/src/staged_streams.rs',
    'crates/foundation/rvoip-core/src/staged_streams/audio_consumer.rs',
    'crates/foundation/rvoip-core/src/staged_streams/guarded_audio.rs',
    'crates/foundation/rvoip-core/src/staged_streams/tests.rs',
    'crates/foundation/rvoip-core/src/stream_lifecycle.rs',
    'crates/foundation/rvoip-core/src/staged_streams/audio_subscriptions.rs',
    'crates/foundation/rvoip-core/src/staged_streams/direct_subscriptions.rs',
    'crates/foundation/rvoip-core/src/staged_streams/tests/direct_subscriptions.rs',
    'crates/uctp/rvoip-uctp/tests/staged_media.rs',
    'crates/uctp/rvoip-uctp/src/staged_media.rs',
    'crates/foundation/rvoip-core/src/staged_streams/tests/guarded_mixer/subscriptions.rs',
    'crates/uctp/rvoip-uctp/tests/support/media_control.rs',
    'crates/foundation/rvoip-core/src/staged_streams/tests/guarded_mixer.rs',
    'crates/uctp/rvoip-uctp/src/v2/native.rs',
    'crates/uctp/rvoip-uctp/tests/support/canonical_media.rs',
    'crates/uctp/rvoip-uctp/tests/support/canonical_media/guarded_mixer.rs',
]


def sources():
    values = base.sources()
    for name in ['scripts/check-gate6-media-scene.py', 'docs/developer/media-scene.md',
                 'patches/rvoip/gate6-media-scene.patch', 'dependencies/source-lock.json']:
        values[name] = digest(ROOT / name)
    for name in FILES:
        values['.deps/rvoip/' + name] = digest(UPSTREAM / name)
    return values


def test_names(path):
    return re.findall(r'#\[(?:tokio::)?test(?:[^\]]*)\]\s*(?:async\s+)?fn\s+(\w+)', path.read_text())


def main():
    assert platform.system() == 'Darwin' and platform.machine() == 'arm64'
    directory = ROOT / '.local/gate-6/media-scene' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    before = sources()
    cargo = ['cargo', 'test', '--locked', '--offline', '-j1', '--target-dir', 'target',
             '--manifest-path', '.deps/rvoip/Cargo.toml']
    suites = ['participant_control', 'participant_roles', 'session_lifecycle', 'peer_admission',
              'peer_sessions', 'stream_readiness', 'per_connection_control', 'conversation_lifecycle']
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('upstream-format', ['rustfmt', '--edition', '2021', '--config', 'skip_children=true', '--check'] +
         ['.deps/rvoip/' + name for name in FILES], 0),
        ('core-library', cargo + ['-p', 'rvoip-core', '--features', 'opus', '--lib', '--', '--test-threads=1'], 305),
        ('participant-session-peer', cargo + ['-p', 'rvoip-core', '--features', 'opus'] +
         [arg for suite in suites for arg in ('--test', suite)] + ['--', '--test-threads=1'], 64),
        ('canonical-peer', cargo + ['-p', 'rvoip-uctp', '--test', 'v2_peer_admission', '--', '--test-threads=1'], 53),
        ('staged-rtp-library', cargo + ['-p', 'rvoip-uctp', '--lib', 'staged_media::tests', '--', '--test-threads=1'], 1),
        ('staged-rtp', cargo + ['-p', 'rvoip-uctp', '--test', 'staged_media', '--', '--test-threads=1'], 9),
        ('contracts', [str(ROOT / '.local/gate-6/verification-venv/bin/python'),
                       'scripts/check-gate6-contracts.py', '--output', str(directory / 'contracts.json')], 0),
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
        binaries = {path: digest(ROOT / path) for path in re.findall(r'Running [^\n]+\((target/debug/deps/[^\)]+)\)', raw)}
        step = dict(name=name, command=command, exit_code=result.returncode, tests_passed=tests,
                    private_log=str(log.relative_to(ROOT)), log_sha256=digest(log),
                    test_binaries_sha256=binaries, passed=result.returncode == 0 and len(tests) >= minimum)
        steps.append(step)
        print(json.dumps(dict(step=name, passed=step['passed'], tests=len(tests))), flush=True)
        if not step['passed']:
            break
    required = {'staged_streams::tests::direct_subscriptions::' + name for name in
                test_names(UPSTREAM / 'crates/foundation/rvoip-core/src/staged_streams/tests/direct_subscriptions.rs')}
    required.add('peer_native::canonical_media::actual_signed_quic_and_wt_direct_video_screen_selection_removal_and_reoffer')
    required.update('staged_streams::tests::' + name for name in ['data_backpressure_preserves_retry_and_sequence', 'stale_media_expires_while_ordered_data_remains_explicitly_buffered'])
    required.add('actual_rtp_video_forwarding_preserves_marker_and_rewrites_destination_payload_type')
    required.update('staged_media::tests::' + name for name in test_names(UPSTREAM / 'crates/uctp/rvoip-uctp/src/staged_media.rs'))
    required.update('staged_streams::tests::media_scene::' + name for name in test_names(UPSTREAM / 'crates/foundation/rvoip-core/src/staged_streams/tests/media_scene.rs'))
    required.update('peer_native::canonical_media::guarded_mixer::' + name for name in test_names(UPSTREAM / 'crates/uctp/rvoip-uctp/tests/support/canonical_media/guarded_mixer.rs') if name.startswith('actual_tls_owned_scene_'))
    missing = sorted(required - {name for step in steps for name in step['tests_passed']})
    after = sources()
    historical = ROOT / 'docs/evidence/gate-6/direct-subscriptions.json'
    report = dict(
        format=1, task='G6-020', implementation_status='in-progress',
        qualification='Owned general media scene with non-audio admission, automatic exact guarded direct fan-out, bounded per-listener ordered-data retry and optional logical audio reselection; local macOS arm64',
        executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
        source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        source_inputs_sha256=before, sources_unchanged=before == after,
        source_lock_patches=len(json.loads((ROOT / 'dependencies/source-lock.json').read_text())['patches']),
        steps=steps, required_tests=sorted(required), missing_required_tests=missing,
        historical_direct_subscriptions=dict(path=str(historical.relative_to(ROOT)), sha256=digest(historical),
                                              disposition='Unchanged historical source snapshot; not current-source qualification'),
        not_qualified=[
            'Production Connect media/participant policies and public canonical offer/answer/ready, confirmation/presence and automatic refresh/step-up installation. Signed local native allocation composes with controlled policy/readiness and the trusted participant SPI.',
            'Non-audio Participant selectors, native reliable-data transport, public durable subscription handlers, guarded SIP/WebRTC admission and browser media; no video decode/render evidence. Audio Participant selectors for owners without currently selected audio are not qualified.',
            'General durable Session projection G6-038, complete G6-037 migration, full workspace/package/release/performance, remote/live-provider profiles, Gate 6 acceptance and publication.',
        ],
        exploratory_attempts=[dict(private_log=str(path.relative_to(ROOT)), sha256=digest(path),
                                   disposition='Unqualified exploratory compilation or behavior attempt')
                              for path in sorted((ROOT / '.local/gate-6').glob('media-scene-*-exploratory.log'))],
        passed=before == after and len(steps) == len(commands) and len(required) == 28 and not missing and all(s['passed'] for s in steps),
    )
    destination = ROOT / 'docs/evidence/gate-6/media-scene.json'
    if destination.exists():
        old = json.loads(destination.read_text())
        report['previous_attempts'] = old.get('previous_attempts', []) + [dict(sha256=digest(destination), **{k: old[k] for k in ('passed', 'executed_at', 'source_inputs_sha256', 'steps')})]
    destination.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(passed=report['passed'], missing=missing)), flush=True)
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
