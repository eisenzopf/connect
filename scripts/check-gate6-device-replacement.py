#!/usr/bin/env python3
"""Source-bound same-participant make-before-break device replacement qualification."""
import datetime
import importlib.util
import json
import pathlib
import platform
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('handoff', ROOT / 'scripts/check-gate6-connection-handoff.py')
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
digest = base.digest
UPSTREAM = ROOT / '.deps/rvoip'
CASES = {
    'crates/foundation/rvoip-core/src/participant_transitions/tests/replacement.rs': 'participant_transitions::tests::replacement::',
    'crates/foundation/rvoip-core/src/staged_streams/tests/replacement_activation.rs': 'staged_streams::tests::replacement_activation::',
    'crates/foundation/rvoip-core/src/staged_streams/tests/private_consult/replacement.rs': 'staged_streams::tests::private_consult::replacement::',
    'crates/uctp/rvoip-uctp/tests/support/canonical_media/guarded_mixer/replacement.rs': 'peer_native::canonical_media::guarded_mixer::replacement::',
    'crates/uctp/rvoip-uctp/tests/support/canonical_media/guarded_mixer/replacement/adapter.rs': 'peer_native::canonical_media::guarded_mixer::replacement::adapter::',
}
PATCH_FILES = re.findall(r'^diff --git a/(.*?) b/', (ROOT / 'patches/rvoip/gate6-device-replacement.patch').read_text(), re.M)
FILES = sorted(set(base.FILES + PATCH_FILES + list(CASES)))

def test_names(path):
    return re.findall(r'#\[(?:tokio::)?test(?:[^\]]*)\]\s*(?:async\s+)?fn\s+(\w+)',path.read_text())



def sources():
    values = base.sources()
    for name in ['scripts/check-gate6-device-replacement.py', 'contracts/device-replacement.md',
                 'patches/rvoip/gate6-device-replacement.patch', 'dependencies/source-lock.json',
                 'docs/evidence/gate-6/connection-handoff.json']:
        values[name] = digest(ROOT / name)
    for name in FILES:
        values['.deps/rvoip/' + name] = digest(UPSTREAM / name)
    return values


def main():
    assert platform.system() == 'Darwin' and platform.machine() == 'arm64'
    directory = ROOT / '.local/gate-6/connection-handoff' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    before = sources()
    cargo = ['cargo', 'test', '--locked', '--offline', '-j1', '--target-dir', 'target', '--manifest-path', '.deps/rvoip/Cargo.toml']
    suites = ['participant_control', 'participant_roles', 'session_lifecycle', 'peer_admission',
              'peer_sessions', 'stream_readiness', 'per_connection_control', 'conversation_lifecycle']
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('upstream-format', ['rustfmt', '--edition', '2021', '--config', 'skip_children=true', '--check'] + ['.deps/rvoip/' + name for name in FILES], 0),
        ('core-library', cargo + ['-p', 'rvoip-core', '--features', 'opus', '--lib', '--', '--test-threads=1'], 395),
        ('participant-session-peer', cargo + ['-p', 'rvoip-core', '--features', 'opus'] + [arg for suite in suites for arg in ('--test', suite)] + ['--', '--test-threads=1'], 64),
        ('canonical-peer', cargo + ['-p', 'rvoip-uctp', '--test', 'v2_peer_admission', '--', '--test-threads=1'], 122),
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
    historical = ROOT / 'docs/evidence/gate-6/connection-handoff.json'
    old = json.loads(historical.read_text())
    assert old['passed'] and len(old['required_tests']) == 191
    required = set(old['required_tests'])
    new_cases = {prefix + name for path, prefix in CASES.items() for name in test_names(UPSTREAM / path)}
    assert len(new_cases) >= 19, len(new_cases)
    required.update(new_cases)
    missing = sorted(required - {name for step in steps for name in step['tests_passed']})
    unchanged = before == sources()
    report = dict(format=1, task='G6-023',
        qualification='Same-participant device replacement: bounded owned preparation and exact reserved native/adapter routes; separate current operator, media, endpoint-signal and local-audio permission; actual isolated decoded input and successful guarded output; configured media-only or answered attributed DTMF confirmation; one ParticipantSession/Session/scene commit with inherited admitted slot at full logical capacity; stable retained remote/main Session; exact authorized history and no rollback on uncertain cleanup. Local macOS arm64, real signed TLS QUIC/WebTransport and controlled encoded-IO browser/telephone adapter SPI. General browser/PSTN/provider networks remain later profile qualification.',
        executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
        source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        source_inputs_sha256=before, sources_unchanged=unchanged,
        source_lock_patches=len(json.loads((ROOT / 'dependencies/source-lock.json').read_text())['patches']),
        steps=steps, required_tests=sorted(required), missing_required_tests=missing,
        historical_connection_handoff=dict(path=str(historical.relative_to(ROOT)), sha256=digest(historical), disposition='Historical sealed G6-022 snapshot; preserved original sources and executable archives are separate from this current-source run.'),
        not_qualified=[
            'Local graph/model commit and FIFO hint are not durable Connect receipts, public Connection.ready or Participant presence. The host installs actual current grants and owns its authoritative journal.',
            'Controlled browser/telephone SPI cases use actual guarded encoded audio and concrete endpoint callback attribution; they do not claim live browser, PSTN/provider-network, general wire DTMF or public workflow installation. G6-024 and later profile tasks remain required.',
            'Cleanup observations cover explicitly observed local resources; unknown remote/provider outcomes do not cause rollback, automatic origination or redial.',
            'Production Store/HTTP/SDK policies, durable operation/reconciliation and public Session/history projection remain G6-037/G6-038 and their dependent work.',
            'Full release/performance, Linux x86_64, remote/native/browser qualification, bounded G6-093 named rehearsals, Gate 6 acceptance and separate publication remain required.',
        ],
        exploratory_attempts=[dict(private_log=str(path.relative_to(ROOT)), sha256=digest(path), disposition='Unqualified exploratory compilation or behavior attempt') for path in sorted((ROOT / '.local/gate-6').glob('device-replacement-*-exploratory.log'))],
        passed=unchanged and len(steps) == len(commands) and len(required) >= 210 and not missing and all(s['passed'] for s in steps))
    report['implementation_status'] = 'verified-upstream-G6-023-scope' if report['passed'] else 'in-progress'
    destination = ROOT / 'docs/evidence/gate-6/device-replacement.json'
    if destination.exists():
        previous = json.loads(destination.read_text())
        report['previous_attempts'] = previous.get('previous_attempts', []) + [dict(sha256=digest(destination), **{k: previous[k] for k in ('passed', 'executed_at', 'source_inputs_sha256', 'steps')})]
    destination.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(passed=report['passed'], missing=missing)), flush=True)
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
