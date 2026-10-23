#!/usr/bin/env python3
"""Source-bound private consultation, hold, isolation and recovery qualification."""
import datetime
import importlib.util
import json
import pathlib
import platform
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('atomic', ROOT / 'scripts/check-gate6-atomic-subscriptions.py')
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
digest = base.digest
UPSTREAM = ROOT / '.deps/rvoip'
CORE_TEST = 'crates/foundation/rvoip-core/src/staged_streams/tests/private_consult.rs'
NATIVE_TEST = 'crates/uctp/rvoip-uctp/tests/support/canonical_media/native_offers/audiences.rs'
FILES = sorted(set(base.FILES + [CORE_TEST, NATIVE_TEST,
    'crates/foundation/rvoip-core/src/staged_streams/audience.rs',
    'crates/foundation/rvoip-core/src/staged_streams/consultation.rs',
    'crates/foundation/rvoip-core/src/staged_streams/audio_consumer.rs',
    'crates/foundation/rvoip-core-traits/src/peer_switch.rs']))


def sources():
    values = base.sources()
    for name in ['scripts/check-gate6-private-consult.py', 'contracts/private-consultation.md',
                 'patches/rvoip/gate6-private-consult.patch', 'dependencies/source-lock.json',
                 'docs/evidence/gate-6/atomic-subscriptions.json']:
        values[name] = digest(ROOT / name)
    for name in FILES:
        values['.deps/rvoip/' + name] = digest(UPSTREAM / name)
    return values


def main():
    assert platform.system() == 'Darwin' and platform.machine() == 'arm64'
    directory = ROOT / '.local/gate-6/private-consult' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    before = sources()
    cargo = ['cargo', 'test', '--locked', '--offline', '-j1', '--target-dir', 'target', '--manifest-path', '.deps/rvoip/Cargo.toml']
    suites = ['participant_control', 'participant_roles', 'session_lifecycle', 'peer_admission',
              'peer_sessions', 'stream_readiness', 'per_connection_control', 'conversation_lifecycle']
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('upstream-format', ['rustfmt', '--edition', '2021', '--config', 'skip_children=true', '--check'] + ['.deps/rvoip/' + name for name in FILES], 0),
        ('core-library', cargo + ['-p', 'rvoip-core', '--features', 'opus', '--lib', '--', '--test-threads=1'], 365),
        ('participant-session-peer', cargo + ['-p', 'rvoip-core', '--features', 'opus'] + [arg for suite in suites for arg in ('--test', suite)] + ['--', '--test-threads=1'], 64),
        ('canonical-peer', cargo + ['-p', 'rvoip-uctp', '--test', 'v2_peer_admission', '--', '--test-threads=1'], 112),
        ('offer-answer-library', cargo + ['-p', 'rvoip-uctp', '--lib', 'v2::auth_dispatch::offer_answer::tests', '--', '--test-threads=1'], 4),
        ('native-offer-library', cargo + ['-p', 'rvoip-uctp', '--lib', 'v2::native_negotiation', '--', '--test-threads=1'], 9),
        ('native-writer-library', cargo + ['-p', 'rvoip-uctp', '--lib', 'v2::auth_peer::exchange::tests', '--', '--test-threads=1'], 6),
        ('staged-rtp-library', cargo + ['-p', 'rvoip-uctp', '--lib', 'staged_media::tests', '--', '--test-threads=1'], 1),
        ('staged-rtp', cargo + ['-p', 'rvoip-uctp', '--test', 'staged_media', '--', '--test-threads=1'], 9),
        ('contracts', [str(ROOT / '.local/gate-6/verification-venv/bin/python'), 'scripts/check-gate6-contracts.py', '--output', str(directory / 'contracts.json')], 0),
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
    historical = ROOT / 'docs/evidence/gate-6/atomic-subscriptions.json'
    old = json.loads(historical.read_text())
    assert old['passed'] and len(old['required_tests']) == 158
    required = set(old['required_tests'])
    required.update('staged_streams::tests::private_consult::' + name for name in base.test_names(UPSTREAM / CORE_TEST))
    required.update('peer_native::canonical_media::native_offers::audiences::' + name for name in base.test_names(UPSTREAM / NATIVE_TEST))
    missing = sorted(required - {name for step in steps for name in step['tests_passed']})
    unchanged = before == sources()
    report = dict(format=1, task='G6-021',
        qualification='Upstream retained private audio consultation and remote hold; actual muted private preparation, exact Connection quarantine, atomic multi-receiver audience and source gates, decoded distinct-owner speaker isolation, timeout/cancellation/hangup restoration and sealed native confirmation carry-forward. Isolated controlled policy/adapter fixtures and local TLS QUIC/WebTransport. Other approved Gate 6 profiles remain required.',
        executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
        source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        source_inputs_sha256=before, sources_unchanged=unchanged,
        source_lock_patches=len(json.loads((ROOT / 'dependencies/source-lock.json').read_text())['patches']),
        steps=steps, required_tests=sorted(required), missing_required_tests=missing,
        historical_atomic_subscriptions=dict(path=str(historical.relative_to(ROOT)), sha256=digest(historical), disposition='Historical sealed G6-020 snapshot; preserved original sources and executable archives are separate from this current-source run.'),
        not_qualified=[
            'Media graph outcomes, preparation observations and sealed native commit proofs are not durable Connect operation receipts, Connection.ready or Participant presence. The host owns its current domain authority and journal.',
            'Decoded private speech uses distinct-owner Core fixtures with actual G.711 encode/decode. The native audience fixture uses two endpoints of one Participant, retains MCU self-echo suppression and qualifies existing signed resource confirmation only.',
            'Prompts, business approval and provider Context bodies stay outside the engine. Production Connect private message/history/context projections, Store policy and HTTP/SDK dispatch belong to their assigned later tasks. External provider cleanup is separate and no live-provider behavior is claimed.',
            'Other transports, full workspace/package/release/performance, Linux x86_64 and remote/native/browser qualification, bounded G6-093 named rehearsals, Gate 6 acceptance and publication remain required.',
        ],
        exploratory_attempts=[dict(private_log=str(path.relative_to(ROOT)), sha256=digest(path), disposition='Unqualified exploratory compilation or behavior attempt') for path in sorted((ROOT / '.local/gate-6').glob('private-consult-*-exploratory.log'))],
        passed=unchanged and len(steps) == len(commands) and len(required) >= 170 and not missing and all(s['passed'] for s in steps))
    report['implementation_status'] = 'verified-upstream-G6-021-scope' if report['passed'] else 'in-progress'
    destination = ROOT / 'docs/evidence/gate-6/private-consultation.json'
    if destination.exists():
        previous = json.loads(destination.read_text())
        report['previous_attempts'] = previous.get('previous_attempts', []) + [dict(sha256=digest(destination), **{k: previous[k] for k in ('passed', 'executed_at', 'source_inputs_sha256', 'steps')})]
    destination.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(passed=report['passed'], missing=missing)), flush=True)
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
