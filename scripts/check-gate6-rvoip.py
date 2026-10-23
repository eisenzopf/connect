#!/usr/bin/env python3
"""Test recorded Gate 6 rvoip patches and save scoped qualification evidence."""
import argparse
import datetime
import hashlib
import json
import pathlib
import platform
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
CORE = ROOT / '.deps/rvoip/crates/foundation/rvoip-core'
COMPONENTS = {
    'wire': ('G6-016', 'wire-types', 'v2::tests', 11,
             'Shared wire decoding, typed payloads and transition guards.'),
    'relationships': ('G6-017', 'relationships', 'relationships::tests', 7,
                      'Shared relationship transitions, membership epochs, session roles and endpoint lineage.'),
    'messages': ('G6-018', 'addressed-messages', 'gate6_', 6,
                 'Engine recipient filtering, typed gateway metadata, frozen audiences, current policy projections and scoped receipts with deterministic adapters.'),
    'participants': ('G6-019', 'participant-transitions', 'participant_transitions::tests', 14,
                     'Process-local participant transition controller and Session leave membership isolation. No actual media activation or retirement.'),
    'orchestration': ('G6-019', 'participant-orchestration', '', 15,
                      'Current-authority participant orchestration over the actual encoded-audio mixer, pending capacity, cancellation, generation-fenced retirement and isolated Session roles.'),
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run_check(command, log):
    with log.open('w') as output:
        result = subprocess.run(command, cwd=ROOT, stdout=output, stderr=subprocess.STDOUT)
    text = log.read_text()
    binaries = re.findall(r'^\s+Running [^\n]+ \(([^\n)]+)\)$', text, re.M)
    return dict(command=command, exit_code=result.returncode,
                test_binaries_sha256={p: digest(ROOT / p) for p in binaries if (ROOT / p).is_file()},
                tests_passed=re.findall(r'^test (\S+) \.\.\. ok$', text, re.M),
                private_log=str(log.relative_to(ROOT)), log_sha256=digest(log))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--component', choices=COMPONENTS, default='wire')
    parser.add_argument('--output', type=pathlib.Path)
    args = parser.parse_args()
    subprocess.run(['bash', 'scripts/prepare-dependencies.sh'], cwd=ROOT, check=True)
    upstream = ROOT / '.deps/rvoip/crates/uctp/rvoip-uctp/src/v2'
    for source, destination in [('contracts/schemas/uctp-envelope-2.schema.json', 'envelope.schema.json'),
                                ('contracts/uctp-vectors/gate6-envelope.json', 'golden-vectors.json')]:
        if digest(ROOT / source) != digest(upstream / destination):
            raise SystemExit(f'Published and compiled contract differ: {source}')
    task, stem, prefix, minimum, scope = COMPONENTS[args.component]
    wire = args.component == 'wire'
    messages = args.component == 'messages'
    orchestration = args.component == 'orchestration'
    participants = args.component in ('participants', 'orchestration')
    package = 'rvoip-uctp' if wire else 'rvoip-core'
    base = ['cargo', 'test', '--locked', '--manifest-path', '.deps/rvoip/Cargo.toml', '-p', package,
            *([] if wire else ['--features', 'opus'])]
    selection = ['--test', 'participant_control'] if orchestration else (['--test', 'messaging'] if messages else ['--lib', prefix])
    command = base + selection + ['--', '--nocapture']
    directory = ROOT / '.local/gate-6'
    if participants:
        directory /= stem + '/' + datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True, exist_ok=True)
    primary = run_check(command, directory / (stem + '.log'))
    focused = [name for name in primary['tests_passed'] if name.startswith(prefix)]
    regressions = []
    if participants and primary['exit_code'] == 0:
        library = ['--lib'] if orchestration else ['--lib', 'relationships::tests']
        regressions.append(run_check(base + library + ['--', '--nocapture'], directory / 'core-library.log'))
        suites = ['session_lifecycle', 'participant_roles', 'messaging']
        if orchestration:
            suites += ['p5_pause_and_listener', 'stream_readiness', 'tenant_and_capacity',
                       'orchestrator_dispatch', 'provisional_early_media']
        selections = [arg for suite in suites for arg in ('--test', suite)]
        regressions.append(run_check(base + selections + ['--', '--nocapture'], directory / 'session-and-media.log'))
    vectors = json.loads((upstream / 'golden-vectors.json').read_text())['vectors'] if wire else []
    source_paths = sorted(upstream.glob('*.rs')) if wire else [
        CORE / 'src/relationships.rs', CORE / 'src/relationships/tests.rs',
        ROOT / '.deps/rvoip/crates/foundation/rvoip-core-traits/src/ids.rs',
    ]
    if messages:
        source_paths = [CORE / path for path in (
            'src/addressed_messages.rs', 'src/config.rs', 'src/orchestrator.rs',
            'src/message.rs', 'tests/messaging.rs')]
    if participants:
        source_paths = [CORE / path for path in (
            'src/lib.rs', 'src/participant_transitions.rs', 'src/participant_transitions/tests.rs',
            'src/relationships.rs', 'src/relationships/tests.rs', 'src/orchestrator.rs',
            'tests/session_lifecycle.rs', 'tests/participant_roles.rs', 'tests/messaging.rs')]
        if orchestration:
            source_paths += [CORE / path for path in (
                'src/conference.rs', 'src/commands.rs', 'src/orchestrator/participant_control.rs',
                'tests/participant_control.rs', 'tests/support/participant_control_fixture.rs')]
    passed = primary['exit_code'] == 0 and len(focused) >= minimum
    if participants:
        passed = passed and len(regressions) == 2 and all(
            check['exit_code'] == 0 and check['tests_passed'] for check in regressions)
    report = dict(
        format=1, task=task, executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        scope=scope + (' Local engine fixtures only; not public Connect service/SDK, external authentication, remote transport, provider or independent client qualification.' if orchestration else
                       ' Not server dispatch, external authentication, live media, provider or independent client conformance.'),
        **primary,
        source_lock_sha256=digest(ROOT / 'dependencies/source-lock.json'),
        schema_sha256=digest(upstream / 'envelope.schema.json'),
        vectors_sha256=digest(upstream / 'golden-vectors.json'),
        upstream_sources={str(p.relative_to(ROOT / '.deps/rvoip')): digest(p) for p in source_paths},
        schema_vectors=len(vectors), valid_type_vectors=sum(v['valid'] for v in vectors),
        focused_tests_passed=focused, passed=bool(passed),
    )
    if participants:
        report.update(
            implementation_status=('upstream participant engine implementation; downstream service integration remains separate' if orchestration else 'partial; G6-019 remains in-progress'),
            regressions=regressions,
            runner_sha256=digest(pathlib.Path(__file__)),
            platform=platform.platform(),
            rustc=subprocess.check_output(['rustc', '-Vv'], text=True).strip(),
            not_qualified=([
                'Connect durable membership/operation policy installation and public Session dispatch remain G6-031/G6-035/G6-038.',
                'Remote native/browser transports, real providers, video/screen/data upgrades and cross-process recovery have separate qualification tasks.',
                'Engine receipts are process-local; local route retirement never asserts a confirmed provider hangup.',
            ] if orchestration else [
                'Orchestrator join/leave and role APIs do not yet use the new controller.',
                'Actual active-media admission/removal, pending slot reservations and generation-fenced adapter cleanup remain unrun.',
                'Current caller grants and durable replay require Connect service/store integration.',
            ]),
        )
    output = args.output or ROOT / 'docs/evidence/gate-6' / (stem + '.json')
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: report[k] for k in ('task', 'exit_code', 'schema_vectors', 'valid_type_vectors', 'passed')}))
    if not report['passed']:
        for check in [primary, *regressions]:
            if check['exit_code']:
                print((ROOT / check['private_log']).read_text()[-6000:])
        raise SystemExit(1)


if __name__ == '__main__':
    main()
