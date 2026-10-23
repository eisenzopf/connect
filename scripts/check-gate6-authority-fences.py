#!/usr/bin/env python3
"""Qualify the local cooperative authority fence foundation and retained blocking work.

Production policy, complete writer coverage and Gate 6 acceptance remain open.
"""
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
import tempfile

import jsonschema

ROOT = pathlib.Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sources():
    paths = [ROOT / name for name in (
        'Cargo.toml', 'Cargo.lock', 'dependencies/source-lock.json',
        'scripts/check-gate6-authority-fences.py', 'scripts/check-gate6-contracts.py',
        'scripts/generate-gate6-event-reference.py', 'docs/developer/authority-fences.md',
        'scripts/prepare-dependencies.py', 'scripts/lib/prepared_archive.py',
        'docs/developer/peer-credentials.md', 'docs/developer/canonical-service.md',
        'docs/developer/events.md', 'docs/developer/provisioning.md',
        'docs/developer/messages.md', 'docs/developer/membership.md',
        'docs/developer/attachments-and-receipts.md', 'docs/developer/operations.md',
        'docs/decisions/014-gate-6-contract-version.md',
        'contracts/openapi-provisioning.json', 'contracts/openapi-events.json',
        'contracts/gate6-event-writers.json',
        'contracts/schemas/uctp-envelope-2.schema.json',
        'contracts/uctp-vectors/gate6-envelope.json')]
    for folder in ('crates/connect-protocol', 'crates/connect-store',
                   'crates/connect-service', 'crates/connect-server'):
        paths += [p for p in (ROOT / folder).rglob('*')
                  if p.is_file() and (p.suffix in ('.rs', '.sql') or p.name == 'Cargo.toml')]
    paths += [ROOT / item['path'] for item in json.loads((ROOT / 'dependencies/source-lock.json').read_text())['patches']]
    return {str(p.relative_to(ROOT)): digest(p) for p in sorted(set(paths))}


def main():
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory = ROOT / '.local/gate-6/authority-fences' / stamp
    samples = directory / 'http-samples'
    directory.mkdir(parents=True)
    before = sources()
    with tempfile.TemporaryDirectory(prefix='connect-authority-case-') as probe:
        lower = pathlib.Path(probe) / 'state.sqlite3'
        lower.touch()
        fixture_case_alias_checked = lower.with_name('STATE.SQLITE3').exists()
    env = dict(os.environ, CONNECT_GATE6_HTTP_SAMPLES=str(samples))
    cargo = ['cargo', 'test', '--locked', '--offline', '-j1']
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('protocol', cargo + ['-p', 'connect-protocol'], 1),
        ('store', cargo + ['-p', 'connect-store'], 250),
        ('service', cargo + ['-p', 'connect-service'], 21),
        ('http', cargo + ['-p', 'connect-server', '--test', 'gate6_provisioning',
                         '--test', 'setup_vault_auth', '--test', 'first_run_schema_boundary'], 9),
        ('contracts', [str(ROOT / '.local/gate-6/verification-venv/bin/python'),
                       'scripts/check-gate6-contracts.py', '--output', str(directory / 'contracts.json')], 0),
        ('progress', ['python3', 'scripts/check-gate6-progress.py'], 0),
        ('diff', ['git', 'diff', '--check'], 0),
    ]
    steps = []
    for name, command, minimum in commands:
        log = directory / (name + '.log')
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT)
        raw = log.read_text(errors='replace')
        tests = re.findall(r'^test (\S+) \.\.\. ok$', raw, re.M)
        binaries = {}
        for name_in_log in re.findall(r'Running [^\n]+\((target/debug/deps/[^)]+)\)', raw):
            path = ROOT / name_in_log
            if path.is_file():
                binaries[name_in_log] = digest(path)
        step = dict(name=name, command=command, exit_code=result.returncode,
                    tests_passed=tests, test_binaries_sha256=binaries,
                    log=str(log.relative_to(ROOT)), log_sha256=digest(log),
                    passed=result.returncode == 0 and len(tests) >= minimum)
        steps.append(step)
        print(json.dumps(dict(step=name, passed=step['passed'], tests=len(tests))), flush=True)
    api = json.loads((ROOT / 'contracts/openapi-provisioning.json').read_text())
    checked, contract_errors = [], []
    for path in sorted(samples.glob('*.json')):
        sample = json.loads(path.read_text())
        try:
            route = api['paths'][sample['route']]['post']
            for value, schema in (
                (sample['request'], route['requestBody']['content']['application/json']['schema']),
                (sample['response'], route['responses'][str(sample['status'])]['content']['application/json']['schema']),
            ):
                jsonschema.Draft202012Validator(dict(schema, components=api['components'])).validate(value)
            checked.append(dict(route=sample['route'], status=sample['status'],
                                path=str(path.relative_to(ROOT)), sha256=digest(path)))
        except (AssertionError, KeyError, jsonschema.ValidationError) as error:
            contract_errors.append(dict(sample=path.name, error=str(error)))
    required_samples = {'signing-key-enroll.json', 'signing-key-stale.json',
                        'signing-key-revoke.json', 'device-revoke.json', 'signing-key-busy.json'}
    missing_samples = sorted(required_samples - {path.name for path in samples.glob('*.json')})
    required_tests = {
        'authority_fences::tests::writer_intent_stops_new_readers_held_proof_reuses_without_reentry_and_other_identity_proceeds',
        'authority_fences::tests::cancellation_deadline_and_partial_acquisition_release_every_scope',
        'authority_fences::tests::namespace_privacy_symlink_alias_hardlink_refusal_and_foreign_guard_are_enforced',
        'authority_fences::tests::actual_child_process_revocation_waits_for_parent_delivery_guard_and_admin_open_preserves_runtime',
        'authority_fences::tests::actual_child_death_releases_kernel_reader_without_persistent_phantom_authority',
        'authority_fences::tests::actual_key_and_device_retirement_wait_for_current_identity_delivery',
        'authority_fences::tests::membership_removal_and_other_member_revocation_wait_for_snapshot_delivery',
        'transitions::tests::administration_child_get_retry_and_claim_preserve_live_transition_worker',
        'messages::authority_tests::cancelled_handler_retains_authority_until_uncancellable_store_work_settles',
        'actual_canonical_peer_refresh_keeps_identity_and_requires_a_new_explicit_credential_binding',
        'key_ownership_validity_revisions_and_retirement_are_not_overridden_by_retry',
        'fresh_schema_snapshot_digest_includes_keys_bindings_and_real_issuance',
        'key_and_credential_history_bounds_do_not_evict_old_bindings_or_grant_the_overflow',
        'persisted_issuance_and_ed25519_binding_survive_restart_and_refresh_without_borrowing_keys',
        'es256_keys_require_exact_owner_algorithm_current_credential_device_and_validity',
        'future_and_expired_keys_never_resolve_from_a_still_current_credential',
        'signing_key_and_device_routes_enforce_admin_target_replay_and_closed_contracts',
        'transitions::tests::process_crashes_cover_durable_boundaries_without_duplicate_callbacks_or_partial_local_commit',
        'transitions::tests::process_crashes_keep_cancellation_evidence_reconciliation_and_cleanup_atomic',
    }
    observed = {test for step in steps for test in step['tests_passed']}
    missing_tests = sorted(required_tests - observed)
    after = sources()
    report = dict(
        format=1, task='G6-037', implementation_status='in-progress', schema=29,
        qualification='Local macOS arm64 cooperative cross-process fence foundation; bounded admission and cleanup, protected writer hooks, administration opening, canceled blocking work, current peer credentials and real loopback HTTP/OpenAPI busy retry',
        executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        platform=platform.platform(),
        fixture_case_alias_branch=dict(executed=fixture_case_alias_checked,
                                      disposition='Case-insensitive temporary fixture filesystem; required namespace case-alias branch executes.' if fixture_case_alias_checked else 'Case-sensitive temporary fixture filesystem; case-alias branch skipped, not qualified.'),
        source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        source_inputs_sha256=before, sources_unchanged=before == after,
        python=sys.version, jsonschema=importlib.metadata.version('jsonschema'),
        steps=steps, required_tests=sorted(required_tests), missing_required_tests=missing_tests,
        http_openapi_samples=checked, missing_http_samples=missing_samples,
        contract_errors=contract_errors,
        ignored_test_disposition={
            'transitions::tests::crash_worker': 'Explicitly invoked at 27 crash boundaries by the two required parent crash tests (expected exit 86), plus one successful administration child invoked by its required parent. The standalone ignored target is not counted as a pass.',
            'authority_fences::tests::process_worker': 'Explicitly invoked in two child processes by required passing parents: a real revocation writer and a reader killed while holding a kernel lock. The standalone ignored target is not counted as a pass.',
        },
        not_qualified=[
            'The owned fence foundation and selected writers are qualified locally. Complete writer coverage (ordinary close/session/task/provider authority, protected topology callbacks and runtime recovery), production policy and binding retirement watchers remain open.',
            'Production peer Device-to-signed-key validation, canonical listener installation, full command/extension coverage and canonical QUIC/HTTP/SDK parity remain open.',
            'Device/key registry revocation does not qualify active native media cleanup or current endpoint/session projection; those integrations remain open.',
            'Remote/browser/live-provider checks, independent conformance, publication and all Gate 6 acceptance remain unqualified.',
        ], acceptance_qualified=False,
    )
    report['unqualified_candidates'] = []
    for relative, reason in (
        ('.local/gate-6/authority-fences-candidate-1.json', 'All 410 selected Rust tests, 179 schema vectors and 11 HTTP/OpenAPI samples passed; intermediate source changed for canonical/blob/case-alias fixes, and the runner incorrectly expected ten instead of nine selected HTTP/CLI-boundary tests. Not sealed.'),
        ('.local/gate-6/authority-fences-tests-1.log', 'Initial fence candidate failed fixture setup and hardlink validation before the fixes.'),
        ('.local/gate-6/authority-blocking-test-1.log', 'Initial cancellation fixture failed compilation because four existing Service test constructors needed the new test hook field.'),
        ('.local/gate-6/authority-fences-tests-3.log', 'Expanded fixture compile errors: lookup algorithm type and snapshot revision field corrected.'),
        ('.local/gate-6/authority-fences-tests-4.log', 'Expanded fixtures supplied revision 1 for newly created revision-0 resources; corrected to current metadata.'),
    ):
        candidate = ROOT / relative
        if candidate.is_file():
            report['unqualified_candidates'].append(dict(path=relative, sha256=digest(candidate), reason=reason))
    report['passed'] = before == after and all(step['passed'] for step in steps) and not missing_tests and not missing_samples and not contract_errors and len(checked) >= 11
    path = ROOT / 'docs/evidence/gate-6/authority-fences.json'
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(passed=report['passed'], rust_tests=sum(len(s['tests_passed']) for s in steps), http_samples=len(checked), missing_tests=missing_tests, missing_samples=missing_samples)))
    raise SystemExit(0 if report['passed'] else 1)


if __name__ == '__main__':
    main()
