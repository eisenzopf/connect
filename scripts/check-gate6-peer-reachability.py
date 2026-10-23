#!/usr/bin/env python3
"""Qualify the G6-026 authenticated peer reachability and guarded local submission on local fixtures."""
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
        'dependencies/source-lock.json', 'scripts/check-gate6-peer-reachability.py',
        'scripts/prepare-dependencies.py', 'scripts/lib/prepared_archive.py',
        'docs/developer/authenticated-dispatch.md', 'docs/developer/authenticated-peer.md', 'docs/developer/step-up.md', 'docs/developer/auth-recovery.md', 'docs/developer/peer-reachability.md', 'contracts/connect-uctp.md',
        'scripts/generate-gate6-schemas.py', 'scripts/generate-gate6-rust.py',
        'scripts/check-gate6-contracts.py', 'contracts/uctp-vectors/gate6-envelope.json',
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
    directory = ROOT / '.local/gate-6/peer-reachability' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
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
                        '.deps/rvoip/crates/uctp/rvoip-uctp/src/v2/replay.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/src/v2/reachability.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/tests/v2_reachability.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/tests/v2_auth_recovery.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/src/v2/auth_dispatch.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/src/v2/auth_peer.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/src/v2/auth_peer/step_up.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/tests/v2_step_up.rs',
                        '.deps/rvoip/crates/identity/auth-core/src/bearer.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/src/v2/payloads.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/tests/v2_auth_peer.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/tests/v2_auth_dispatch.rs'], 0),
        ('peer-reachability', cargo + ['-p', 'rvoip-uctp', '--test', 'v2_reachability'], 13),
        ('auth-recovery', cargo + ['-p', 'rvoip-uctp', '--test', 'v2_auth_recovery'], 11),
        ('step-up', cargo + ['-p', 'rvoip-uctp', '--test', 'v2_step_up'], 15),
        ('canonical-peer', cargo + ['-p', 'rvoip-uctp', '--test', 'v2_auth_peer'], 17),
        ('contracts', [str(ROOT / '.local/gate-6/verification-venv/bin/python'), 'scripts/check-gate6-contracts.py', '--output', str(directory / 'contracts.json')], 0),
        ('auth-regression', cargo + ['-p', 'rvoip-auth-core', '--lib',
            '--test', 'principal', '--test', 'jwt', '--test', 'jwks',
            '--test', 'introspection', '--test', 'credential_diagnostics',
            '--test', 'provider_contracts', '--test', 'dpop', '--test', 'aauth_principal'], 133),
        ('uctp-regression', cargo + ['-p', 'rvoip-uctp', '--lib',
            '--test', 'v2_auth_dispatch', '--test', 'sig9421_gate', '--test', 'pending_dispatch',
            '--test', 'jwt_integration', '--test', 'credential_diagnostics'], 109),
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
    test_source = (UPSTREAM / 'crates/uctp/rvoip-uctp/tests/v2_reachability.rs').read_text()
    required = set(re.findall(r'#\[tokio::test(?:\([^\]]*\))?\]\s*async fn ([A-Za-z_][A-Za-z_0-9]*)\(', test_source))
    observed = {name for step in steps for name in step['tests_passed']}
    missing = sorted(required - observed)
    after = sources()
    report = dict(format=1, task='G6-026', implementation_status='in-progress',
        qualification='authenticated live-peer reachability/heartbeat with staged registration, current revocation lifetime and guarded actor-backed local writer submission; real local uctp/2 TLS/QUIC, controlled authority fixtures; complete G6-026 remains open',
        executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        platform=platform.platform(),
        source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        source_inputs_sha256=before, sources_unchanged=before == after,
        steps=steps, required_reachability_tests=sorted(required), missing_required_tests=missing,
        not_qualified=[
            'Real local uctp/2 TLS/QUIC proves registry-routed signed frames and current delivery denial/revocation. Authority and device/identity policies are controlled fixtures, not production Connect store integration.',
            'LocalSubmission means writer admission only, not remote delivery/acceptance/readiness/recipient receipt. Interrupted partial writes are explicit unknown outcomes; durable application reconciliation remains separate.',
            'Connect EndpointRuntime observation bridge, canonical Session invitation/acceptance, reconnect/logical-Session replay and common production QUIC/WT/WS profile installation remain open.',
            'Default scope/resource/capability and full G6-026 normative audit remain open. No public SDK/browser, remote/provider or media-quality qualification inferred.',
            'G6-026 and all Gate 6 acceptance remain incomplete.'
        ],
        passed=before == after and len(required) >= 13 and not missing and all(step['passed'] for step in steps))
    report['exploratory_attempts'] = [dict(private_log=str(path.relative_to(ROOT)), log_sha256=digest(path),
        disposition='Earlier reachability candidate run; the first new network revocation fixture consumed a stale prior Notify permit, corrected with an explicit barrier reset before its pending delivery. Failed and successful candidates remain recorded; compiler failures and runtime outcomes remain preserved. Only the final complete unchanged-source run qualifies the recorded scope.',
        failures_observed=bool(re.search(r'error\[|error:|FAILED', path.read_text(errors='replace'))))
        for path in sorted((ROOT / '.local/gate-6').glob('reachability-tests-attempt*.log'))]
    destination = ROOT / 'docs/evidence/gate-6/peer-reachability.json'
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
