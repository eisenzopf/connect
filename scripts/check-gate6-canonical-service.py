#!/usr/bin/env python3
"""Qualify the local canonical data dispatcher and bounded authenticated replies.

This does not qualify production listeners, device policy or Gate 6 acceptance.
Raw outputs and fixture state stay under ignored .local/.
"""
import datetime
import hashlib
import json
import pathlib
import platform
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sources():
    files = [ROOT / name for name in (
        'Cargo.toml', 'Cargo.lock', 'dependencies/source-lock.json',
        'scripts/check-gate6-canonical-service.py', 'scripts/check-gate6-contracts.py',
        'scripts/generate-gate6-schemas.py', 'scripts/generate-gate6-rust.py',
        'scripts/prepare-dependencies.py', 'scripts/lib/prepared_archive.py',
        'contracts/schemas/uctp-envelope-2.schema.json',
        'contracts/connect-uctp.md',
        'contracts/uctp-vectors/gate6-envelope.json',
        'docs/developer/canonical-service.md', '.deps/rvoip/Cargo.toml',
        '.deps/rvoip/crates/sip/rvoip-sip/src/auth/mod.rs',
        '.deps/rvoip/Cargo.lock', 'crates/connect-server/src/runtime.rs')]
    for directory in (
        'crates/connect-service', 'crates/connect-store', 'crates/connect-protocol',
        '.deps/rvoip/crates/uctp/rvoip-uctp',
        '.deps/rvoip/crates/foundation/rvoip-core-traits',
        '.deps/rvoip/crates/identity/auth-core',
    ):
        path = ROOT / directory
        files += list(path.rglob('*.rs')) + [path / 'Cargo.toml']
    lock = json.loads((ROOT / 'dependencies/source-lock.json').read_text())
    files += [ROOT / row['path'] for row in lock['patches']]
    return {str(path.relative_to(ROOT)): digest(path) for path in sorted(set(files))}


def main():
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory = ROOT / '.local/gate-6/canonical-service' / stamp
    directory.mkdir(parents=True)
    before = sources()
    cargo = ['cargo', 'test', '--locked', '--offline', '-j1', '--target-dir',
             'target', '--manifest-path', '.deps/rvoip/Cargo.toml']
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('upstream-format', ['rustfmt', '--edition', '2021', '--config', 'skip_children=true', '--check'] + [
            '.deps/rvoip/crates/' + name for name in (
                'foundation/rvoip-core-traits/src/identity.rs',
                'sip/rvoip-sip/src/auth/mod.rs',
                'uctp/rvoip-uctp/src/v2/mod.rs',
                'uctp/rvoip-uctp/src/v2/auth_dispatch.rs',
                'uctp/rvoip-uctp/src/v2/auth_peer.rs',
                'uctp/rvoip-uctp/src/v2/auth_peer/step_up.rs',
                'uctp/rvoip-uctp/src/v2/payloads.rs',
                'uctp/rvoip-uctp/src/v2/validation.rs',
                'uctp/rvoip-uctp/tests/v2_auth_peer.rs',
                'uctp/rvoip-uctp/tests/v2_step_up.rs',
                'uctp/rvoip-uctp/tests/support/reply_batches.rs')], 0),
        ('contracts', [str(ROOT / '.local/gate-6/verification-venv/bin/python'),
                       'scripts/check-gate6-contracts.py', '--output', str(directory / 'contracts.json')], 0),
        ('service', ['cargo', 'test', '--locked', '--offline', '-j1', '-p', 'connect-service'], 15),
        ('authenticated-replies', cargo + ['-p', 'rvoip-uctp', '--lib',
            '--test', 'v2_auth_peer', '--test', 'v2_step_up', '--test', 'v2_auth_dispatch'], 75),
        ('canonical-transports', cargo + ['-p', 'rvoip-uctp',
            '--test', 'v2_canonical_transports', '--test', 'v2_auth_recovery',
            '--test', 'v2_secure_profile', '--test', 'v2_peer_admission'], 100),
        ('identity-and-signature', cargo + ['-p', 'rvoip-core-traits', '-p', 'rvoip-auth-core', '--lib'], 100),
        ('progress', ['python3', 'scripts/check-gate6-progress.py'], 0),
        ('diff', ['git', 'diff', '--check'], 0),
    ]
    steps = []
    for name, command, minimum in commands:
        log = directory / (name + '.log')
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, stdout=output, stderr=subprocess.STDOUT)
        raw = log.read_text(errors='replace')
        tests = re.findall(r'^test (\S+) \.\.\. ok$', raw, re.M)
        binaries = {}
        for binary in re.findall(r'^\s*Running .*? \(([^)]+)\)', raw, re.M):
            path = ROOT / binary
            if path.is_file():
                binaries[binary] = digest(path)
        step = dict(name=name, command=command, exit_code=result.returncode,
                    log=str(log.relative_to(ROOT)), log_sha256=digest(log),
                    tests_passed=tests, test_binaries_sha256=binaries,
                    passed=result.returncode == 0 and len(tests) >= minimum)
        steps.append(step)
        print(json.dumps(dict(step=name, passed=step['passed'], tests=len(tests))), flush=True)
    required = {
        'canonical_signed_send_shares_receipt_with_control_and_history_is_ordered_and_scoped',
        'ordinary_authenticated_composition_rejects_forged_namespace_method_expiry_and_scopes',
        'reply_batches::ordered_signed_batch_retains_current_lease_through_last_frame_with_queued_revocation',
        'reply_batches::empty_overcount_and_overbyte_batches_reject_without_truncating',
        'reply_batches::forged_correlated_mutation_cannot_execute_or_consume_a_different_reply_family',
        'reply_batches::actual_tls_quic_preserves_multi_reply_order_correlation_and_signatures',
        'stepup_executes_one_held_command_and_signs_every_ordered_batch_reply_under_transition_lease',
    }
    observed = {name for step in steps for name in step['tests_passed']}
    missing = sorted(required - observed)
    after = sources()
    report = dict(
        format=1, task='G6-037', implementation_status='in-progress',
        qualification='Local signed canonical data dispatch to the durable Store; bounded ordered signed replies and transition leases',
        executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        platform=platform.platform(),
        source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        source_inputs_sha256=before, sources_unchanged=before == after,
        steps=steps, required_tests=sorted(required), missing_required_tests=missing,
        not_qualified=[
            'The dedicated SIP auth unit suite was interrupted during development-dependency compilation before tests. The SIP library and its exhaustive bearer label mapping compiled transitively in the authenticated reply and canonical transport suites; no SIP unit-suite pass is claimed.',
            'Production canonical listener installation and removal of competing stock command dispatch remain open.',
            'Controlled fixture keys and transport leases do not qualify production device enrollment, current key/credential metadata, per-Identity authority fences or retirement watchers.',
            'Conversation create/list/close, membership invitation semantics, generic operations with target floors and transactional revisions, retained subscriptions and session/media dispatch remain open.',
            'The durable dispatcher and actual TLS QUIC batch writer are qualified in separate local fixtures; full canonical QUIC/HTTP/SDK parity is not claimed.',
            'Remote/browser/live-provider checks, independent conformance, publication and all Gate 6 acceptance remain unqualified.',
        ],
        acceptance_qualified=False,
    )
    report['unqualified_candidates'] = []
    for relative, reason in (
        ('.local/gate-6/canonical-replies-service-2.log', 'History ACK rejected a valid long cursor before the Rust lexical/length fix.'),
        ('.local/gate-6/canonical-replies-upstream-3.log', 'SIP exhaustive CredentialKind match lacked the new Bearer label.'),
        ('.local/gate-6/canonical-replies-service-diagnostic.log', 'Confirmed the long-cursor ACK serialization failure.'),
        ('.local/gate-6/canonical-service-candidate-4.json', 'Revoked-credential error mapping failed; sources changed while correcting it. Dedicated SIP unit build was interrupted before tests.'),
    ):
        path = ROOT / relative
        if path.is_file():
            report['unqualified_candidates'].append(dict(path=relative, sha256=digest(path), reason=reason))
    report['passed'] = before == after and not missing and all(step['passed'] for step in steps)
    destination = ROOT / 'docs/evidence/gate-6/canonical-service.json'
    destination.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(report=str(destination.relative_to(ROOT)), passed=report['passed'],
                          rust_tests=sum(len(s['tests_passed']) for s in steps), missing=missing)))
    raise SystemExit(0 if report['passed'] else 1)


if __name__ == '__main__':
    main()
