#!/usr/bin/env python3
"""Qualify owned record finalization and original-document recovery for open G6-027."""
import datetime
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
PACKAGES = ['rvoip-core-traits', 'rvoip-core', 'rvoip-vapi', 'rvoip-vcon-postgres']
base_spec = importlib.util.spec_from_file_location('agent_profile', ROOT / 'scripts/check-gate6-agent-observations.py')
base = importlib.util.module_from_spec(base_spec)
base_spec.loader.exec_module(base)
REQUIRED = base.REQUIRED + [
    'store::vcon_store::diagnostic_tests::colliding_logical_ids_never_cross_tenant_conversation_or_session_storage_scope',
    'store::vcon_store::diagnostic_tests::concurrent_puts_publish_whole_scoped_entries_and_exhaustion_has_no_partial_write',
    'store::vcon_store::diagnostic_tests::forged_vcon_hash_never_returns_bytes_and_preserves_the_current_record',
    'store::vcon_store::diagnostic_tests::stored_hash_uses_vcon_sha512_base64url_format',
    'store::vcon_store::diagnostic_tests::vcon_store_debug_never_renders_urls_hashes_keys_or_payloads',
    'ending_a_session_emits_vcon_ready_with_resolvable_handle',
    'session_vcon_handle_exposes_live_builder_during_session',
    'invalid_snapshot_preserves_metadata_outcome_without_announcing_invalid_capture',
    'tests::migration_defines_expected_table',
    'records::capture::tests::bounded_capture_reports_omissions_and_never_widens_frozen_audience',
    'records::capture::retention_tests::independent_named_retention_and_external_audio_reference_never_invent_captured_bytes',
    'gate6_explicit_record_capture_preserves_private_frozen_message_audience_and_outcome',
    'store::vcon_store::diagnostic_tests::original_document_retry_is_atomic_and_changed_body_manifest_owner_or_tombstone_conflicts',
    'finalization_recovery::failed_store_is_explicit_and_same_session_retry_recovers_one_metadata_only_record',
    'finalization_recovery::cancelled_after_real_store_commit_retries_original_document_without_duplicate_or_detached_work',
    'finalization_recovery::bounded_store_timeout_preserves_unknown_and_other_session_can_finalize',
    'finalization_recovery::invalid_capture_and_fabricated_store_handle_never_announce_readiness',
    'records::tests::persisted_complete_manifest_roundtrips_and_never_invents_audience_or_source_identity',
    'records::tests::audience_projection_prunes_private_dependencies_remaps_all_public_refs_and_withholds_raw_media_urls',
    'records::tests::policy_guard_and_record_read_survive_preparation_and_cancelled_or_foreign_views_cannot_borrow_them',
    'records::tests::retention_is_independent_and_expired_media_keeps_valid_unavailable_metadata',
    'records::tests::transfer_channel_history_and_analysis_indices_are_remapped_after_hidden_leading_party_and_dialog',
    'store::vcon_store::diagnostic_tests::read_lease_fences_real_async_delivery_and_fair_delete_while_other_record_proceeds',
    'store::vcon_store::diagnostic_tests::cancelled_pending_delete_releases_writer_intent_and_bad_scope_or_revision_cannot_mutate',
    'store::vcon_store::diagnostic_tests::lost_delete_response_recovers_only_the_same_original_intent_and_redacts_diagnostics',
]


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def source_inputs():
    result = base.source_inputs()
    for name in ('scripts/validate-gate6-record-postgres-run.py', 'scripts/run-gate6-local-record-postgres.py', 'scripts/validate-gate6-record-view.py', 'scripts/check-gate6-record-projection.py', 'scripts/check-gate6-record-finalization.py', 'scripts/validate-gate6-record-capture.py', 'docs/developer/record-finalization.md', 'scripts/check-gate6-record-lifecycle.py', 'scripts/check-gate6-record-storage.py', 'contracts/connect-records.md', 'docs/developer/conversation-records.md', 'docs/developer/record-lifecycle.md', 'docs/developer/record-projection.md'):
        result[name] = digest(ROOT / name)
    # Bind include_str!/include_bytes! assets too, including the Postgres SQL
    # migration and canonical schema fixtures. Dependency archives are caches.
    for tree in ('crates', '.deps', 'contracts'):
        for folder, dirs, files in os.walk(ROOT / tree):
            dirs[:] = [d for d in dirs if d not in ('.git', '.archives', 'target', 'node_modules', '.local')]
            for name in files:
                path = Path(folder) / name
                if path.is_file():
                    result[str(path.relative_to(ROOT))] = digest(path)
    return dict(sorted(result.items()))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--local-postgres', action='store_true', help='Run a fresh bounded local Docker database fixture; no existing endpoint is used.')
    args = parser.parse_args()
    required = REQUIRED + (['tests::live_put_get_delete_list_and_hash'] if args.local_postgres else [])
    directory = ROOT / '.local/gate-6/record-finalization' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    before = source_inputs()
    steps, artifacts, inventories = [], {}, {}

    def run(name, command, manifest=None):
        log = directory / (name + '.log')
        env = dict(os.environ)
        if manifest:
            env['CONNECT_G6_RECORD_VIEW_EXPORT_DIR'] = str(directory / 'schema-views')
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
    files = re.findall(r'^\+\+\+ b/(.+\.rs)$', (ROOT / 'patches/rvoip/gate6-conversation-records.patch').read_text(), re.M)
    if ok:
        ok = run('upstream-format', ['rustfmt', '--edition', '2021', '--config', 'skip_children=true', '--check']
                 + ['.deps/rvoip/' + name for name in sorted(files)])
    if ok:
        command = ['cargo', 'test', '--locked', '--offline', '-j2', '--target-dir', 'target', '--manifest-path', '.deps/rvoip/Cargo.toml']
        command += [value for package in PACKAGES for value in ('-p', package)]
        command += ['--features', 'rvoip-vcon-postgres/core-store,rvoip-vcon-postgres/live-tests', '--lib', '--test', 'mock_transport', '--test', 'vcon_emission', '--test', 'messaging', '--no-run', '--message-format=json']
        ok = run('joint-build', command)
        for line in (directory / 'joint-build.log').read_text(errors='replace').splitlines():
            try:
                item = json.loads(line)
            except ValueError:
                continue
            if item.get('reason') == 'compiler-artifact' and item.get('executable'):
                artifacts[item['target']['name']] = dict(executable=item['executable'], manifest=item['manifest_path'],
                                                        sha256=digest(Path(item['executable'])))
        ok = ok and set(artifacts) == {'rvoip_core_traits', 'rvoip_core', 'rvoip_vapi', 'rvoip_vcon_postgres', 'mock_transport', 'vcon_emission', 'messaging'}
        steps[-1]['passed'] = ok
    for target in ('rvoip_core_traits', 'rvoip_vapi', 'rvoip_core', 'mock_transport', 'vcon_emission', 'messaging', 'rvoip_vcon_postgres'):
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
    local_database = None
    if ok and args.local_postgres:
        artifact = artifacts['rvoip_vcon_postgres']
        ok = run('local-postgres', ['python3', 'scripts/run-gate6-local-record-postgres.py', artifact['executable'], '--evidence-dir', str(directory / 'local-postgres')])
        local_database = json.loads((directory / 'local-postgres/report.json').read_text())
        ok = ok and local_database['passed'] and digest(Path(artifact['executable'])) == artifact['sha256']
        steps[-1]['passed'] = ok
        if ok:
            # Count the actual compiled Rust result, never the wrapper's output.
            case_log = directory / 'local-postgres/test.log'
            cases = re.findall(r'^test (.+?) \.\.\. ok$', case_log.read_text(), re.M)
            ok = cases == ['tests::live_put_get_delete_list_and_hash']
            steps[-1].update(passed=ok, runner_log=steps[-1]['log'], runner_log_sha256=steps[-1]['log_sha256'],
                             log=str(case_log.relative_to(ROOT)), log_sha256=digest(case_log), tests_passed=cases,
                             test_binary_sha256=artifact['sha256'])
        if ok:
            ok = run('local-postgres-evidence', ['python3', 'scripts/validate-gate6-record-postgres-run.py', str(directory / 'local-postgres/report.json')])
    if ok:
        schema_python = os.environ.get('CONNECT_G6_SCHEMA_PYTHON', str(ROOT / '.local/gate-6/verification-venv/bin/python'))
        ok = run('view-schema', [schema_python, 'scripts/validate-gate6-record-capture.py', str(directory / 'schema-views')])
    if ok:
        ok = run('server-integration', ['cargo', 'check', '--locked', '--offline', '-j1', '-p', 'connect-server'])
    if ok:
        ok = run('progress', ['python3', 'scripts/check-gate6-progress.py'])
    if ok:
        ok = run('diff', ['git', 'diff', '--check'])
    observed = {test for step in steps for test in step['tests_passed']}
    missing = sorted(set(required) - observed)
    ignored = {test for step in steps for test in step['tests_ignored']} - observed
    unchanged = before == source_inputs()
    report = dict(task='G6-027', implementation_status='in-progress', full_task_qualification=False,
                  qualification='Actual explicitly selected message and attributed agent capture, default metadata-only Session outcome including invalid raw-capture fallback, bounded omissions and source audience/provenance preservation, plus caller-owned bounded Session record finalization, explicit capture failure and unknown store outcomes, frozen original-document retry, exact atomic create binding and no deleted-record resurrection. Actual Orchestrator failure/cancellation/concurrent-retry/independent-progress fixtures plus full affected engine, transport and canonical export regressions, independent view schema and locked Connect server check on local macOS arm64. The separately selected owned temporary database lane records its exact configuration and cleanup.',
                  not_qualified=['Full G6-027: complete lineage/media/provider-reference and conversation-only capture mapping, physical retention/selective redaction and durable host operation/current-policy installation; configured signed-source verification; actual canonical fetch, large-record and authorized media URL delivery. Finalization intents are process-local, not a pending-capture restart journal.',
                                 'Remote or existing database behavior and crash/disk persistence are not qualified by a fresh local temporary database fixture.',
                                 'Live provider, production Connect durable service/journal/HTTP/native authority, Linux x86_64/remote/browser profiles and Gate 6 acceptance.'],
                  executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
                  rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(),
                  source_inputs_sha256=before, sources_unchanged=unchanged, artifacts=artifacts,
                  test_inventories=inventories, steps=steps,
                  runtime_tests=sum(len(step['tests_passed']) for step in steps), runtime_unique_tests=len(observed),
                  ignored_tests=sorted(ignored), ignored_disposition='The live database case is excluded from the ordinary library run. If --local-postgres is selected it is separately executed against an owned temporary fixture, and only its actual Rust log is counted. Remaining ignored targets are not passes.',
                  required_cases=required, missing_required_cases=missing, local_database=local_database,
                  passed=ok and unchanged and not missing and all(step['passed'] for step in steps))
    path = ROOT / 'docs/evidence/gate-6/record-finalization.json'
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(passed=report['passed'], tests=report['runtime_tests'], missing=missing, report=str(path.relative_to(ROOT)))), flush=True)
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
