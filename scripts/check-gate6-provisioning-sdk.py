#!/usr/bin/env python3
"""Qualify the explicit provisioning SDK and CLI on local macOS arm64."""
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
spec = importlib.util.spec_from_file_location('application_sdk', ROOT / 'scripts/check-gate6-application-sdk.py')
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
digest = base.digest


def sources():
    values = base.sources()
    for name in ('scripts/check-gate6-provisioning-sdk.py', 'scripts/check-gate6-provisioning.py'):
        values[name] = digest(ROOT / name)
    return values


def check_samples(directory):
    import jsonschema
    directory = directory.resolve()
    api = json.loads((ROOT / 'contracts/openapi-provisioning.json').read_text())
    apis = {'provisioning': api, 'attachments': json.loads((ROOT / 'contracts/openapi-attachments.json').read_text())}
    checked = []
    for path in sorted(directory.glob('*.json')):
        sample = json.loads(path.read_text())
        name = sample.get('api', 'provisioning')
        selected = apis[name]
        route = selected['paths'][sample['route']]['post']
        for schema, value in (
            (route['requestBody']['content']['application/json']['schema'], sample['request']),
            (route['responses'][str(sample['status'])]['content']['application/json']['schema'], sample['response']),
        ):
            jsonschema.Draft202012Validator(dict(schema, components=selected['components'])).validate(value)
        checked.append(dict(api=name, route=sample['route'], status=sample['status'],
                            private_sample=str(path.relative_to(ROOT)), sha256=digest(path)))
    assert {v['route'] for v in checked if v['api'] == 'provisioning'} == set(api['paths']), 'Missing route samples'
    assert len(api['paths']) >= 9 and len(checked) >= len(api['paths'])
    print(json.dumps(dict(passed=True, routes=len(api['paths']), samples=checked)), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--samples', type=pathlib.Path)
    args = parser.parse_args()
    if args.samples:
        try:
            check_samples(args.samples)
        except Exception as error:
            # Validation errors may contain private sample values.
            print(json.dumps(dict(passed=False, error=type(error).__name__)), flush=True)
            raise SystemExit(1)
        return
    assert platform.system() == 'Darwin' and platform.machine() == 'arm64'
    directory = ROOT / '.local/gate-6/provisioning-sdk' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    before = sources()
    env = dict(os.environ, CONNECT_GATE6_HTTP_SAMPLES=str(directory / 'http-samples'))
    cargo = ['cargo', 'test', '--locked', '--offline', '-j1']
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('client', cargo + ['-p', 'connect-client', '--lib', '--', '--test-threads=1'], 40),
        ('docs', cargo + ['-p', 'connect-client', '--doc'], 3),
        ('application-sdk', cargo + ['-p', 'connect-server', '--test', 'gate6_application_sdk', '--', '--test-threads=1'], 10),
        ('http-provisioning', cargo + ['-p', 'connect-server', '--test', 'gate6_provisioning', '--', '--test-threads=1'], 11),
        ('openapi', [str(ROOT / '.local/gate-6/verification-venv/bin/python'), 'scripts/check-gate6-provisioning-sdk.py', '--samples', str(directory / 'http-samples')], 0),
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
        if name in ('application-sdk', 'http-provisioning') and result.returncode == 0:
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
        ('crates/connect-client/src/application/tests.rs', 'application::tests::'),
        ('crates/connect-client/src/canonical/tests.rs', 'canonical::tests::'),
        ('crates/connect-server/tests/gate6_application_sdk.rs', ''),
        ('crates/connect-server/tests/gate6_provisioning.rs', ''),
    ):
        required.update(base.names(ROOT / file, prefix))
    missing = sorted(required - {t for step in steps for t in step['tests_passed']})
    after = sources()
    historical = ROOT / 'docs/evidence/gate-6/application-sdk.json'
    report = dict(format=1, task='G6-037', implementation_status='in-progress',
        qualification='Explicit trusted provisioning SDK/CLI over the documented current HTTP routes, stable logical IDs, current-state replay, authority refusal and bounded shared transactions on local macOS arm64',
        executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
        source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        source_inputs_sha256=before, sources_unchanged=before == after, steps=steps,
        required_tests=sorted(required), missing_required_tests=missing,
        historical_application_sdk=dict(path=str(historical.relative_to(ROOT)), sha256=digest(historical), disposition='Unmodified historical source snapshot; not current-source qualification'),
        not_qualified=[
            'Credential secret issuance/recovery and rotation tooling remains on earlier configurations. No new credential issuance or recovery API is introduced.',
            'Full ConnectClient/ClientConfig, setup/presence, connected-app, media and remaining task/example consumer migration; G6-037 remains in progress.',
            'All canonical/extension families, refresh/step-up, physical HTTP response-write custody, production media/G6-020, provider/voice/SIP and complete Gate 6 acceptance.',
            'All-workspace metadata and broader bootstrap/native/provider regressions were not repeated for this additive API/CLI migration. Changed client, server library, integration and installed CLI binaries are compiled and exercised.',
            'Linux, remote/public ingress, live providers, physical browsers, release/performance and publication.'
        ],
        exploratory_attempts=[dict(private_log=str(p.relative_to(ROOT)), sha256=digest(p), disposition='Unqualified exploratory run') for p in sorted((ROOT / '.local/gate-6').glob('provisioning-sdk-*-exploratory.log'))],
        passed=before == after and len(steps) == len(commands) and len(required) >= 43 and not missing and all(step['passed'] for step in steps))
    destination = ROOT / 'docs/evidence/gate-6/provisioning-sdk.json'
    if destination.exists():
        previous = json.loads(destination.read_text())
        report['previous_attempts'] = previous.get('previous_attempts', []) + [dict(sha256=digest(destination), **{k: previous[k] for k in ('passed', 'executed_at', 'source_inputs_sha256', 'steps')})]
    destination.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(passed=report['passed'], missing=missing)), flush=True)
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
