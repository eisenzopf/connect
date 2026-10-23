#!/usr/bin/env python3
"""Qualify G6-025 reusable observations with actual local HTTP/WebSocket fixtures."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
PACKAGES = ['rvoip-core-traits', 'rvoip-core', 'rvoip-vapi']
REQUIRED = [
    'agent::tests::partial_final_and_repeated_provider_ids_remain_distinct_without_claiming_capture',
    'agent::tests::complete_metadata_is_bounded_and_invalid_references_or_gap_cannot_publish_a_prefix',
    'agent::tests::fresh_context_has_bounded_canonical_scope_and_preserves_incomplete_data',
    'agent::tests::diagnostics_exclude_transcript_action_arguments_ids_and_context',
    'agent::tests::global_projection_and_diagnostics_exclude_private_agent_content',
    'operational_events::tests::observation_correlations_distinguish_stream_incarnations_and_missing_binding',
    'operational_events::tests::cancelled_backpressured_send_marks_stream_degraded',
    'operational_events::tests::cancelled_lifecycle_after_mutation_before_send_marks_stream_degraded',
    'operational_events::tests::sequence_exhaustion_notifies_health_subscribers',
    'operational_events::tests::failed_bounded_send_notifies_health_subscribers',
    'agent_observations::tests::provider_call_assertions_cannot_borrow_a_current_call_or_terminate_it',
    'agent_observations::tests::transcript_speech_and_action_results_keep_their_actual_completeness',
    'agent_observations::tests::documented_tool_call_lists_and_wrapped_calls_are_normalized_once',
    'agent_observations::tests::one_slot_backpressure_recovers_with_known_gap_instead_of_starving_content',
    'agent_observations::tests::an_oversized_tool_batch_is_not_truncated_into_a_valid_prefix',
    'agent_observations_are_current_source_facts_and_model_actions_never_execute',
    'fresh_agent_context_preparation_is_scoped_pure_and_included_in_actual_create',
    'agent_observation_backpressure_keeps_actual_voice_live_and_reports_known_gap',
]


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def source_inputs():
    paths = {ROOT / name for name in (
        'Cargo.toml', 'Cargo.lock', 'dependencies/source-lock.json',
        'scripts/prepare-dependencies.sh', 'scripts/format.sh',
        'scripts/check-gate6-agent-observations.py', 'scripts/check-gate6-progress.py',
        'contracts/agent-observations-profile.md', 'docs/developer/agent-observations.md',
        'contracts/connect-uctp.md', 'contracts/schemas/uctp-envelope-2.schema.json',
        'contracts/openapi-events.json',
    )}
    lock = json.loads((ROOT / 'dependencies/source-lock.json').read_text())
    paths.update(ROOT / entry['path'] for entry in lock['patches'])
    for tree in ('crates', '.deps'):
        for folder, dirs, files in os.walk(ROOT / tree):
            dirs[:] = [d for d in dirs if d not in ('.git', 'target', 'node_modules')]
            paths.update(Path(folder) / name for name in files
                         if name.endswith('.rs') or name in ('Cargo.toml', 'Cargo.lock', 'build.rs'))
    return {str(path.relative_to(ROOT)): digest(path) for path in sorted(paths)}


def main():
    directory = ROOT / '.local/gate-6/agent-observations' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    before = source_inputs()
    steps, artifacts, inventories = [], {}, {}

    def run(name, command, manifest=None):
        log = directory / (name + '.log')
        env = dict(os.environ)
        if manifest:
            env['CARGO_MANIFEST_DIR'] = str(Path(manifest).parent)
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT)
        tests = re.findall(r'^test (.+?) \.\.\. ok$', log.read_text(errors='replace'), re.M)
        ignored = re.findall(r'^test (.+?) \.\.\. ignored(?:,.*)?$', log.read_text(errors='replace'), re.M)
        steps.append(dict(name=name, command=command, environment={'CARGO_MANIFEST_DIR': env['CARGO_MANIFEST_DIR']} if manifest else {},
                          passed=result.returncode == 0, exit_code=result.returncode, tests_passed=tests,
                          tests_ignored=ignored, log=str(log.relative_to(ROOT)), log_sha256=digest(log),
                          test_binary_sha256=digest(Path(command[0])) if manifest else None))
        print(json.dumps(dict(step=name, passed=steps[-1]['passed'], tests=len(tests), ignored=len(ignored))), flush=True)
        return steps[-1]['passed']

    ok = run('dependencies', ['bash', 'scripts/prepare-dependencies.sh'])
    if ok:
        ok = run('format', ['bash', 'scripts/format.sh', '--check'])
    files = re.findall(r'^\+\+\+ b/(.+\.rs)$', (ROOT / 'patches/rvoip/gate6-agent-observations.patch').read_text(), re.M)
    if ok:
        ok = run('upstream-format', ['rustfmt', '--edition', '2021', '--config', 'skip_children=true', '--check']
                 + ['.deps/rvoip/' + name for name in sorted(files)])
    if ok:
        command = ['cargo', 'test', '--locked', '--offline', '-j1', '--target-dir', 'target', '--manifest-path', '.deps/rvoip/Cargo.toml']
        command += [value for package in PACKAGES for value in ('-p', package)]
        command += ['--lib', '--test', 'mock_transport', '--no-run', '--message-format=json']
        ok = run('joint-build', command)
        for line in (directory / 'joint-build.log').read_text(errors='replace').splitlines():
            try:
                item = json.loads(line)
            except ValueError:
                continue
            if item.get('reason') == 'compiler-artifact' and item.get('executable'):
                artifacts[item['target']['name']] = dict(executable=item['executable'], manifest=item['manifest_path'],
                                                        sha256=digest(Path(item['executable'])))
        ok = ok and set(artifacts) == {'rvoip_core_traits', 'rvoip_core', 'rvoip_vapi', 'mock_transport'}
        steps[-1]['passed'] = ok
    for target in ('rvoip_core_traits', 'rvoip_vapi', 'rvoip_core', 'mock_transport'):
        if not ok:
            break
        artifact = artifacts[target]
        ok = run(target + '-inventory', [artifact['executable'], '--list', '--format', 'terse'], artifact['manifest'])
        inventory = re.findall(r'^(.+): test$', (directory / (target + '-inventory.log')).read_text(), re.M)
        inventories[target] = inventory
        if ok:
            ok = run(target + '-suite', [artifact['executable'], '--test-threads=1'], artifact['manifest'])
            observed = steps[-1]['tests_passed'] + steps[-1]['tests_ignored']
            missing = sorted(set(inventory) - set(observed))
            ok = ok and bool(inventory) and not missing and digest(Path(artifact['executable'])) == artifact['sha256']
            steps[-1].update(passed=ok, missing_inventory_cases=missing)
    if ok:
        ok = run('server-integration', ['cargo', 'check', '--locked', '--offline', '-j1', '-p', 'connect-server'])
    if ok:
        ok = run('progress', ['python3', 'scripts/check-gate6-progress.py'])
    if ok:
        ok = run('diff', ['git', 'diff', '--check'])
    observed = {test for step in steps for test in step['tests_passed']}
    missing = sorted(set(REQUIRED) - observed)
    ignored = {test for step in steps for test in step['tests_ignored']}
    unchanged = before == source_inputs()
    report = dict(task='G6-025',
                  qualification='Reusable provider-neutral agent observations and actual connection-stage attribution; immutable bounded context preparation for fresh AI legs. Actual isolated HTTP/WebSocket and Core media fixtures on local macOS arm64, full affected libraries and mock transport suite, locked Connect server compilation.',
                  not_qualified=['Live provider availability, remote context application or delivery.',
                                 'Durable Connect journal/HTTP/native service projection, current host audience and effect authority installation in G6-037/G6-038.',
                                 'Linux, designated remote/native/browser profiles, complete G6-024 media quality, and Gate 6 acceptance.'],
                  executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
                  rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(),
                  source_inputs_sha256=before, sources_unchanged=unchanged, artifacts=artifacts,
                  test_inventories=inventories, steps=steps,
                  runtime_tests=sum(len(step['tests_passed']) for step in steps), runtime_unique_tests=len(observed),
                  ignored_tests=sorted(ignored), ignored_disposition='Ignored standalone targets are not passes; inspect suite logs and parent-case evidence for any explicit child execution.',
                  required_cases=REQUIRED, missing_required_cases=missing,
                  passed=ok and unchanged and not missing and all(step['passed'] for step in steps))
    path = ROOT / 'docs/evidence/gate-6/agent-observations.json'
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(passed=report['passed'], tests=report['runtime_tests'], missing=missing, report=str(path.relative_to(ROOT)))), flush=True)
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
