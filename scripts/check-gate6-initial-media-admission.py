#!/usr/bin/env python3
"""Qualify phased first media preparation and held readiness rechecks locally."""
import datetime
import importlib.util
import json
import pathlib
import platform
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('media_control', ROOT / 'scripts/check-gate6-media-control.py')
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)


def sources():
    result = base.sources()
    for name in ('scripts/check-gate6-initial-media-admission.py', 'docs/developer/initial-media-admission.md'):
        result[name] = base.digest(ROOT / name)
    return result


def main():
    directory = ROOT / '.local/gate-6/initial-media-admission' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    before = sources()
    cargo = ['cargo', 'test', '--locked', '--offline', '-j1', '--target-dir', 'target', '--manifest-path', '.deps/rvoip/Cargo.toml']
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('upstream-format', ['rustfmt', '--edition', '2021', '--config', 'skip_children=true', '--check'] + ['.deps/rvoip/' + name for name in (
            'crates/foundation/rvoip-core/src/stream_lifecycle.rs',
            'crates/foundation/rvoip-core/src/staged_streams.rs',
            'crates/foundation/rvoip-core/src/stream_lifecycle/tests.rs',
            'crates/foundation/rvoip-core/src/staged_streams/tests.rs',
            'crates/uctp/rvoip-uctp/tests/support/media_control.rs')], 0),
        ('peer-wire-admission', cargo + ['-p', 'rvoip-uctp', '--test', 'v2_peer_admission'], 45),
        ('canonical-transports', cargo + ['-p', 'rvoip-uctp', '--test', 'v2_canonical_transports'], 39),
        ('core-library', cargo + ['-p', 'rvoip-core', '--features', 'opus', '--lib'], 243),
        ('core-lifecycle', cargo + ['-p', 'rvoip-core', '--features', 'opus'] + [arg for suite in ('participant_control', 'session_lifecycle', 'participant_roles', 'stream_readiness', 'per_connection_control', 'conversation_lifecycle', 'peer_admission', 'peer_sessions') for arg in ('--test', suite)], 64),
        ('staged-rtp', cargo + ['-p', 'rvoip-uctp', '--test', 'staged_media', '--test', 'peer_media_router'], 13),
        ('delivery-fences', cargo + ['-p', 'rvoip-core-traits', '--lib', 'peer_switch::tests'], 12),
        ('contracts', [str(ROOT / '.local/gate-6/verification-venv/bin/python'), 'scripts/check-gate6-contracts.py', '--output', str(directory / 'contracts.json')], 0),
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
            binaries[path] = base.digest(ROOT / path)
        step = dict(name=name, command=command, exit_code=result.returncode, tests_passed=tests,
                    private_log=str(log.relative_to(ROOT)), log_sha256=base.digest(log),
                    test_binaries_sha256=binaries, passed=result.returncode == 0 and len(tests) >= minimum)
        steps.append(step)
        print(json.dumps(dict(step=name, passed=step['passed'], tests=len(tests))), flush=True)
        if not step['passed']:
            break
    required = set()
    for relative, prefix in (
        ('foundation/rvoip-core/src/stream_lifecycle/tests.rs', 'stream_lifecycle::tests::'),
        ('foundation/rvoip-core/src/staged_streams/tests.rs', 'staged_streams::tests::'),
        ('uctp/rvoip-uctp/tests/support/media_control.rs', 'peer_native::canonical_media::media_control::')):
        raw = (ROOT / '.deps/rvoip/crates' / relative).read_text()
        required.update(prefix + name for name in re.findall(r'#\[(?:tokio::)?test(?:\([^\]]*\))?\]\s*(?:async )?fn ([A-Za-z_][A-Za-z_0-9]*)\(', raw))
    observed = {name for step in steps for name in step['tests_passed']}
    missing = sorted(required - observed)
    after = sources()
    report = dict(format=1, task='G6-020', implementation_status='in-progress',
        qualification='Initial Preparing/Ready media staging; Active-only commit and packets; same-grant readiness rechecks with cancellation/deadline fences; local TLS QUIC/WT and RTP using controlled policy/readiness',
        executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
        source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        source_inputs_sha256=before, sources_unchanged=before == after, steps=steps,
        required_tests=sorted(required), missing_required_tests=missing,
        not_qualified=[
            'Canonical offer/answer/ready, actual endpoint confirmation and Participant presence; ACK reports engine revision only.',
            'Production Connect current media policy, retained credential rotation and managed mixer adoption.',
            'Reliable data, WSS/WebRTC/SIP, heterogeneous codec/video/browser quality, logical recovery and full public integration.',
            'Linux, remote/native/browser, live-provider, publication and all Gate 6 acceptance criteria.'],
        passed=before == after and len(steps) == len(commands) and len(required) >= 40 and not missing and all(step['passed'] for step in steps))
    report['exploratory_attempts'] = [dict(private_log=str(path.relative_to(ROOT)), log_sha256=base.digest(path),
        disposition='Exploratory candidate; not part of final source-bound qualification',
        failures_observed=bool(re.search(r'error\[|error:|FAILED', path.read_text(errors='replace'))))
        for path in sorted((ROOT / '.local/gate-6').glob('phased-media-*-tests-*.log'))]
    destination = ROOT / 'docs/evidence/gate-6/initial-media-admission.json'
    if destination.exists():
        previous = json.loads(destination.read_text())
        report['previous_attempts'] = previous.get('previous_attempts', []) + [dict(report_sha256=base.digest(destination), disposition='Unqualified earlier candidate; initial runner selected the general transport target instead of the native peer target and used incorrect suite-count floors. Full named-case checks prevented qualification.', **{k: previous[k] for k in ('executed_at', 'passed', 'steps', 'source_inputs_sha256')})]
    destination.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(passed=report['passed'], missing_required_tests=missing)), flush=True)
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
