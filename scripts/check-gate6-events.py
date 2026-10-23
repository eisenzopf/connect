#!/usr/bin/env python3
"""Qualify G6-036 current snapshots, journal recovery and bounded private observations."""
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
    for name in ('docs/developer/events.md', 'docs/developer/operations.md', 'docs/developer/attachments-and-receipts.md'):
        inputs[name] = digest(ROOT / name)
    return dict(sorted(inputs.items()))

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=pathlib.Path)
    args = parser.parse_args()
    directory = ROOT / '.local/gate-6/events' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    samples = directory / 'http-samples'
    env = dict(os.environ, CONNECT_GATE6_EVENT_SAMPLES=str(samples), PYTHONPATH=str(ROOT / 'sdk/python'))
    before = source_inputs()
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('event-reference', [sys.executable, 'scripts/generate-gate6-event-reference.py', '--check'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('typescript-format', ['npm', 'run', 'format:check', '--prefix', 'sdk/typescript'], 0),
        ('typescript-build', ['npm', 'run', 'build', '--prefix', 'sdk/typescript'], 0),
        ('policy-regression', ['cargo', 'test', '--locked', '--offline', '-j', '1', '--no-fail-fast',
            '-p', 'connect-store', '-p', 'connect-service', '-p', 'connect-protocol', '-p', 'connect-client',
            '-p', 'connect-connector-api', '-p', 'connect-mcp', '--lib', '--tests'], 418),
        ('server-regression', ['cargo', 'test', '--locked', '--offline', '-j', '1', '--no-fail-fast', '-p', 'connect-server', '--lib',
            '--test', 'gate6_events', '--test', 'gate6_operations', '--test', 'gate6_provisioning', '--test', 'gate6_attachment_links', '--test', 'gate6_receipts', '--test', 'gate6_messages', '--test', 'gate6_routing',
            '--test', 'native_messaging', '--test', 'sdk_facade', '--test', 'language_sdks', '--test', 'artifact_recovery',
            '--test', 'transport_parity', '--test', 'gate4_agent_communications', '--test', 'gate4_agent_optional_tasks',
            '--test', 'gate4_agent_authority', '--test', 'gate4_agent_parity', '--test', 'gate4_native_tasks', '--test', 'gate4_native_grants', '--test', 'gate4_native_artifacts',
            '--test', 'gate4_mcp_tools', '--test', 'gate4_mcp_operation_parity', '--test', 'gate4_mcp_auxiliary',
            '--test', 'gate4_vapi_pending_continuation', '--test', 'provider_context_manifest', '--test', 'provider_flow',
            '--test', 'connector_template', '--test', 'connector_contribution', '--test', 'first_run_schema_boundary'], 115),
        ('typescript-tests', ['node', '--test', 'sdk/tests/clients.test.mjs', 'sdk/tests/tasks.test.mjs', 'sdk/tests/reference_client.test.mjs', 'sdk/tests/events.test.mjs'], 0),
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
            passed=result.returncode == 0 and len(tests) >= minimum and (language_count is None or language_count >= (211 if name == 'typescript-tests' else 29))))
        print(json.dumps(dict(step=name, passed=steps[-1]['passed'], rust_tests=len(tests))), flush=True)
    api = json.loads((ROOT / 'contracts/openapi-events.json').read_text())
    checked, contract_errors = [], []
    try:
        for path in sorted(samples.glob('*.json')):
            sample = json.loads(path.read_text())
            for field, component in [('request', 'Operation'), ('response', 'Response')]:
                schema = dict(api['components']['schemas'][component], components=api['components'])
                jsonschema.Draft202012Validator(schema).validate(sample[field])
            checked.append(dict(private_sample=str(path.relative_to(ROOT)), sample_sha256=digest(path)))
        assert {p.stem for p in samples.glob('*.json')} == {'snapshot','events','observations'}, 'Missing actual HTTP samples'
    except (AssertionError, KeyError, jsonschema.ValidationError) as error:
        contract_errors.append(type(error).__name__ + ': ' + str(error).splitlines()[0][:200])
    required_tests = {'native_and_http_share_snapshot_catchup_private_observations_and_restart_boundaries'}
    for path, prefix in [('crates/connect-store/src/events/tests.rs','events::tests::'), ('crates/connect-service/src/event_tests.rs','event_tests::')]:
        test_source=(ROOT/path).read_text()
        required_tests.update(prefix+name for name in re.findall(r'^(?:async )?fn ([a-z_]+)\(\)',test_source,re.M))
    transition_source=(ROOT/'crates/connect-store/src/transitions/tests.rs').read_text()
    stage_sources=re.findall(r'let (?:decision_)?stages = \[(.*?)\];',transition_source,re.S)
    crash_boundaries=[stage for source in stage_sources for stage in re.findall(r'"([^"\n]+)"',source)]
    assert len(crash_boundaries)==27
    # Verify the audit covers every production insertion site, not just a known
    # helper list. Test-only SQL fixtures are intentionally outside this inventory.
    writer_audit=json.loads((ROOT/'contracts/gate6-event-writers.json').read_text())
    expected_writers={}
    for writer in writer_audit['writers']:
        expected_writers[writer['path']]=expected_writers.get(writer['path'],0)+writer['insert_sites']
    actual_writers={}
    for path in (ROOT/'crates/connect-store/src').rglob('*.rs'):
        if 'tests' in path.parts or path.stem == 'tests' or path.stem.endswith('_tests'):continue
        count=len(re.findall(r'INSERT (?:OR [A-Z]+ )?INTO journal\b',path.read_text()))
        if count:actual_writers[str(path.relative_to(ROOT))]=count
    audit_matches=expected_writers==actual_writers
    observed = {test for step in steps for test in step['tests_passed']}
    missing = sorted(required_tests - observed)
    earlier = []
    for path in sorted((ROOT / '.local/gate-6').glob('*attempt*.log')):
        if not path.name.startswith(('events-',)):
            continue
        if re.search(r'error\[|error:|FAILED|error TS', path.read_text(errors='replace')):
            earlier.append(dict(private_log=str(path.relative_to(ROOT)), log_sha256=digest(path),
                disposition='Earlier unsealed candidate failed. Current fixes include deterministic snapshot eviction, test-only buffer occupancy access, explicit SessionEnd reason, the MessageAudience constructor and stored SIP/context rows for the membership audience fixture. Earlier failures remain preserved; only the complete current source snapshot is qualification.'))
    after = source_inputs()
    passed = before == after and all(step['passed'] for step in steps) and len(checked) == 3 and not contract_errors and not missing and audit_matches
    report = dict(format=1, task='G6-036', executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        scope='Current authorized topology snapshots at one transaction watermark with bounded frozen pages and per-item reauthorization; typed journal facts with original stored resource versions/provenance and owner-only core correlation; padded encrypted cursors; explicit retention/expiry/eviction/restart resync; bounded stateless live catch-up and physical-peer cancellation; bounded source/generation/audience-bound transient transcript/speech/action observations with declared capabilities and no implicit action execution. Actual local native Rust, HTTP, TypeScript and Python paths plus closed OpenAPI validation. Backup/future cursors, escaped byte-budget pages, global cache identity pressure, private queue gaps, membership readmission, stale callbacks and prior ledger crash regressions are exercised. Source adapter rows are isolated control fixtures; physical multi-party/stream/provider normalization, canonical wire, remote hosting and full Gate6 acceptance are not qualified.',
        writer_inventory=dict(expected=expected_writers,actual=actual_writers,passed=audit_matches),
        crash_boundaries=crash_boundaries,
        ignored_test_disposition={'transitions::tests::crash_worker': 'Explicitly executed in 27 child processes by the two passing parent crash tests; each must exit 86 at its selected boundary. Not an unexecuted behavioral pass.'},
        platform=platform.platform(), machine=platform.machine(),
        schema=int(re.search(r'SCHEMA_VERSION: u32 = (\d+)', (ROOT / 'crates/connect-store/src/lib.rs').read_text()).group(1)),
        source_revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
        source_inputs_sha256=before, sources_unchanged=before == after, source_lock_sha256=digest(ROOT / 'dependencies/source-lock.json'),
        steps=steps, earlier_failed_attempts=earlier, openapi_samples=checked, contract_errors=contract_errors,
        missing_required_tests=missing, python=sys.version, jsonschema=importlib.metadata.version('jsonschema'), passed=passed)
    output = args.output or ROOT / 'docs/evidence/gate-6/events.json'
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
