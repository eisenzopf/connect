#!/usr/bin/env python3
"""Qualify the partial G6-026 authenticated canonical backend on local fixtures."""
import datetime
import hashlib
import json
import pathlib
import platform
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
UPSTREAM = ROOT / '.deps/rvoip'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sources():
    paths = [ROOT / name for name in (
        'dependencies/source-lock.json', 'scripts/check-gate6-auth-dispatch.py',
        'scripts/prepare-dependencies.py', 'scripts/lib/prepared_archive.py',
        'docs/developer/authenticated-dispatch.md', 'contracts/connect-uctp.md',
        'contracts/schemas/uctp-envelope-2.schema.json',
        '.deps/rvoip/Cargo.toml', '.deps/rvoip/Cargo.lock')]
    for name in ('identity/auth-core', 'foundation/rvoip-core-traits',
                 'foundation/rvoip-core', 'uctp/rvoip-uctp',
                 'uctp/rvoip-quic', 'uctp/rvoip-webtransport', 'uctp/rvoip-websocket'):
        directory = UPSTREAM / 'crates' / name
        paths += list(directory.rglob('*.rs')) + [directory / 'Cargo.toml']
    lock = json.loads((ROOT / 'dependencies/source-lock.json').read_text())
    paths += [ROOT / item['path'] for item in lock['patches']]
    return {str(path.relative_to(ROOT)): digest(path) for path in sorted(set(paths))}


def main():
    directory = ROOT / '.local/gate-6/auth-dispatch' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    before = sources()
    cargo = ['cargo', 'test', '--locked', '--offline', '-j', '1', '--target-dir',
             'target', '--manifest-path', '.deps/rvoip/Cargo.toml']
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('auth-format', ['rustfmt', '--edition', '2021', '--config', 'skip_children=true', '--check',
                        '.deps/rvoip/crates/identity/auth-core/src/sig9421.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/src/v2/mod.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/src/v2/auth_dispatch.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/tests/v2_auth_dispatch.rs'], 0),
        ('canonical-dispatch', cargo + ['-p', 'rvoip-uctp', '--test', 'v2_auth_dispatch'], 20),
        ('auth-regression', cargo + ['-p', 'rvoip-auth-core', '--lib',
            '--test', 'principal', '--test', 'jwt', '--test', 'jwks',
            '--test', 'introspection', '--test', 'credential_diagnostics',
            '--test', 'provider_contracts', '--test', 'dpop', '--test', 'aauth_principal'], 100),
        ('uctp-regression', cargo + ['-p', 'rvoip-uctp', '--lib',
            '--test', 'sig9421_gate', '--test', 'pending_dispatch',
            '--test', 'jwt_integration', '--test', 'credential_diagnostics'], 70),
        ('staged-rtp-regression', cargo + ['-p', 'rvoip-uctp', '--test', 'staged_media', '--test', 'peer_media_router'], 12),
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
        for path in re.findall(r'Running [^\n]+\((target/debug/deps/[^\)]+)\)', raw):
            if (ROOT / path).is_file():
                binaries[path] = digest(ROOT / path)
        step = dict(name=name, command=command, exit_code=result.returncode,
                    tests_passed=tests, private_log=str(log.relative_to(ROOT)),
                    log_sha256=digest(log), test_binaries_sha256=binaries,
                    passed=result.returncode == 0 and len(tests) >= minimum)
        steps.append(step)
        print(json.dumps(dict(step=name, passed=step['passed'], tests=len(tests))), flush=True)
    test_source = (UPSTREAM / 'crates/uctp/rvoip-uctp/tests/v2_auth_dispatch.rs').read_text()
    required = set(re.findall(r'#\[tokio::test(?:\([^\]]*\))?\]\s*async fn ([a-z_]+)\(', test_source))
    observed = {name for step in steps for name in step['tests_passed']}
    missing = sorted(required - observed)
    after = sources()
    report = dict(format=1, task='G6-026', implementation_status='in-progress',
        qualification='explicit canonical authenticated dispatch backend with real Ed25519/JCS signatures and controlled current-authority/credential fixtures; public handshake and refresh/step-up remain incomplete',
        executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        platform=platform.platform(),
        source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        source_inputs_sha256=before, sources_unchanged=before == after,
        steps=steps, required_dispatch_tests=sorted(required), missing_required_tests=missing,
        not_qualified=[
            'No public uctp/2 or connect/2 handshake/profile/transport installation. Existing v1 coordinator regression is not canonical handshake evidence.',
            'Auth refresh, keepalive and pending step-up challenge validation, assurance provenance changes/downgrade handling and expiry event publication remain open. Such messages cannot enter this ordinary dispatch handler.',
            'Current-authority guards and bearer validators in new dispatch tests are controlled isolated fixtures; Connect store-policy installation remains G6-037. JWT/JWKS/introspection regression uses local fixtures, not live provider validation.',
            'One-effect retry fixture is process-local, not durable operation recovery. Production restart replay retention, multi-process replay storage and physical activation integration remain unqualified.',
            'No media quality, public SDK/client, remote, browser or live provider qualification; G6-026 and Gate 6 acceptance remain incomplete.'
        ],
        passed=before == after and len(required) >= 20 and not missing and all(step['passed'] for step in steps))
    report['exploratory_attempts'] = [dict(private_log=str(path.relative_to(ROOT)), log_sha256=digest(path),
        disposition='Earlier local candidate run; first 15 tests passed with an unused import warning, then 20 tests passed after bounded credential-expiry/drop/cancellation guards. Only this complete unchanged-source run qualifies the recorded scope.',
        failures_observed=bool(re.search(r'error\[|error:|FAILED', path.read_text(errors='replace'))))
        for path in sorted((ROOT / '.local/gate-6').glob('auth-dispatch-tests-attempt*.log'))]
    destination = ROOT / 'docs/evidence/gate-6/auth-dispatch.json'
    if destination.exists():
        previous = json.loads(destination.read_text())
        report['previous_attempts'] = previous.get('previous_attempts', []) + [dict(
            executed_at=previous['executed_at'], passed=previous['passed'], report_sha256=digest(destination),
            steps=previous['steps'], source_inputs_sha256=previous['source_inputs_sha256'])]
    destination.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(task=report['task'], passed=report['passed'], sources_unchanged=report['sources_unchanged'], missing_required_tests=missing)), flush=True)
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
