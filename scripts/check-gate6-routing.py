#!/usr/bin/env python3
"""Qualify G6-032 participant endpoint routing with isolated local fixtures."""
import argparse
import datetime
import hashlib
import json
import os
import pathlib
import platform
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_inputs():
    paths = [ROOT / name for name in (
        'Cargo.toml', 'Cargo.lock', 'dependencies/source-lock.json',
        'scripts/check-gate6-routing.py', 'scripts/check-gate6-contracts.py',
        'scripts/prepare-dependencies.sh', 'scripts/format.sh',
        'docs/developer/routing.md', 'docs/decisions/014-gate-6-contract-version.md',
    )]
    for folder in ('crates', 'contracts'):
        paths.extend(path for path in (ROOT / folder).rglob('*')
                     if path.is_file() and (path.suffix in ('.rs', '.sql', '.json', '.md')
                                            or path.name == 'Cargo.toml'))
    return {str(path.relative_to(ROOT)): digest(path) for path in sorted(set(paths))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=pathlib.Path)
    args = parser.parse_args()
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory = ROOT / '.local/gate-6/routing' / stamp
    directory.mkdir(parents=True)
    before = source_inputs()
    steps = []
    samples=directory / 'http-samples'
    env=dict(os.environ,CONNECT_GATE6_ROUTING_SAMPLES=str(samples))
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('policy-regression', ['cargo', 'test', '--locked', '--no-fail-fast',
                               '-p', 'connect-store', '-p', 'connect-service',
                               '-p', 'connect-protocol', '-p', 'connect-client', '-p', 'connect-connector-api',
                               '--lib', '--tests'], 250),
        ('server-regression', ['cargo', 'test', '--locked', '--no-fail-fast', '-p', 'connect-server',
                               '--lib', '--test', 'gate6_provisioning', '--test', 'gate6_routing',
                               '--test', 'native_messaging', '--test', 'sdk_facade'], 50),
        ('contracts', [sys.executable, 'scripts/check-gate6-contracts.py',
                       '--output', str(directory / 'contracts.json')], 0),
        ('progress', [sys.executable, 'scripts/check-gate6-progress.py'], 0),
        ('diff', ['git', 'diff', '--check'], 0),
    ]
    for name, command, minimum in commands:
        log = directory / (name + '.log')
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, env=env,
                                    stdout=output, stderr=subprocess.STDOUT)
        raw = log.read_text()
        names = re.findall(r'^test (\S+) \.\.\. ok$', raw, re.M)
        binaries = {}
        for path in re.findall(r'Running [^\n]+\((target/debug/deps/[^\)]+)\)', raw):
            binary = ROOT / path
            if binary.is_file():
                binaries[path] = digest(binary)
        steps.append(dict(name=name, command=command, exit_code=result.returncode,
                          tests_passed=names, test_binaries_sha256=binaries,
                          private_log=str(log.relative_to(ROOT)), log_sha256=digest(log),
                          passed=result.returncode == 0 and len(names) >= minimum))
        print(json.dumps(dict(step=name, passed=steps[-1]['passed'], tests=len(names))), flush=True)
    dispositions = {
        'routing-focused-attempt4.log': 'Runtime generation tests temporarily passed the cache twice to the observation method after a test-helper refactor; corrected argument lists. This was an unsealed candidate, not a qualification pass.',
        'routing-focused-attempt6.log': 'The command repeated the global --lib flag; Cargo refused it before executing tests. The corrected invocation uses the flag once for all selected packages.',
    }
    earlier = [dict(private_log=str(path.relative_to(ROOT)), log_sha256=digest(path),
                    disposition=reason, scope='Earlier unsealed candidate; not current qualification.')
               for name, reason in dispositions.items()
               if (path := ROOT / '.local/gate-6' / name).exists()]
    import jsonschema
    api=json.loads((ROOT/'contracts/openapi-routing.json').read_text())
    checked=[]
    contract_errors=[]
    try:
        for path in sorted(samples.glob('*.json')):
            sample=json.loads(path.read_text())
            for field,component in [('request','Operation'),('response','Response')]:
                schema=dict(api['components']['schemas'][component],components=api['components'])
                jsonschema.Draft202012Validator(schema).validate(sample[field])
            checked.append(dict(private_sample=str(path.relative_to(ROOT)),sample_sha256=digest(path)))
        assert checked, 'Missing actual SDK/HTTP route sample'
    except (AssertionError,KeyError,jsonschema.ValidationError) as error:
        contract_errors.append(type(error).__name__)
    after = source_inputs()
    passed = before == after and all(step['passed'] for step in steps) and bool(checked) and not contract_errors
    report = dict(format=1, task='G6-032',
                  executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  scope='Participant endpoint selection, current ownership/scope/tenant, bounded fresh runtime observations and generations, explicit preference/failover/unknown attempts, consent and configured credential bindings; real native TLS/QUIC SDK and loopback HTTP selection parity with an actual OpenAPI sample. Endpoint observations are local fixtures; no physical endpoint dial, real provider reachability, multiparty media admission, browser/WebTransport sessions, remote hosting or full Gate 6 acceptance qualification.',
                  platform=platform.platform(), machine=platform.machine(), schema=int(re.search(r"SCHEMA_VERSION: u32 = (\d+)", (ROOT / "crates/connect-store/src/lib.rs").read_text()).group(1)),
                  source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                  source_inputs_sha256=before, sources_unchanged=before == after,
                  source_lock_sha256=digest(ROOT / 'dependencies/source-lock.json'),
                  steps=steps, earlier_failed_attempts=earlier, openapi_samples=checked, contract_errors=contract_errors, passed=passed)
    output = args.output or ROOT / 'docs/evidence/gate-6/routing.json'
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.exists():
        previous = json.loads(output.read_text())
        report['previous_attempts'] = previous.get('previous_attempts', []) + [dict(
            executed_at=previous['executed_at'], passed=previous['passed'],
            report_sha256=digest(output), steps=previous['steps'],
            source_inputs_sha256=previous['source_inputs_sha256'])]
    output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(task=report['task'], passed=passed, sources_unchanged=before == after)), flush=True)
    if not passed:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
