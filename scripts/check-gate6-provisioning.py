#!/usr/bin/env python3
"""Qualify trusted Gate 6 provisioning against isolated store and HTTP fixtures."""
import argparse
import datetime
import hashlib
import importlib.metadata
import json
import os
import pathlib
import platform
import re
import subprocess
import sys

import jsonschema

ROOT = pathlib.Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=pathlib.Path)
    args = parser.parse_args()
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory = ROOT / '.local/gate-6/provisioning' / stamp
    directory.mkdir(parents=True)
    samples = directory / 'http-samples'
    env = dict(os.environ, CONNECT_GATE6_HTTP_SAMPLES=str(samples))
    steps = []
    for name, command in [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh']),
        ('format', ['bash', 'scripts/format.sh', '--check']),
        ('store', ['cargo', 'test', '--locked', '-p', 'connect-store', '--lib', '--test', 'gate6_schema', '--test', 'gate6_provisioning']),
        ('http', ['cargo', 'test', '--locked', '-p', 'connect-server', '--test', 'gate6_provisioning']),
    ]:
        log = directory / (name + '.log')
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT)
        text = log.read_text()
        names = re.findall(r'^test (\S+) \.\.\. ok$', text, re.M)
        binaries = {}
        for name_in_log in re.findall(r'Running [^\n]+\((target/debug/deps/[^\)]+)\)', text):
            binary = ROOT / name_in_log
            if binary.is_file():
                binaries[name_in_log] = digest(binary)
        steps.append(dict(name=name, command=command, exit_code=result.returncode,
                          tests_passed=names, test_binaries_sha256=binaries,
                          private_log=str(log.relative_to(ROOT)), log_sha256=digest(log),
                          passed=result.returncode == 0 and len(names) >= {'store': 15, 'http': 6}.get(name, 0)))
        print(json.dumps({'step': name, 'passed': steps[-1]['passed'], 'tests': len(names)}), flush=True)
    api_path = ROOT / 'contracts/openapi-provisioning.json'
    api = json.loads(api_path.read_text())
    checked, contract_errors = [], []
    try:
        for path in sorted(samples.glob('*.json')):
            sample = json.loads(path.read_text())
            route = api['paths'][sample['route']]['post']
            reference = route['requestBody']['content']['application/json']['schema']
            jsonschema.Draft202012Validator(dict(reference, components=api['components'])).validate(sample['request'])
            response = route['responses'][str(sample['status'])]['content']['application/json']['schema']
            jsonschema.Draft202012Validator(dict(response, components=api['components'])).validate(sample['response'])
            checked.append(dict(route=sample['route'], status=sample['status'], request_schema=sample['request_schema'],
                                private_sample=str(path.relative_to(ROOT)), sample_sha256=digest(path)))
        assert {row['route'] for row in checked} == set(api['paths']), 'Missing actual HTTP route samples'
        assert len(checked) >= 6
    except (AssertionError, KeyError, jsonschema.ValidationError) as error:
        # Do not expose request values in a validator traceback.
        contract_errors.append(type(error).__name__)
    historical_failures = []
    for path, reason in [
        ('provisioning-schema-attempt1.log', 'Historical schema test tried to mutate an endpoint address after ownership became immutable; test now mutates permitted private proof metadata.'),
        ('provisioning-http-attempt1.log', 'HTTP middleware borrowed the whole request across async authentication; fixed by capturing only its headers.'),
        ('provisioning-http-attempt2.log', 'Integration test imported a private module; the bounded HTTP listener now has a documented public re-export.'),
    ]:
        log = ROOT / '.local/gate-6' / path
        if log.exists():
            historical_failures.append(dict(private_log=str(log.relative_to(ROOT)), log_sha256=digest(log),
                                            disposition=reason, source_scope='Earlier unsealed candidate; not current-candidate qualification.'))
    sources = [ROOT / name for name in ('Cargo.toml', 'Cargo.lock', 'dependencies/source-lock.json',
                                      'scripts/check-gate6-provisioning.py', 'scripts/format.sh',
                                      'contracts/openapi-provisioning.json', 'docs/developer/provisioning.md',
                                      'docs/decisions/014-gate-6-contract-version.md')]
    for folder in ('crates', 'examples'):
        sources.extend(path for path in (ROOT / folder).rglob('*')
                       if path.is_file() and (path.suffix in ('.rs', '.sql') or path.name == 'Cargo.toml')
                       and 'target' not in path.parts)
    passed = all(step['passed'] for step in steps) and not contract_errors
    report = dict(format=1, task='G6-030', executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  scope='Trusted provisioning, identity/endpoint revocation, ownership constraints, retry receipts, loopback HTTP admission and actual OpenAPI exchange checks. Fixture addresses only; no provider calls, route reachability, SMS consent, active media teardown, Linux/remote or full Gate 6 qualification.',
                  platform=platform.platform(), machine=platform.machine(), schema=20,
                  source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                  source_inputs_sha256={str(path.relative_to(ROOT)): digest(path) for path in sorted(set(sources))},
                  source_lock_sha256=digest(ROOT / 'dependencies/source-lock.json'),
                  python=sys.version, jsonschema=importlib.metadata.version('jsonschema'),
                  steps=steps, openapi_samples=checked, contract_errors=contract_errors,
                  earlier_failed_attempts=historical_failures, passed=passed)
    output = args.output or ROOT / 'docs/evidence/gate-6/provisioning.json'
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.exists():
        previous = json.loads(output.read_text())
        report['previous_attempts'] = previous.get('previous_attempts', []) + [{
            'executed_at': previous['executed_at'], 'passed': previous['passed'],
            'report_sha256': digest(output), 'steps': previous['steps'],
            'contract_errors': previous['contract_errors'], 'source_inputs_sha256': previous['source_inputs_sha256'],
        }]
    output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'task': report['task'], 'passed': passed, 'openapi_samples': len(checked), 'contract_errors': contract_errors}), flush=True)
    if not passed:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
