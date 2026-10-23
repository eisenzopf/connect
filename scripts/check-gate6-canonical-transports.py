#!/usr/bin/env python3
"""Qualify G6-026 canonical QUIC, WebTransport and WSS drivers on local TLS fixtures."""
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
        'dependencies/source-lock.json', 'scripts/check-gate6-canonical-transports.py',
        'scripts/prepare-dependencies.py', 'scripts/lib/prepared_archive.py',
        'docs/developer/authenticated-dispatch.md', 'docs/developer/authenticated-peer.md', 'docs/developer/step-up.md', 'docs/developer/auth-recovery.md', 'docs/developer/peer-reachability.md', 'docs/developer/canonical-transports.md', 'contracts/connect-uctp.md',
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
    directory = ROOT / '.local/gate-6/canonical-transports' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
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
                        '.deps/rvoip/crates/uctp/rvoip-uctp/src/v2/server.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/tests/v2_canonical_transports.rs',
                        '.deps/rvoip/crates/uctp/rvoip-quic/src/canonical.rs',
                        '.deps/rvoip/crates/uctp/rvoip-webtransport/src/canonical.rs',
                        '.deps/rvoip/crates/uctp/rvoip-webtransport/src/framing.rs',
                        '.deps/rvoip/crates/uctp/rvoip-websocket/src/canonical.rs',
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
        ('canonical-transports', cargo + ['-p', 'rvoip-uctp', '--test', 'v2_canonical_transports'], 17),
        ('quic-driver-regression', cargo + ['-p', 'rvoip-quic', '--lib', '--test', 'adapter', '--test', 'loopback', '--test', 'multi_session_peer'], 1),
        ('wt-driver-regression', cargo + ['-p', 'rvoip-webtransport', '--lib', '--test', 'adapter', '--test', 'loopback', '--test', 'peer_media_routing'], 1),
        ('wss-driver-regression', cargo + ['-p', 'rvoip-websocket', '--features', 'wss', '--lib', '--test', 'adapter', '--test', 'loopback', '--test', 'wss_loopback', '--test', 'lifecycle_capacity', '--test', 'conversation_create'], 1),
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
    test_source = (UPSTREAM / 'crates/uctp/rvoip-uctp/tests/v2_canonical_transports.rs').read_text()
    required = set(re.findall(r'#\[(?:tokio::)?test(?:\([^\]]*\))?\]\s*(?:async )?fn ([A-Za-z_][A-Za-z_0-9]*)\(', test_source))
    observed = {name for step in steps for name in step['tests_passed']}
    missing = sorted(required - observed)
    after = sources()
    report = dict(format=1, task='G6-026', implementation_status='in-progress',
        qualification='Canonical profile factories driving one authenticated peer actor across real local pinned TLS uctp/2 QUIC, HTTP/3 WebTransport bidirectional framing and WSS text frames, bounded admission, retained drain/shutdown and guarded reachability submission; controlled authority fixtures; complete G6-026 remains open',
        executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        platform=platform.platform(),
        source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        source_inputs_sha256=before, sources_unchanged=before == after,
        steps=steps, required_canonical_transport_tests=sorted(required), missing_required_tests=missing,
        not_qualified=[
            'Concrete canonical backend factories are exercised through actual local TLS connections. Public Connect listener/profile selection, durable store policy and service dispatch installation remain G6-037 work.',
            'Controlled authority/device fixtures do not qualify Connect EndpointRuntime/IdentityProvider reachability bridging, incoming Session invitation/acceptance, logical-Session reconnect or cross-peer/restart replay.',
            'Caller owns the listening endpoint, its pre-admission handshake limits and channel-send failure cleanup. Driver admission/drain/shutdown counts are transport observations, never application delivery or authorization evidence.',
            'No browser, remote/provider, media quality, public SDK, remaining scope/resource/capability audit or Gate 6 acceptance is inferred. G6-026 stays in progress.',
        ],
        passed=before == after and len(required) >= 17 and not missing and all(step['passed'] for step in steps))
    report['exploratory_attempts'] = [dict(private_log=str(path.relative_to(ROOT)), log_sha256=digest(path),
        disposition='Exploratory canonical transport candidate run. Compiler, lockfile and runtime failures are preserved; only the complete unchanged-source qualification establishes the recorded scope.',
        failures_observed=bool(re.search(r'error\[|error:|FAILED', path.read_text(errors='replace'))))
        for path in sorted((ROOT / '.local/gate-6').glob('canonical-transports-tests-attempt*.log'))]
    destination = ROOT / 'docs/evidence/gate-6/canonical-transports.json'
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
