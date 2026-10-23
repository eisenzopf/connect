#!/usr/bin/env python3
"""Source-bound local canonical transition dispatch, proof custody and shared receipt checks."""
import argparse
import datetime
import importlib.util
import json
import os
import pathlib
import platform
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('revocation', ROOT / 'scripts/check-gate6-credential-revocation.py')
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
digest = base.digest
names = base.names


def sources():
    values = base.sources()
    name = 'scripts/check-gate6-canonical-transitions.py'
    values[name] = digest(ROOT / name)
    return values


def check_samples(directory):
    import jsonschema
    directory = directory.resolve()
    api = json.loads((ROOT/'contracts/openapi-operations.json').read_text())
    checked = []
    for path in sorted(directory.glob('canonical-transition-*.json')):
        sample = json.loads(path.read_text())
        assert sample['api']=='operations'
        route = api['paths'][sample['route']]['post']
        for schema, value in (
            (route['requestBody']['content']['application/json']['schema'],sample['request']),
            (route['responses'][str(sample['status'])]['content']['application/json']['schema'],sample['response']),
        ):
            jsonschema.Draft202012Validator(dict(schema,components=api['components'])).validate(value)
        checked.append(dict(private_sample=str(path.relative_to(ROOT)),sha256=digest(path),status=sample['status']))
    required = {f'canonical-transition-{kind}-{phase}.json' for kind in range(3) for phase in ('status','replay','cancel')} | {'canonical-transition-http-first.json','canonical-transition-lost-source.json'}
    assert required=={p.name for p in directory.glob('canonical-transition-*.json')}
    assert {row['status'] for row in checked}=={200,403}
    print(json.dumps(dict(passed=True,samples=checked)),flush=True)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--samples',type=pathlib.Path);args=parser.parse_args()
    if args.samples:
        try:check_samples(args.samples)
        except Exception as error:
            print(json.dumps(dict(passed=False,error=type(error).__name__)),flush=True);raise SystemExit(1)
        return
    assert platform.system() == 'Darwin' and platform.machine() == 'arm64'
    directory = ROOT / '.local/gate-6/canonical-transitions' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    before = sources()
    env = dict(os.environ, CONNECT_GATE6_HTTP_SAMPLES=str(directory / 'http-samples'))
    cargo = ['cargo', 'test', '--locked', '--offline', '-j1']
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('client', cargo + ['-p', 'connect-client', '--lib', '--', '--test-threads=1'], 40),
        ('docs', cargo + ['-p', 'connect-client', '--doc'], 3),
        ('store-transitions', cargo + ['-p', 'connect-store', '--lib', 'transitions::tests', '--', '--test-threads=1'], 46),
        ('canonical-transitions', cargo + ['-p', 'connect-server', '--test', 'gate6_canonical_transitions', '--', '--test-threads=1'], 3),
        ('production-native', cargo + ['-p', 'connect-server', '--test', 'gate6_canonical_runtime', '--', '--test-threads=1'], 11),
        ('openapi', [str(ROOT / '.local/gate-6/verification-venv/bin/python'), 'scripts/check-gate6-canonical-transitions.py', '--samples', str(directory / 'http-samples')], 0),
        ('contracts', [str(ROOT / '.local/gate-6/verification-venv/bin/python'), 'scripts/check-gate6-contracts.py', '--output', str(directory / 'contracts.json')], 0),
        ('progress', ['python3', 'scripts/check-gate6-progress.py'], 0),
        ('diff', ['git', 'diff', '--check'], 0),
    ]
    steps = []
    for name, command, minimum in commands:
        log = directory / (name + '.log')
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT)
        raw = log.read_text(errors='replace')
        tests = re.findall(r'^test (.+?) \.\.\. ok$', raw, re.M)
        binaries = {p: digest(ROOT / p) for p in re.findall(r'Running [^\n]+\((target/debug/deps/[^\)]+)\)', raw)}
        if name in ('canonical-transitions', 'production-native') and result.returncode == 0:
            binaries['target/debug/connect'] = digest(ROOT / 'target/debug/connect')
        step = dict(name=name, command=command, exit_code=result.returncode, tests_passed=tests,
                    private_log=str(log.relative_to(ROOT)), log_sha256=digest(log),
                    test_binaries_sha256=binaries, passed=result.returncode == 0 and len(tests) >= minimum)
        steps.append(step)
        print(json.dumps(dict(step=name, passed=step['passed'], tests=len(tests))), flush=True)
        if not step['passed']:
            break
    required = set()
    for file, prefix in (
        ('crates/connect-client/src/canonical/tests.rs', 'canonical::tests::'),
        ('crates/connect-server/tests/gate6_canonical_transitions.rs', ''),
        ('crates/connect-server/tests/gate6_canonical_runtime.rs', ''),
    ):
        required.update(names(ROOT / file, prefix))
    required.update('transitions::tests::peer_operations::'+n for n in names(ROOT/'crates/connect-store/src/transitions/tests/peer_operations.rs') if n.startswith('canonical_transition_'))
    missing = sorted(required - {t for step in steps for t in step['tests_passed']})
    after = sources()
    historical = ROOT / 'docs/evidence/gate-6/credential-revocation.json'
    report = dict(format=1, task='G6-037', implementation_status='in-progress',
        qualification='Signed consult/handoff/replacement dispatch to the existing durable transition ledger with normalized exact peer proof, retained source readers, immutable operation expiry, HTTP/native status/replay/cancellation and honest pending state; local macOS arm64',
        executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
        source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        source_inputs_sha256=before, sources_unchanged=before == after, steps=steps,
        required_tests=sorted(required), missing_required_tests=missing,
        historical_credential_revocation=dict(path=str(historical.relative_to(ROOT)), sha256=digest(historical), disposition='Unchanged historical source snapshot; not current-source qualification'),
        ignored_test_disposition={'transitions::tests::crash_worker': 'Explicit worker invoked by required passing crash/recovery parent tests; its standalone ignored entry is not counted as a pass.'},
        not_qualified=[
            'Fixture adapter rows qualify signed dispatch/admission/cancellation only, not physical media readiness, actual consultation/handoff/device movement or coordinator completion.',
            'Remaining session/connection/stream installation, approved application profiles and obsolete dispatch-path removal; G6-037 remains in progress. G6-020 media and later execution tasks remain open.',
            'Complete high-level SDK/consumer migration, credential secret issuance/recovery, refresh/step-up, HTTP physical-write custody and complete Gate 6 acceptance.',
            'All-workspace metadata, broader bootstrap/provider/workflow regressions and unchanged credential-revocation qualification were not repeated. Changed libraries, SDK/test/CLI binaries build and execute; selected production native regressions repeat.',
            'Linux, remote/public ingress, live providers, physical browsers, release/performance and publication.'
        ],
        exploratory_attempts=[dict(private_log=str(p.relative_to(ROOT)), sha256=digest(p), disposition='Unqualified exploratory run') for p in sorted((ROOT / '.local/gate-6').glob('canonical-transitions-*-exploratory.log'))],
        passed=before == after and len(steps) == len(commands) and len(required) >= 26 and not missing and all(step['passed'] for step in steps))
    destination = ROOT / 'docs/evidence/gate-6/canonical-transitions.json'
    if destination.exists():
        previous = json.loads(destination.read_text())
        report['previous_attempts'] = previous.get('previous_attempts', []) + [dict(sha256=digest(destination), **{k: previous[k] for k in ('passed', 'executed_at', 'source_inputs_sha256', 'steps')})]
    destination.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(passed=report['passed'], missing=missing)), flush=True)
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
