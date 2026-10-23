#!/usr/bin/env python3
"""Qualify persisted local peer credential/key facts and admin HTTP contracts.

This is not production peer-policy, listener, provider or Gate 6 qualification.
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

import jsonschema

ROOT = pathlib.Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sources():
    paths = [ROOT / name for name in (
        'Cargo.toml', 'Cargo.lock', 'dependencies/source-lock.json',
        'scripts/check-gate6-peer-credentials.py', 'scripts/check-gate6-contracts.py',
        'scripts/prepare-dependencies.py', 'scripts/lib/prepared_archive.py',
        'docs/developer/peer-credentials.md', 'docs/developer/canonical-service.md',
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
    directory = ROOT / '.local/gate-6/peer-credentials' / stamp
    samples = directory / 'http-samples'
    directory.mkdir(parents=True)
    before = sources()
    env = dict(os.environ, CONNECT_GATE6_HTTP_SAMPLES=str(samples))
    cargo = ['cargo', 'test', '--locked', '--offline', '-j1']
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('protocol', cargo + ['-p', 'connect-protocol'], 1),
        ('store', cargo + ['-p', 'connect-store', '--lib', '--test', 'gate6_schema',
                          '--test', 'gate6_provisioning', '--test', 'gate6_peer_credentials'], 220),
        ('service', cargo + ['-p', 'connect-service'], 21),
        ('http', cargo + ['-p', 'connect-server', '--test', 'gate6_provisioning',
                         '--test', 'setup_vault_auth'], 8),
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
                        'signing-key-revoke.json', 'device-revoke.json'}
    missing_samples = sorted(required_samples - {path.name for path in samples.glob('*.json')})
    required_tests = {
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
        format=1, task='G6-037', implementation_status='in-progress', schema=28,
        qualification='Persisted issuer facts, Identity/Device EdDSA/ES256 key ownership and explicit credential bindings; stable Identity through actual canonical refresh, bounded admin receipts and actual loopback HTTP/OpenAPI exchanges',
        executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        platform=platform.platform(), source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        source_inputs_sha256=before, sources_unchanged=before == after,
        python=sys.version, jsonschema=importlib.metadata.version('jsonschema'),
        steps=steps, required_tests=sorted(required_tests), missing_required_tests=missing_tests,
        http_openapi_samples=checked, missing_http_samples=missing_samples,
        contract_errors=contract_errors,
        ignored_test_disposition={'transitions::tests::crash_worker': 'Explicitly invoked in 27 child processes by the two required passing parent crash tests, requiring exit 86 at each selected boundary. The standalone ignored target is not counted as a test pass.'},
        not_qualified=[
            'Current key/metadata lookups are snapshots, not owned execution or delivery guards. Cross-facade/process authority fences and binding retirement watchers remain open.',
            'Production peer Device-to-signed-key validation, canonical listener installation, full command/extension coverage and canonical QUIC/HTTP/SDK parity remain open.',
            'Device/key registry revocation does not qualify active native media cleanup or current endpoint/session projection; those integrations remain open.',
            'Remote/browser/live-provider checks, independent conformance, publication and all Gate 6 acceptance remain unqualified.',
        ], acceptance_qualified=False,
    )
    report['unqualified_candidates'] = []
    for relative, reason in (
        ('.local/gate-6/peer-credentials-candidate-1.json', 'Unsealed intermediate candidate: HTTP compile ran while stable Identity/typed credential binding changes were in progress.'),
        ('.local/gate-6/peer-credentials-candidate-2.json', 'All selected tests and ten HTTP samples passed, but source changed during final documentation review and removal of an unused canonical-path legacy principal construction.'),
    ):
        candidate = ROOT / relative
        if candidate.is_file():
            report['unqualified_candidates'].append(dict(path=relative, sha256=digest(candidate), reason=reason))
    report['passed'] = before == after and all(step['passed'] for step in steps) and not missing_tests and not missing_samples and not contract_errors and len(checked) >= 10
    path = ROOT / 'docs/evidence/gate-6/peer-credentials.json'
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(passed=report['passed'], rust_tests=sum(len(s['tests_passed']) for s in steps), http_samples=len(checked), missing_tests=missing_tests, missing_samples=missing_samples)))
    raise SystemExit(0 if report['passed'] else 1)


if __name__ == '__main__':
    main()
