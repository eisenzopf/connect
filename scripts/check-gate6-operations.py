#!/usr/bin/env python3
"""Qualify G6-035 durable operations, fenced workers, uncertainty and atomic decisions."""
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
import importlib.util
ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('message_qualification', ROOT / 'scripts/check-gate6-messages.py')
shared = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shared)

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def source_inputs():
    inputs = shared.source_inputs()
    for name in ('docs/developer/operations.md', 'docs/developer/attachments-and-receipts.md'):
        inputs[name] = digest(ROOT / name)
    return dict(sorted(inputs.items()))

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=pathlib.Path)
    args = parser.parse_args()
    directory = ROOT / '.local/gate-6/operations' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    samples = directory / 'http-samples'
    env = dict(os.environ, CONNECT_GATE6_OPERATION_SAMPLES=str(samples), PYTHONPATH=str(ROOT / 'sdk/python'))
    before = source_inputs()
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('typescript-format', ['npm', 'run', 'format:check', '--prefix', 'sdk/typescript'], 0),
        ('typescript-build', ['npm', 'run', 'build', '--prefix', 'sdk/typescript'], 0),
        ('policy-regression', ['cargo', 'test', '--locked', '--offline', '-j', '1', '--no-fail-fast',
            '-p', 'connect-store', '-p', 'connect-service', '-p', 'connect-protocol', '-p', 'connect-client',
            '-p', 'connect-connector-api', '-p', 'connect-mcp', '--lib', '--tests'], 390),
        ('server-regression', ['cargo', 'test', '--locked', '--offline', '-j', '1', '--no-fail-fast', '-p', 'connect-server', '--lib',
            '--test', 'gate6_operations', '--test', 'gate6_provisioning', '--test', 'gate6_attachment_links', '--test', 'gate6_receipts', '--test', 'gate6_messages', '--test', 'gate6_routing',
            '--test', 'native_messaging', '--test', 'sdk_facade', '--test', 'language_sdks', '--test', 'artifact_recovery',
            '--test', 'transport_parity', '--test', 'gate4_agent_communications', '--test', 'gate4_agent_optional_tasks',
            '--test', 'gate4_agent_authority', '--test', 'gate4_agent_parity', '--test', 'gate4_native_tasks', '--test', 'gate4_native_grants', '--test', 'gate4_native_artifacts',
            '--test', 'gate4_mcp_tools', '--test', 'gate4_mcp_operation_parity', '--test', 'gate4_mcp_auxiliary',
            '--test', 'gate4_vapi_pending_continuation', '--test', 'provider_context_manifest', '--test', 'provider_flow',
            '--test', 'connector_template', '--test', 'connector_contribution', '--test', 'first_run_schema_boundary'], 70),
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
        tests = re.findall(r'^test (\S+) \.\.\. ok$', raw, re.M)
        language_count = None
        if name == 'typescript-tests':
            match = re.search(r'^(?:#|ℹ) pass (\d+)$', raw, re.M)
            language_count = int(match.group(1)) if match else 0
        elif name == 'python-tests':
            match = re.search(r'Ran (\d+) tests?', raw)
            language_count = int(match.group(1)) if match else 0
        binaries = {}
        for path in re.findall(r'Running [^\n]+\((target/debug/deps/[^\)]+)\)', raw):
            binary = ROOT / path
            if binary.is_file():
                binaries[path] = digest(binary)
        steps.append(dict(name=name, command=command, exit_code=result.returncode, tests_passed=tests, language_tests_passed=language_count,
            test_binaries_sha256=binaries, private_log=str(log.relative_to(ROOT)), log_sha256=digest(log),
            passed=result.returncode == 0 and len(tests) >= minimum and (language_count is None or language_count >= (175 if name == 'typescript-tests' else 25))))
        print(json.dumps(dict(step=name, passed=steps[-1]['passed'], rust_tests=len(tests))), flush=True)
    api = json.loads((ROOT / 'contracts/openapi-operations.json').read_text())
    checked, contract_errors = [], []
    try:
        for path in sorted(samples.glob('*.json')):
            sample = json.loads(path.read_text())
            for field, component in [('request', 'Operation'), ('response', 'Response')]:
                schema = dict(api['components']['schemas'][component], components=api['components'])
                jsonschema.Draft202012Validator(schema).validate(sample[field])
            checked.append(dict(private_sample=str(path.relative_to(ROOT)), sample_sha256=digest(path)))
        assert {p.stem for p in samples.glob('*.json')} == {'admit','status','cancel'}, 'Missing actual HTTP samples'
    except (AssertionError, KeyError, jsonschema.ValidationError) as error:
        contract_errors.append(type(error).__name__ + ': ' + str(error).splitlines()[0][:200])
    required_tests = {
        'fresh_exchanges_and_current_credentials_share_one_durable_transition_receipt',
    }
    test_source = (ROOT / 'crates/connect-store/src/transitions/tests.rs').read_text()
    # All top-level behavioral tests are required. The ignored worker entry is
    # explicitly executed by the parent, with actual exit-code/counter assertions.
    names = re.findall(r'^fn ([a-z_]+)\(\)', test_source, re.M)
    required_tests.update('transitions::tests::' + name for name in names if name != 'crash_worker')
    stage_sources = re.findall(r'let (?:decision_)?stages = \[(.*?)\];', test_source, re.S)
    crash_boundaries = [stage for source in stage_sources for stage in re.findall(r'"([^"\n]+)"', source)]
    assert len(crash_boundaries) == 27 and len(set(crash_boundaries)) == 27
    observed = {test for step in steps for test in step['tests_passed']}
    missing = sorted(required_tests - observed)
    earlier = []
    for path in sorted((ROOT / '.local/gate-6').glob('*attempt*.log')):
        if not path.name.startswith(('ledger-',)):
            continue
        if re.search(r'error\[|error:|FAILED|error TS', path.read_text(errors='replace')):
            earlier.append(dict(private_log=str(path.relative_to(ROOT)), log_sha256=digest(path),
                disposition='Earlier unsealed candidate failed. The HTTP shape-rejection expectation was corrected to 422; the protected reconciliation callback received explicit Rust parameter types. Other draft compile/test failures remain preserved here; only the complete current source snapshot is qualification.'))
    after = source_inputs()
    passed = before == after and all(step['passed'] for step in steps) and len(checked) == 3 and not contract_errors and not missing
    report = dict(format=1, task='G6-035', executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        scope='Stable tenant/principal operation IDs and normalized digests; current source/recipient authority, epoch/revision/connection/endpoint snapshots; atomic reservation/worker fencing and local decision receipts; attributed effect submission/observation/compensation, cancellation/reconciliation, restart/backup, capacity and retention tombstones. Twenty-seven actual child-process crash boundaries use a durable isolated fixture-provider counter and local SQL role changes. Native Rust, loopback HTTP, TypeScript and Python share fresh exchanges and original logical IDs; same-process server restart fences old leases. Protected SQL coordinator hooks are tested foundations; physical readiness, consultation/handoff/replacement coordinators, canonical wire dispatch, live providers, remote hosting and full Gate 6 acceptance are not qualified.',
        crash_boundaries=crash_boundaries,
        ignored_test_disposition={'transitions::tests::crash_worker': 'Explicitly executed in 27 child processes by the two passing parent crash tests; each must exit 86 at its selected boundary. Not an unexecuted behavioral pass.'},
        platform=platform.platform(), machine=platform.machine(),
        schema=int(re.search(r'SCHEMA_VERSION: u32 = (\d+)', (ROOT / 'crates/connect-store/src/lib.rs').read_text()).group(1)),
        source_revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
        source_inputs_sha256=before, sources_unchanged=before == after, source_lock_sha256=digest(ROOT / 'dependencies/source-lock.json'),
        steps=steps, earlier_failed_attempts=earlier, openapi_samples=checked, contract_errors=contract_errors,
        missing_required_tests=missing, python=sys.version, jsonschema=importlib.metadata.version('jsonschema'), passed=passed)
    output = args.output or ROOT / 'docs/evidence/gate-6/operations.json'
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.exists():
        previous = json.loads(output.read_text())
        report['previous_attempts'] = previous.get('previous_attempts', []) + [dict(executed_at=previous['executed_at'],
            passed=previous['passed'], sources_unchanged=previous['sources_unchanged'], missing_required_tests=previous['missing_required_tests'], contract_errors=previous['contract_errors'], report_sha256=digest(output), steps=previous['steps'], source_inputs_sha256=previous['source_inputs_sha256'])]
    output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(task=report['task'], passed=passed, sources_unchanged=before == after, missing_required_tests=missing)), flush=True)
    if not passed:
        raise SystemExit(1)

if __name__ == '__main__':
    main()

