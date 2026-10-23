#!/usr/bin/env python3
"""Qualify required credential retirement, signed terminal assurance changes and exact candidate/current/successor isolation."""
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
        'dependencies/source-lock.json', 'scripts/check-gate6-assurance-retirement.py',
        'scripts/prepare-dependencies.py', 'scripts/lib/prepared_archive.py',
        'docs/developer/authenticated-dispatch.md', 'docs/developer/authenticated-peer.md', 'docs/developer/step-up.md', 'docs/developer/auth-recovery.md', 'docs/developer/peer-reachability.md', 'docs/developer/canonical-transports.md', 'docs/developer/secure-profile.md', 'docs/developer/peer-admission.md', 'docs/developer/peer-sessions.md', 'docs/developer/peer-native.md', 'docs/developer/peer-exchange.md', 'docs/developer/session-cleanup.md', 'docs/developer/canonical-media.md', 'docs/developer/media-control.md', 'docs/developer/assurance-retirement.md', 'docs/developer/authentication-audit.md', 'contracts/uctp-gate-6-requirements.json', 'docs/developer/stream-lifecycle.md', 'docs/developer/staged-streams.md', 'docs/developer/staged-rtp.md', 'contracts/connect-uctp.md',
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
    directory = ROOT / '.local/gate-6/media-control' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
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
                        '.deps/rvoip/crates/uctp/rvoip-uctp/src/v2/scope_policy.rs',
                        '.deps/rvoip/crates/foundation/rvoip-core/src/peer_admission.rs',
                        '.deps/rvoip/crates/foundation/rvoip-core/src/orchestrator/peer_sessions.rs',
                        '.deps/rvoip/crates/foundation/rvoip-core/src/orchestrator/peer_native.rs',
                        '.deps/rvoip/crates/foundation/rvoip-core/src/adapter.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/src/v2/native.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/src/v2/native_media.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/tests/support/canonical_media.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/tests/support/media_control.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/src/v2/auth_peer/exchange.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/tests/support/peer_exchange.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/tests/support/peer_native.rs',
                        '.deps/rvoip/crates/foundation/rvoip-core/tests/peer_sessions.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/tests/support/peer_sessions.rs',
                        '.deps/rvoip/crates/foundation/rvoip-core/src/lib.rs',
                        '.deps/rvoip/crates/foundation/rvoip-core/src/orchestrator/participant_control/end_session_tests.rs',
                        '.deps/rvoip/crates/foundation/rvoip-core/src/orchestrator.rs',
                        '.deps/rvoip/crates/foundation/rvoip-core/src/orchestrator/participant_control.rs',
                        '.deps/rvoip/crates/foundation/rvoip-core/tests/peer_admission.rs',
                        '.deps/rvoip/crates/foundation/rvoip-core/tests/support/participant_control_fixture.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/tests/v2_peer_admission.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/tests/v2_secure_profile.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/tests/staged_media.rs',
                        '.deps/rvoip/crates/foundation/rvoip-core/src/stream_lifecycle.rs',
                        '.deps/rvoip/crates/foundation/rvoip-core/src/stream_lifecycle/tests.rs',
                        '.deps/rvoip/crates/foundation/rvoip-core/src/staged_streams.rs',
                        '.deps/rvoip/crates/foundation/rvoip-core/src/staged_streams/tests.rs',
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
                        '.deps/rvoip/crates/uctp/rvoip-uctp/src/v2/auth_peer/retirement.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/tests/support/assurance_retirement.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/tests/support/retirement_policy.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/tests/v2_step_up.rs',
                        '.deps/rvoip/crates/identity/auth-core/src/bearer.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/src/v2/payloads.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/tests/v2_auth_peer.rs',
                        '.deps/rvoip/crates/uctp/rvoip-uctp/tests/v2_auth_dispatch.rs'], 0),
        ('peer-core-sessions',cargo+['-p','rvoip-core','--features','opus','--test','peer_sessions'],10),
        ('peer-core-admission',cargo+['-p','rvoip-core','--features','opus','--test','peer_admission'],9),
        ('peer-wire-admission',cargo+['-p','rvoip-uctp','--test','v2_peer_admission'],43),
        ('secure-profile', cargo + ['-p','rvoip-uctp','--test','v2_secure_profile'],10),
        ('core-library',cargo+['-p','rvoip-core','--features','opus','--lib'],237),
        ('participant-and-media',cargo+['-p','rvoip-core','--features','opus']+[arg for suite in ['participant_control','session_lifecycle','participant_roles','messaging','p5_pause_and_listener','stream_readiness','tenant_and_capacity','orchestrator_dispatch','provisional_early_media'] for arg in ('--test',suite)],149),
        ('core-ownership-regression',cargo+['-p','rvoip-core','--features','opus','--test','per_connection_control','--test','v2b_semaphore_admission','--test','conversation_lifecycle'],6),
        ('delivery-fences',cargo+['-p','rvoip-core-traits','--lib','peer_switch::tests'],12),
        ('canonical-transports', cargo + ['-p', 'rvoip-uctp', '--test', 'v2_canonical_transports'], 38),
        ('quic-driver-regression', cargo + ['-p', 'rvoip-quic', '--lib', '--test', 'adapter', '--test', 'loopback', '--test', 'multi_session_peer'], 1),
        ('wt-driver-regression', cargo + ['-p', 'rvoip-webtransport', '--lib', '--test', 'adapter', '--test', 'loopback', '--test', 'peer_media_routing'], 1),
        ('wss-driver-regression', cargo + ['-p', 'rvoip-websocket', '--features', 'wss', '--lib', '--test', 'adapter', '--test', 'loopback', '--test', 'wss_loopback', '--test', 'lifecycle_capacity', '--test', 'conversation_create'], 1),
        ('peer-reachability', cargo + ['-p', 'rvoip-uctp', '--test', 'v2_reachability'], 13),
        ('auth-recovery', cargo + ['-p', 'rvoip-uctp', '--test', 'v2_auth_recovery'], 11),
        ('step-up', cargo + ['-p', 'rvoip-uctp', '--test', 'v2_step_up'], 19),
        ('canonical-peer', cargo + ['-p', 'rvoip-uctp', '--test', 'v2_auth_peer'], 25),
        ('contracts', [str(ROOT / '.local/gate-6/verification-venv/bin/python'), 'scripts/check-gate6-contracts.py', '--output', str(directory / 'contracts.json')], 0),
        ('auth-regression', cargo + ['-p', 'rvoip-auth-core', '--lib',
            '--test', 'principal', '--test', 'jwt', '--test', 'jwks',
            '--test', 'introspection', '--test', 'credential_diagnostics',
            '--test', 'provider_contracts', '--test', 'dpop', '--test', 'aauth_principal'], 133),
        ('uctp-regression', cargo + ['-p', 'rvoip-uctp', '--lib',
            '--test', 'v2_auth_dispatch', '--test', 'sig9421_gate', '--test', 'pending_dispatch',
            '--test', 'jwt_integration', '--test', 'credential_diagnostics'], 109),
        ('staged-rtp-regression', cargo + ['-p', 'rvoip-uctp', '--test', 'staged_media', '--test', 'peer_media_router'], 13),
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
    required = set()
    for relative,prefix in [
        ('foundation/rvoip-core/src/orchestrator/participant_control/end_session_tests.rs','orchestrator::participant_control::end_session_tests::'),
        ('foundation/rvoip-core/tests/peer_admission.rs',''),
        ('foundation/rvoip-core/tests/peer_sessions.rs',''),
        ('foundation/rvoip-core/src/orchestrator/peer_sessions.rs','orchestrator::peer_sessions::tests::'),
        ('uctp/rvoip-uctp/tests/support/peer_sessions.rs','peer_sessions::'),
        ('uctp/rvoip-uctp/tests/support/peer_native.rs','peer_native::'),
        ('uctp/rvoip-uctp/tests/support/canonical_media.rs','peer_native::canonical_media::'),
        ('uctp/rvoip-uctp/tests/support/media_control.rs','peer_native::canonical_media::media_control::'),
        ('uctp/rvoip-uctp/tests/support/peer_exchange.rs','peer_exchange::'),
        ('foundation/rvoip-core/tests/per_connection_control.rs',''),
        ('foundation/rvoip-core/tests/v2b_semaphore_admission.rs',''),
        ('foundation/rvoip-core/tests/conversation_lifecycle.rs',''),
        ('uctp/rvoip-uctp/tests/v2_peer_admission.rs',''),
        ('uctp/rvoip-uctp/tests/v2_secure_profile.rs',''),
        ('uctp/rvoip-uctp/tests/v2_canonical_transports.rs',''),
        ('uctp/rvoip-uctp/tests/v2_step_up.rs',''),
        ('uctp/rvoip-uctp/tests/v2_auth_peer.rs',''),
        ('uctp/rvoip-uctp/tests/v2_reachability.rs',''),
        ('uctp/rvoip-uctp/tests/support/assurance_retirement.rs','assurance_retirement::'),
        ('uctp/rvoip-uctp/tests/staged_media.rs',''),
        ('foundation/rvoip-core/src/stream_lifecycle/tests.rs','stream_lifecycle::tests::'),
        ('foundation/rvoip-core/src/staged_streams/tests.rs','staged_streams::tests::')]:
        test_source = (UPSTREAM / 'crates' / relative).read_text()
        required.update(prefix+name for name in re.findall(r'#\[(?:tokio::)?test(?:\([^\]]*\))?\]\s*(?:async )?fn ([A-Za-z_][A-Za-z_0-9]*)\(',test_source))
    observed = {name for step in steps for name in step['tests_passed']}
    missing = sorted(required - observed)
    after = sources()
    report = dict(format=1, task='G6-026', implementation_status='in-progress',
        qualification='Required current credential-incarnation retirement signals, exact old/current/candidate/successor isolation and signed terminal assurance-change/goodbye on expiry and factual revocation; local actual TLS QUIC plus QUIC/WT/WSS regressions with controlled authority policies; final G6-026 audit remains open',
        executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        platform=platform.platform(),
        source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        source_inputs_sha256=before, sources_unchanged=before == after,
        steps=steps, required_assurance_retirement_tests=sorted(required), missing_required_tests=missing,
        not_qualified=[
            'The actual canonical driver/physical media/Core route integration uses a controlled current media policy and readiness negotiator. Reserved ACKs do not establish connection.answer/ready, Participant presence or completed operations. Full canonical negotiation, endpoint confirmation, managed mixer and retained subscription handlers remain open.',
            'Controllers retain their original credential bound and fail closed at expiry. Resource/handle history persists through refresh, but production credential rotation, Connect durable policy installation and ParticipantPolicy held-authority reuse must still be installed. The media engine now has an explicit held-control hook; controlled fixture composition does not qualify the production store policy. No stale snapshot can grant media or replay authority.',
            'Local cleanup confirmation is not provider delivery/readiness/hangup. Uncertain cleanup retains route liveness/controller ownership and physical quota. Shared logical Sessions and unrelated physical peers must survive EOF.',
            'IdentityProvider/Connect EndpointRuntime bridging, invitation delivery/acceptance, durable logical Session/operation replay across peers/reconnect/restart, and final signing/replay-lifetime scope audit remain open; terminal assurance retirement is covered here with controlled current policy signals.',
            'Connect durable/public dispatch, full media/codec/browser/SDK/remote/provider profiles and every Gate 6 acceptance criterion remain unqualified by this report.',
        ],
        passed=before == after and len(required) >= 246 and not missing and all(step['passed'] for step in steps))
    report['exploratory_attempts'] = [dict(private_log=str(path.relative_to(ROOT)), log_sha256=digest(path),
        disposition='Retained development attempts: initial helper visibility correction; TLS fixture retained host connection before reading terminal frames; candidate step-up watch select syntax correction (rustc also reported an incremental ICE); existing pending-exchange test updated to require the new ordered assurance-change before goodbye. Final qualification is independent.',
        failures_observed=bool(re.search(r'error\[|error:|FAILED|compiler unexpectedly panicked', path.read_text(errors='replace'))))
        for path in sorted((ROOT / '.local/gate-6').glob('assurance-retirement-*-attempt*.log')) if 'qualification' not in path.name]
    destination = ROOT / 'docs/evidence/gate-6/assurance-retirement.json'
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
