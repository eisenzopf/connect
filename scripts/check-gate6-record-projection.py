#!/usr/bin/env python3
"""Qualify persisted source manifests and protected projection for open G6-027."""
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
    'invalid_snapshot_is_not_stored_or_announced',
    'tests::migration_defines_expected_table',
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
    for name in ('scripts/validate-gate6-record-postgres-run.py', 'scripts/run-gate6-local-record-postgres.py', 'scripts/validate-gate6-record-view.py', 'scripts/check-gate6-record-projection.py', 'scripts/check-gate6-record-lifecycle.py', 'scripts/check-gate6-record-storage.py', 'contracts/connect-records.md', 'docs/developer/conversation-records.md', 'docs/developer/record-lifecycle.md', 'docs/developer/record-projection.md'):
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
    directory = ROOT / '.local/gate-6/record-projection' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
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
        command += ['--features', 'rvoip-vcon-postgres/core-store,rvoip-vcon-postgres/live-tests', '--lib', '--test', 'mock_transport', '--test', 'vcon_emission', '--no-run', '--message-format=json']
        ok = run('joint-build', command)
        for line in (directory / 'joint-build.log').read_text(errors='replace').splitlines():
            try:
                item = json.loads(line)
            except ValueError:
                continue
            if item.get('reason') == 'compiler-artifact' and item.get('executable'):
                artifacts[item['target']['name']] = dict(executable=item['executable'], manifest=item['manifest_path'],
                                                        sha256=digest(Path(item['executable'])))
        ok = ok and set(artifacts) == {'rvoip_core_traits', 'rvoip_core', 'rvoip_vapi', 'rvoip_vcon_postgres', 'mock_transport', 'vcon_emission'}
        steps[-1]['passed'] = ok
    for target in ('rvoip_core_traits', 'rvoip_vapi', 'rvoip_core', 'mock_transport', 'vcon_emission', 'rvoip_vcon_postgres'):
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
        ok = run('view-schema', [schema_python, 'scripts/validate-gate6-record-view.py', str(directory / 'schema-views')])
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
                  qualification='Persisted complete per-item source/audience/provenance/dependency and retention manifests, body/manifest digest-bound stored reads, protected current-policy projection and canonical reference rewriting, unavailable media and private data refusal. Actual memory-store reconstruction, current-policy writer lifetime and storage custody fixtures; full current engine/voice/export regressions and locked Connect server check on local macOS arm64. The optional database/live fixture compiles; the explicitly selected bounded local Docker database lane and its cleanup are recorded separately.',
                  not_qualified=['Full G6-027: actual source/capture-policy integration, physical retention/selective redaction and durable host operation/current-policy installation; owned explicit finalization/unavailable/recovery states; configured signed-source verification; actual canonical fetch, large-record and authorized media URL delivery. The current projection lease is a reusable controlled-policy fixture, not production Connect authorization.',
                                 'Remote or existing database behavior and crash/disk persistence are not qualified by a fresh local temporary database fixture.',
                                 'Live provider, production Connect durable service/journal/HTTP/native authority, Linux/remote/browser profiles and Gate 6 acceptance.'],
                  executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
                  rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(),
                  source_inputs_sha256=before, sources_unchanged=unchanged, artifacts=artifacts,
                  test_inventories=inventories, steps=steps,
                  runtime_tests=sum(len(step['tests_passed']) for step in steps), runtime_unique_tests=len(observed),
                  ignored_tests=sorted(ignored), ignored_disposition='The live database case is excluded from the ordinary library run. If --local-postgres is selected it is separately executed against an owned temporary fixture, and only its actual Rust log is counted. Remaining ignored targets are not passes.',
                  required_cases=required, missing_required_cases=missing, local_database=local_database,
                  passed=ok and unchanged and not missing and all(step['passed'] for step in steps))
    path = ROOT / 'docs/evidence/gate-6/record-projection.json'
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(passed=report['passed'], tests=report['runtime_tests'], missing=missing, report=str(path.relative_to(ROOT)))), flush=True)
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
