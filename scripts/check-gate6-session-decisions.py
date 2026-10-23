#!/usr/bin/env python3
"""Qualify the installed pending-invitation decision slice of G6-037."""
import argparse
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
REQUIRED = {
    'peer_authority::tests::session_decisions::session_decision_reuses_exact_peer_proof_and_replays_one_original_receipt',
    'peer_authority::tests::session_decisions::session_decision_stale_revision_rolls_back_and_spoofed_participant_has_no_receipt',
    'peer_authority::tests::session_decisions::session_rejection_requires_invitee_and_replay_requires_current_authority',
    'peer_authority::tests::session_decisions::session_decision_expiring_behind_sql_wait_has_no_receipt_or_revision_effect',
    'canonical_session_decisions::tests::session_decision_parser_retains_every_typed_field',
    'canonical_session_decisions::tests::session_decision_parser_refuses_extra_resources_missing_ids_and_oversized_reason',
    'canonical_session_decisions::tests::legacy_application_profile_cannot_dispatch_new_invitation_decisions',
    'signed_invitation_decisions_share_http_receipts_without_changing_other_sessions',
}


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def source_inputs():
    paths = {ROOT / name for name in ('Cargo.toml', 'Cargo.lock', 'dependencies/source-lock.json',
             'scripts/check-gate6-session-decisions.py', 'scripts/format.sh',
             'scripts/prepare-dependencies.sh', 'contracts/openapi-session-decisions.json',
             'docs/developer/session-decisions.md', 'docs/evidence/gate-6/upstream.json',
             'contracts/connect-uctp.md', 'contracts/schemas/uctp-envelope-2.schema.json')}
    for directory in ('crates', 'examples'):
        paths.update((ROOT / directory).rglob('*.rs'))
        paths.update((ROOT / directory).rglob('Cargo.toml'))
    paths.update((ROOT / 'crates/connect-store/migrations').glob('*.sql'))
    upstream = json.loads((ROOT / 'docs/evidence/gate-6/upstream.json').read_text())
    assert upstream['passed'] and len(upstream['patches']) == 103
    for name, want in upstream['source_inputs_sha256'].items():
        if name.startswith(('.deps/', 'patches/')) or name == 'dependencies/source-lock.json':
            assert sha(ROOT / name) == want, name
            paths.add(ROOT / name)
    return {str(p.relative_to(ROOT)): sha(p) for p in sorted(paths)}


def validate_openapi(directory):
    from jsonschema import Draft202012Validator
    document = json.loads((ROOT / 'contracts/openapi-session-decisions.json').read_text())
    def validator(name):
        schema = {'$ref': '#/components/schemas/' + name, 'components': document['components']}
        Draft202012Validator.check_schema(schema)
        return Draft202012Validator(schema)
    request, response = validator('Operation'), validator('Response')
    samples = [sample for path in sorted(directory.glob('*.json')) for sample in json.loads(path.read_text())]
    assert len(samples) == 6
    assert {(s['request']['request']['decision'], s['case']) for s in samples} == {
        (d, case) for d in ('reject', 'cancel') for case in ('replay', 'spoof', 'stale')}
    for sample in samples:
        request.validate(sample['request'])
        response.validate(sample['response'])
        assert str(sample['status']) in document['paths']['/v1/operations']['post']['responses']
    vectors = []
    original = samples[0]['request']
    for field, bad in [('operation_id', ''), ('by_participant', 'bad space'), ('reason_code', 65536),
                       ('reason_code', -1), ('expected_revision', 9007199254740992), ('reason', 'x' * 257)]:
        value = json.loads(json.dumps(original)); value['request'][field] = bad
        assert list(request.iter_errors(value)), field
        vectors.append(field)
    value = json.loads(json.dumps(original)); value['request']['grant'] = '*'
    assert list(request.iter_errors(value)); vectors.append('unknown-grant')
    print(json.dumps(dict(passed=True, actual_http_exchanges=len(samples), refused_schema_vectors=len(vectors))))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--validate-openapi', type=Path)
    args = parser.parse_args()
    if args.validate_openapi:
        validate_openapi(args.validate_openapi)
        return
    directory = ROOT / '.local/gate-6/session-decisions' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    samples = directory / 'samples'; samples.mkdir()
    before = source_inputs()
    spec = importlib.util.spec_from_file_location('statuses', ROOT / 'scripts/check-gate6-websocket-media.py')
    statuses = importlib.util.module_from_spec(spec); spec.loader.exec_module(statuses)
    steps = []
    env = dict(os.environ, CONNECT_G6_SESSION_DECISION_SAMPLES=str(samples))
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('store-authority', ['cargo', 'test', '--locked', '--offline', '-j2', '-p', 'connect-store', '--lib', 'peer_authority::tests', '--', '--test-threads=1'], 4),
        ('service-parser', ['cargo', 'test', '--locked', '--offline', '-j2', '-p', 'connect-service', '--lib', 'canonical_session_decisions', '--', '--test-threads=1'], 3),
        ('signed-runtime', ['cargo', 'test', '--locked', '--offline', '-j2', '-p', 'connect-server', '--test', 'gate6_canonical_runtime', '--', '--test-threads=1'], 12),
        ('openapi', [str(ROOT / '.local/gate-6/verification-venv/bin/python'), 'scripts/check-gate6-session-decisions.py', '--validate-openapi', str(samples)], 0),
        ('progress', ['python3', 'scripts/check-gate6-progress.py'], 0),
        ('diff', ['git', 'diff', '--check'], 0),
    ]
    for name, command, minimum in commands:
        log = directory / (name + '.log')
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT)
        raw = log.read_text(errors='replace'); tests = statuses.passed_tests(raw)
        binaries = {str(Path(p).relative_to(ROOT)): sha(p) for p in
                    (ROOT / value for value in re.findall(r'^\s+Running .+ \(([^)]+)\)$', raw, re.M))}
        passed = result.returncode == 0 and len(tests) >= minimum
        step = dict(name=name, command=command, passed=passed, exit_code=result.returncode,
                    tests_passed=tests, log=str(log.relative_to(ROOT)), log_sha256=sha(log), test_binaries_sha256=binaries)
        steps.append(step)
        print(json.dumps(dict(step=name, passed=passed, tests=len(tests))), flush=True)
        if not passed:
            break
    observed = {test for step in steps for test in step['tests_passed']}
    missing = sorted(REQUIRED - observed)
    unchanged = before == source_inputs()
    binaries = {p: digest for step in steps for p, digest in step['test_binaries_sha256'].items()}
    if all(step['passed'] for step in steps) and len(steps) == len(commands):
        binaries['target/debug/connect'] = sha(ROOT / 'target/debug/connect')
    result = dict(task='G6-037', implementation_status='partial-invitation-decisions',
        passed=len(steps) == len(commands) and all(s['passed'] for s in steps) and unchanged and not missing,
        executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        qualification='Current-source local macOS arm64 pending invitation decisions via held Store admission, production signed QUIC SDK, current explicit HTTP facade, original receipt recovery and bounded refusal. Full canonical runtime regressions repeat; no full upstream suite total is combined.',
        not_qualified=['Remaining G6-037 command/application-profile installation and obsolete path removal, including target /v2/commands.', 'G6-038 general Session projection, acceptance, join/leave/upgrade/end and media orchestration.', 'Linux, live browser/remote/provider lanes, full Gate 6 acceptance and publication.'],
        source_inputs_sha256=before, sources_unchanged=unchanged, required_tests=sorted(REQUIRED), missing_required_tests=missing,
        runtime_tests=sum(len(s['tests_passed']) for s in steps), steps=steps,
        executable_sha256=binaries, actual_http_samples_sha256={str(p.relative_to(ROOT)):sha(p) for p in samples.glob('*.json')},
        upstream_checkpoint_sha256=sha(ROOT / 'docs/evidence/gate-6/upstream.json'), schema_version=34, source_lock_patches=103)
    if any(s['name'] == 'openapi' and s['passed'] for s in steps):
        result['openapi_verification'] = json.loads((directory / 'openapi.log').read_text())
    path = ROOT / 'docs/evidence/gate-6/session-decisions.json'
    path.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({k:result[k] for k in ('passed','runtime_tests','sources_unchanged','missing_required_tests')}))
    raise SystemExit(0 if result['passed'] else 1)


if __name__ == '__main__':
    main()
