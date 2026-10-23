#!/usr/bin/env python3
"""Qualify G6-034 attachment audiences, current links and recipient receipt evidence."""
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
    for name in ('docs/developer/attachments-and-receipts.md',):
        inputs[name] = digest(ROOT / name)
    return dict(sorted(inputs.items()))

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=pathlib.Path)
    args = parser.parse_args()
    directory = ROOT / '.local/gate-6/attachments' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    samples = directory / 'http-samples'
    env = dict(os.environ, CONNECT_GATE6_ATTACHMENT_SAMPLES=str(samples), PYTHONPATH=str(ROOT / 'sdk/python'))
    before = source_inputs()
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('typescript-format', ['npm', 'run', 'format:check', '--prefix', 'sdk/typescript'], 0),
        ('typescript-build', ['npm', 'run', 'build', '--prefix', 'sdk/typescript'], 0),
        ('policy-regression', ['cargo', 'test', '--locked', '--offline', '-j', '1', '--no-fail-fast',
            '-p', 'connect-store', '-p', 'connect-service', '-p', 'connect-protocol', '-p', 'connect-client',
            '-p', 'connect-connector-api', '-p', 'connect-mcp', '--lib', '--tests'], 350),
        ('server-regression', ['cargo', 'test', '--locked', '--offline', '-j', '1', '--no-fail-fast', '-p', 'connect-server', '--lib',
            '--test', 'gate6_attachment_links', '--test', 'gate6_receipts', '--test', 'gate6_messages', '--test', 'gate6_routing',
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
        binaries = {}
        for path in re.findall(r'Running [^\n]+\((target/debug/deps/[^\)]+)\)', raw):
            binary = ROOT / path
            if binary.is_file():
                binaries[path] = digest(binary)
        steps.append(dict(name=name, command=command, exit_code=result.returncode, tests_passed=tests,
            test_binaries_sha256=binaries, private_log=str(log.relative_to(ROOT)), log_sha256=digest(log),
            passed=result.returncode == 0 and len(tests) >= minimum))
        print(json.dumps(dict(step=name, passed=steps[-1]['passed'], rust_tests=len(tests))), flush=True)
    api = json.loads((ROOT / 'contracts/openapi-attachments.json').read_text())
    checked, contract_errors = [], []
    try:
        for path in sorted(samples.glob('*.json')):
            sample = json.loads(path.read_text())
            for field, component in [('request', 'Operation'), ('response', 'Response')]:
                schema = dict(api['components']['schemas'][component], components=api['components'])
                jsonschema.Draft202012Validator(schema).validate(sample[field])
            checked.append(dict(private_sample=str(path.relative_to(ROOT)), sample_sha256=digest(path)))
        assert {p.stem for p in samples.glob('*.json')} == {'upload-link','upload-chunk','inspect','download-link','download-chunk','receipt-ack','receipts'}, 'Missing actual HTTP samples'
    except (AssertionError, KeyError, jsonschema.ValidationError) as error:
        contract_errors.append(type(error).__name__ + ': ' + str(error).splitlines()[0][:200])
    required_tests = {
        'task_links_keep_task_grant_ceiling_and_renewal_never_repeats_the_chunk',
        'stale_links_fail_after_expiry_restart_backup_and_credential_rotation',
        'read_opt_in_is_explicit_monotonic_and_preferences_filter_old_events_before_paging',
        'failed_receipt_journal_rolls_back_facts_and_same_id_can_retry_after_recovery',
        'task_and_invocation_receipt_queries_use_task_recipient_ceiling_and_cannot_report_as_a_client',
        'completed_provider_answer_is_not_read_and_suppressed_sms_stays_failed_without_dispatch',
        'issued_links_upload_once_and_download_only_for_current_message_and_file_recipient',
        'recipient_receipts_and_opt_in_read_reports_share_native_http_mcp_and_sdk_policy',
        'canonical_default_label_and_summary_history_keep_private_attachment_metadata',
    }
    observed = {test for step in steps for test in step['tests_passed']}
    missing = sorted(required_tests - observed)
    earlier = []
    for path in sorted((ROOT / '.local/gate-6').glob('*attempt*.log')):
        if not path.name.startswith(('attachment-links-', 'recipient-receipts-', 'attachments-canonical-')):
            continue
        if re.search(r'error\[|error:|FAILED|error TS', path.read_text(errors='replace')):
            earlier.append(dict(private_log=str(path.relative_to(ROOT)), log_sha256=digest(path),
                disposition='Earlier unsealed candidate failed. Fixture/API names, tool inventory, request ID binding, TypeScript narrowing, cancellation test placement and provenance argument were corrected; only the complete current run is qualification.'))
    after = source_inputs()
    passed = before == after and all(step['passed'] for step in steps) and len(checked) == 7 and not contract_errors and not missing
    report = dict(format=1, task='G6-034', executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        scope='Explicit frozen file audiences; current message/file ACL intersection and integrity; resumable credential/runtime/task/grant-bound upload/download links; expiry, revocation, cancellation, restart and backup; per-recipient/per-channel queued/accepted/delivered/read/failed/unknown observations, explicit optional read reporting and current preference filtering before paging; monotonic duplicates/reordering, rollback/recovery, private task queries, no carrier-to-read/human conversion and unchanged SMS suppression. Actual local native SDK, HTTP raw bytes/control, MCP, TypeScript and Python; canonical payload adapter and summary retention. No canonical wire rendering/dispatch (G6-037), substrate media delivery orchestration, live providers, remote hosting, deployment or full Gate 6 acceptance qualification.',
        platform=platform.platform(), machine=platform.machine(),
        schema=int(re.search(r'SCHEMA_VERSION: u32 = (\d+)', (ROOT / 'crates/connect-store/src/lib.rs').read_text()).group(1)),
        source_revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
        source_inputs_sha256=before, sources_unchanged=before == after, source_lock_sha256=digest(ROOT / 'dependencies/source-lock.json'),
        steps=steps, earlier_failed_attempts=earlier, openapi_samples=checked, contract_errors=contract_errors,
        missing_required_tests=missing, python=sys.version, jsonschema=importlib.metadata.version('jsonschema'), passed=passed)
    output = args.output or ROOT / 'docs/evidence/gate-6/attachments.json'
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.exists():
        previous = json.loads(output.read_text())
        report['previous_attempts'] = previous.get('previous_attempts', []) + [dict(executed_at=previous['executed_at'],
            passed=previous['passed'], report_sha256=digest(output), steps=previous['steps'], source_inputs_sha256=previous['source_inputs_sha256'])]
    output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(task=report['task'], passed=passed, sources_unchanged=before == after, missing_required_tests=missing)), flush=True)
    if not passed:
        raise SystemExit(1)

if __name__ == '__main__':
    main()
