#!/usr/bin/env python3
"""Check the G6-020 process-local model without claiming media integration."""
import datetime
import hashlib
import json
import pathlib
import platform
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
CORE = ROOT / '.deps/rvoip/crates/foundation/rvoip-core'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sources():
    paths = list(CORE.rglob('*.rs'))
    paths += list((CORE.parent / 'rvoip-core-traits').rglob('*.rs'))
    paths += [CORE / 'Cargo.toml', CORE.parent / 'rvoip-core-traits/Cargo.toml']
    paths += [ROOT / name for name in (
        'dependencies/source-lock.json', '.deps/rvoip/Cargo.toml',
        '.deps/rvoip/Cargo.lock', 'scripts/check-gate6-stream-model.py',
        'scripts/prepare-dependencies.py', 'scripts/lib/prepared_archive.py',
        'docs/developer/stream-lifecycle.md')]
    lock = json.loads((ROOT / 'dependencies/source-lock.json').read_text())
    paths += [ROOT / patch['path'] for patch in lock['patches']]
    return {str(path.relative_to(ROOT)): digest(path) for path in sorted(set(paths))}


def main():
    directory = ROOT / '.local/gate-6/stream-model' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    before = sources()
    suites = ['participant_control', 'session_lifecycle', 'participant_roles',
              'messaging', 'p5_pause_and_listener', 'stream_readiness',
              'tenant_and_capacity', 'orchestrator_dispatch', 'provisional_early_media']
    cargo = ['cargo', 'test', '--locked', '--offline', '-j', '1', '--target-dir', 'target',
             '--manifest-path', '.deps/rvoip/Cargo.toml', '-p', 'rvoip-core', '--features', 'opus']
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('model-format', ['rustfmt', '--edition', '2021', '--config', 'skip_children=true', '--check',
                         str((CORE / 'src/stream_lifecycle.rs').relative_to(ROOT)),
                         str((CORE / 'src/stream_lifecycle/tests.rs').relative_to(ROOT))], 0),
        ('core-library', cargo + ['--lib'], 197),
        ('participant-and-media', cargo + [arg for suite in suites for arg in ('--test', suite)], 149),
        ('progress', ['python3', 'scripts/check-gate6-progress.py'], 0),
        ('diff', ['git', 'diff', '--check'], 0),
    ]
    steps = []
    for name, command, minimum in commands:
        log = directory / (name + '.log')
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, stdout=output, stderr=subprocess.STDOUT)
        raw = log.read_text()
        tests = re.findall(r'^test (\S+) \.\.\. ok$', raw, re.M)
        binaries = {}
        for path in re.findall(r'Running [^\n]+\((target/debug/deps/[^\)]+)\)', raw):
            if (ROOT / path).is_file():
                binaries[path] = digest(ROOT / path)
        step = dict(name=name, command=command, exit_code=result.returncode,
                    tests_passed=tests, private_log=str(log.relative_to(ROOT)),
                    log_sha256=digest(log), test_binaries_sha256=binaries,
                    passed=result.returncode == 0 and len(tests) >= minimum)
        steps.append(step)
        print(json.dumps(dict(step=name, passed=step['passed'], tests=len(tests))), flush=True)
    source = (CORE / 'src/stream_lifecycle/tests.rs').read_text()
    required = {'stream_lifecycle::tests::' + name for name in re.findall(r'#\[test\]\s*fn ([a-z_]+)\(', source)}
    observed = {name for step in steps for name in step['tests_passed']}
    missing = sorted(required - observed)
    after = sources()
    report = dict(format=1, task='G6-020', implementation_status='in-progress',
                  qualification='process-local model and upstream regression only',
                  profile_binding_width_bits=16,
                  candidate_review='An earlier passing model candidate used a wider transport-local handle. Follow-up comparison with the existing UCTP datagram header narrowed ready/model bindings to u16; external integers require checked conversion. This does not qualify datagram routing or physical media.',
                  executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  platform=platform.platform(), source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                  source_inputs_sha256=before, sources_unchanged=before == after,
                  steps=steps, required_model_tests=sorted(required), missing_required_tests=missing,
                  not_qualified=[
                      'The new model does not yet own adapter renegotiation or physical media routes.',
                      'No new physical audio/video/screen/data upgrade, downgrade, pause, resume or subscription behavior is qualified.',
                      'Current policy installation, atomic media graph commit, cancellation and uncertain cleanup remain open.',
                      'Public Connect canonical UCTP dispatch, SDK integration, independent clients, browser/native transport and providers remain separate.',
                      'All G6-020 implementation verification and Gate 6 acceptance remain incomplete.'
                  ],
                  passed=before == after and len(required) >= 16 and not missing and all(step['passed'] for step in steps))
    attempts = []
    for path in sorted((ROOT / '.local/gate-6').glob('streams-model-attempt*.log')):
        raw = path.read_text(errors='replace')
        attempts.append(dict(private_log=str(path.relative_to(ROOT)), log_sha256=digest(path),
                             disposition=('Initial upstream-default target build was interrupted to reuse the existing explicit target cache; no test result.' if path.name.endswith('attempt1.log') else
                                          'Exploratory earlier model snapshot; only the current complete unchanged source run establishes this scoped report.'),
                             failures_observed=bool(re.search(r'error\[|error:|FAILED', raw))))
    report['exploratory_attempts'] = attempts
    destination = ROOT / 'docs/evidence/gate-6/stream-model.json'
    if destination.exists():
        previous = json.loads(destination.read_text())
        report['previous_attempts'] = previous.get('previous_attempts', []) + [
            dict(executed_at=previous['executed_at'], passed=previous['passed'],
                 report_sha256=digest(destination), steps=previous['steps'],
                 source_inputs_sha256=previous['source_inputs_sha256'])]
    destination.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(task=report['task'], passed=report['passed'], sources_unchanged=report['sources_unchanged'], missing_required_tests=missing)), flush=True)
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
