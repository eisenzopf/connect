#!/usr/bin/env python3
"""Qualify G6-033 addressed messages with isolated local fixtures and real facades."""
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

try:
    import jsonschema
except ImportError:
    raise SystemExit('Use the Gate 6 verification environment or install scripts/requirements-gate6.txt')

ROOT = pathlib.Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_inputs():
    paths = [ROOT / name for name in (
        'Cargo.toml', 'Cargo.lock', 'dependencies/source-lock.json',
        'scripts/check-gate6-messages.py', 'scripts/check-gate6-contracts.py',
        'scripts/prepare-dependencies.sh', 'scripts/format.sh',
        'docs/developer/messages.md', 'docs/decisions/014-gate-6-contract-version.md',
    )]
    for folder in ('crates', 'contracts', 'sdk', 'examples', 'scripts'):
        paths.extend(path for path in (ROOT / folder).rglob('*')
                     if path.is_file() and not {'node_modules', 'target', '__pycache__'} & set(path.parts)
                     and (path.suffix in ('.rs', '.sql', '.json', '.md', '.ts', '.js', '.mjs', '.py')
                          or path.name == 'Cargo.toml'))
    return {str(path.relative_to(ROOT)): digest(path) for path in sorted(set(paths))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=pathlib.Path)
    args = parser.parse_args()
    directory = ROOT / '.local/gate-6/messages' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    samples = directory / 'http-samples'
    env = dict(os.environ, CONNECT_GATE6_MESSAGE_SAMPLES=str(samples), PYTHONPATH=str(ROOT / 'sdk/python'))
    before = source_inputs()
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('typescript-build', ['npm', 'run', 'build', '--prefix', 'sdk/typescript'], 0),
        ('policy-regression', ['cargo', 'test', '--locked', '--offline', '-j', '1', '--no-fail-fast',
                               '-p', 'connect-store', '-p', 'connect-service', '-p', 'connect-protocol',
                               '-p', 'connect-client', '-p', 'connect-connector-api', '-p', 'connect-mcp',
                               '--lib', '--tests'], 330),
        ('server-regression', ['cargo', 'test', '--locked', '--offline', '-j', '1', '--no-fail-fast', '-p', 'connect-server',
                               '--lib', '--test', 'gate6_messages', '--test', 'gate6_routing',
                               '--test', 'native_messaging', '--test', 'sdk_facade',
                               '--test', 'gate4_agent_communications', '--test', 'gate4_agent_optional_tasks',
                               '--test', 'gate4_agent_authority', '--test', 'gate4_native_tasks',
                               '--test', 'gate4_native_grants', '--test', 'gate4_mcp_tools',
                               '--test', 'gate4_vapi_pending_continuation', '--test', 'provider_context_manifest',
                               '--test', 'provider_flow', '--test', 'connector_template',
                               '--test', 'connector_contribution', '--test', 'first_run_schema_boundary'], 65),
        ('typescript-tests', ['node', '--test', 'sdk/tests/clients.test.mjs', 'sdk/tests/tasks.test.mjs', 'sdk/tests/reference_client.test.mjs'], 0),
        ('python-tests', [sys.executable, '-m', 'unittest', 'discover', '-s', 'sdk/tests', '-p', 'test_*.py'], 0),
        ('contracts', [sys.executable, 'scripts/check-gate6-contracts.py', '--output', str(directory / 'contracts.json')], 0),
        ('progress', [sys.executable, 'scripts/check-gate6-progress.py'], 0),
        ('diff', ['git', 'diff', '--check'], 0),
    ]
    steps = []
    for name, command, minimum in commands:
        log = directory / (name + '.log')
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT)
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
        print(json.dumps(dict(step=name, passed=steps[-1]['passed'], rust_tests=len(names))), flush=True)
    api = json.loads((ROOT / 'contracts/openapi-messages.json').read_text())
    checked = []
    contract_errors = []
    try:
        for path in sorted(samples.glob('*.json')):
            sample = json.loads(path.read_text())
            for field, component in [('request', 'Operation'), ('response', 'Response')]:
                schema = dict(api['components']['schemas'][component], components=api['components'])
                jsonschema.Draft202012Validator(schema).validate(sample[field])
            checked.append(dict(private_sample=str(path.relative_to(ROOT)), sample_sha256=digest(path)))
        assert len(checked) == 2, 'Missing actual send/history HTTP samples'
    except (AssertionError, KeyError, jsonschema.ValidationError) as error:
        contract_errors.append(type(error).__name__)
    dispositions = {
        'messages-qualification-attempt2.log': 'Policy checks passed; disk exhaustion interrupted server test compilation. Further old browser/MCP fixture inputs were migrated; the changed source snapshot was not qualified.',
        'messages-qualification-attempt3.log': 'Broad server tests exposed two obsolete recall assertions (missing timestamps and raw replies). Tests now assert recorded acceptance timestamps and typed reply correlation. The failed run is preserved, not called a pass.',
        'messages-qualification-attempt1.log': 'Unsealed qualification: policy suite passed, server linker exhausted disk and the launcher lacked jsonschema. Final source review also added current original-job authority to legacy reply completion. Re-run in the existing verification environment with fresh source hashes; this attempt is not a pass.',
        'messages-core-attempt1.log': 'Draft migration shadowed a test helper; fixed the fixture binding.',
        'messages-core-attempt2.log': 'Found missing-source recall regression, obsolete absent-timestamp assertion, and early rejection before task business receipts; fixed behavior and recorded timestamp assertion.',
        'messages-addressed-attempt1.log': 'New history-share test initially used an incorrect DTO field/variant; corrected to the public contract.',
        'messages-regression-attempt1.log': 'Integration fixture qualified the protocol type through crate instead of connect_protocol; corrected the import.',
        'messages-regression-attempt2.log': 'Broad replay regression exposed bypassed current pairing state after moving receipt lookup; preserved live admission for non-send operations.',
        'messages-python-tests-attempt1.log': 'Test launcher omitted PYTHONPATH; rerun with the public SDK path.',
        'messages-ts-build-attempt1.log': 'Response validator needed explicit numeric narrowing of a parsed membership epoch.',
        'messages-extra-attempt1.log': 'New task test used a collaboration credential without task_create; corrected the test credential instead of broadening ordinary agent authority.',
        'messages-server-attempt1.log': 'Migrated fixture consumed a reused String; changed it to a borrow.',
        'messages-server-attempt2.log': 'Linker ran out of disk space; reclaimed inactive compiler caches and limited builds to one job. No application state deleted.',
        'messages-server-attempt3.log': 'MCP typed draft schema exposed missing oneOf/const support in its closed argument validator; implemented and tested it.',
        'messages-server-attempt4.log': 'Changed conversation intent checked the new conversation against the old source before conflict detection; fixed original receipt source selection.',
        'messages-server-attempt5.log': 'New CLI parity assertion incorrectly decoded the default unwrapped Outcome as an optional Response envelope; corrected the probe.',
    }
    earlier = [dict(private_log=str(path.relative_to(ROOT)), log_sha256=digest(path), disposition=reason,
                    scope='Earlier unsealed candidate, not current qualification.')
               for name, reason in dispositions.items() if (path := ROOT / '.local/gate-6' / name).exists()]
    after = source_inputs()
    passed = before == after and all(step['passed'] for step in steps) and len(checked) == 2 and not contract_errors
    report = dict(format=1, task='G6-033', executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  scope='Addressed/typed durable messages, current frozen epochs and references, atomic fanout, replay/conflict/recovery, selected provider jobs and explicit unsupported content, narrowed replies and independent inbound policy. Canonical payload service adapters; actual local native Rust SDK, HTTP, CLI, MCP, TypeScript and Python share one accepted intent. OpenAPI validates actual loopback send/history samples. No canonical wire/profile dispatch (G6-037), message-bound private artifact creation/links or delivery observations (G6-034), physical providers, remote hosting or full Gate 6 acceptance qualification.',
                  platform=platform.platform(), machine=platform.machine(), schema=int(re.search(r"SCHEMA_VERSION: u32 = (\d+)", (ROOT / "crates/connect-store/src/lib.rs").read_text()).group(1)),
                  source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                  source_inputs_sha256=before, sources_unchanged=before == after,
                  source_lock_sha256=digest(ROOT / 'dependencies/source-lock.json'),
                  steps=steps, earlier_failed_attempts=earlier, openapi_samples=checked, contract_errors=contract_errors,
                  python=sys.version, jsonschema=importlib.metadata.version('jsonschema'), passed=passed)
    output = args.output or ROOT / 'docs/evidence/gate-6/messages.json'
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.exists():
        previous = json.loads(output.read_text())
        report['previous_attempts'] = previous.get('previous_attempts', []) + [dict(
            executed_at=previous['executed_at'], passed=previous['passed'], report_sha256=digest(output),
            steps=previous['steps'], source_inputs_sha256=previous['source_inputs_sha256'])]
    output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(task=report['task'], passed=passed, sources_unchanged=before == after)), flush=True)
    if not passed:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
