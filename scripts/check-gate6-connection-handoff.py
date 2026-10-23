#!/usr/bin/env python3
"""Source-bound managed participant handoff, speaking roles and retained bridge qualification."""
import datetime
import importlib.util
import json
import pathlib
import platform
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('consult', ROOT / 'scripts/check-gate6-private-consult.py')
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
digest = base.digest
UPSTREAM = ROOT / '.deps/rvoip'
CORE_TEST = 'crates/foundation/rvoip-core/src/staged_streams/tests/private_consult.rs'
NATIVE_TEST = 'crates/uctp/rvoip-uctp/tests/support/canonical_media/guarded_mixer/handoff.rs'
MODEL_TEST = 'crates/foundation/rvoip-core/src/participant_transitions/tests.rs'
FILES = sorted(set(base.FILES + [CORE_TEST, NATIVE_TEST, MODEL_TEST,
    'crates/foundation/rvoip-core/src/staged_streams/handoff.rs',
    'crates/foundation/rvoip-core/tests/participant_control.rs',
    'crates/foundation/rvoip-core/src/participant_transitions.rs',
    'crates/foundation/rvoip-core/src/participant_transitions/handoff.rs',
    'crates/foundation/rvoip-core/src/orchestrator/participant_control/handoff.rs',
    'crates/uctp/rvoip-uctp/tests/support/canonical_media/guarded_mixer.rs']))



def sources():
    values = base.sources()
    for name in ['scripts/check-gate6-connection-handoff.py', 'contracts/connection-handoff.md',
                 'patches/rvoip/gate6-connection-handoff.patch', 'dependencies/source-lock.json',
                 'docs/evidence/gate-6/private-consultation.json']:
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
        ('core-library', cargo + ['-p', 'rvoip-core', '--features', 'opus', '--lib', '--', '--test-threads=1'], 382),
        ('participant-session-peer', cargo + ['-p', 'rvoip-core', '--features', 'opus'] + [arg for suite in suites for arg in ('--test', suite)] + ['--', '--test-threads=1'], 64),
        ('canonical-peer', cargo + ['-p', 'rvoip-uctp', '--test', 'v2_peer_admission', '--', '--test-threads=1'], 116),
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
    historical = ROOT / 'docs/evidence/gate-6/private-consultation.json'
    old = json.loads(historical.read_text())
    assert old['passed'] and len(old['required_tests']) == 170
    required = set(old['required_tests'])
    required.update('staged_streams::tests::private_consult::' + name for name in base.base.test_names(UPSTREAM / CORE_TEST) if name.startswith('handoff_'))
    required.update('peer_native::canonical_media::guarded_mixer::handoff::' + name for name in base.base.test_names(UPSTREAM / NATIVE_TEST))
    required.update('participant_transitions::tests::' + name for name in base.base.test_names(UPSTREAM / MODEL_TEST) if name.startswith('handoff_model_'))
    missing = sorted(required - {name for step in steps for name in step['tests_passed']})
    unchanged = before == sources()
    report = dict(format=1, task='G6-022',
        qualification='Upstream readiness-gated participant handoff: actual decoded input and guarded output evidence, current whole-handoff media and operator/role policy, one managed participant/model/scene commit, observer or leave across outgoing connections, retained remote/main Session IDs, exact historical local replay, stale/competing/denied/cancelled/terminated recovery, and truthful local cleanup uncertainty. Isolated policy/storage fixtures and real signed-authenticated distinct TLS QUIC/WebTransport peers with controlled media negotiation; other approved Gate 6 profiles remain required.',
        executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
        source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        source_inputs_sha256=before, sources_unchanged=unchanged,
        source_lock_patches=len(json.loads((ROOT / 'dependencies/source-lock.json').read_text())['patches']),
        steps=steps, required_tests=sorted(required), missing_required_tests=missing,
        historical_private_consultation=dict(path=str(historical.relative_to(ROOT)), sha256=digest(historical), disposition='Historical sealed G6-021 snapshot; preserved original sources and executable archives are separate from this current-source run.'),
        not_qualified=[
            'The graph/model result and FIFO hint are local committed facts, not durable Connect operation receipts, public connection.ready or Participant presence. The host installs actual current grants and owns its authoritative journal.',
            'Decoded Core cases include actual Human/Ai kinds and distinct owners in both directions. Managed TLS cases use distinct authenticated human identities, actual codec IO and controlled domain/negotiation storage; they do not claim public workflow installation or live providers.',
            'Cleanup confirmation covers only explicitly observed local resources. Remote/provider effects remain pending or unknown where no acknowledgment was observed; no rollback or redial is inferred.',
            'Conversation-wide durable role projection, production Store/HTTP/SDK operation/reconciliation and public history/event privacy remain their assigned later Gate 6 tasks. Local Session roles and physical speaking roles are verified together here.',
            'Full release/performance, Linux x86_64 and remote/native/browser qualification, G6-093 bounded named rehearsals, Gate 6 acceptance and separate publication remain required.',
        ],
        exploratory_attempts=[dict(private_log=str(path.relative_to(ROOT)), sha256=digest(path), disposition='Unqualified exploratory compilation or behavior attempt') for path in sorted((ROOT / '.local/gate-6').glob('connection-handoff-*-exploratory.log'))],
        passed=unchanged and len(steps) == len(commands) and len(required) >= 191 and not missing and all(s['passed'] for s in steps))
    report['implementation_status'] = 'verified-upstream-G6-022-scope' if report['passed'] else 'in-progress'
    destination = ROOT / 'docs/evidence/gate-6/connection-handoff.json'
    if destination.exists():
        previous = json.loads(destination.read_text())
        report['previous_attempts'] = previous.get('previous_attempts', []) + [dict(sha256=digest(destination), **{k: previous[k] for k in ('passed', 'executed_at', 'source_inputs_sha256', 'steps')})]
    destination.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(passed=report['passed'], missing=missing)), flush=True)
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
