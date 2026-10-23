#!/usr/bin/env python3
"""Verify bounded per-packet DTMF custody and affected RTP/SIP/WebRTC behavior."""
import datetime
import hashlib
import json
import pathlib
import platform
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
UPSTREAM = ROOT / '.deps/rvoip'
CHANGED = ['crates/media/media-core/src/dtmf_authority.rs', 'crates/media/media-core/src/lib.rs',
           'crates/media/media-core/src/error.rs', 'crates/media/media-core/src/relay/controller/dtmf_transmitter.rs',
           'crates/foundation/rvoip-core/src/dtmf.rs']
REQUIRED = ['relay::controller::dtmf_transmitter::authority_tests::completed_first_digit_then_gap_revocation_is_an_uncertain_sequence',
            'dtmf_authority::tests::denied_expired_and_stalled_acquisition_never_poll_io',
            'dtmf_authority::tests::dropping_request_aborts_pending_io_before_releasing_owned_guard',
            'dtmf_authority::tests::retirement_and_deadline_abort_owned_io_before_releasing_guard',
            'dtmf_authority::tests::transport_failure_is_unknown_and_never_an_automatic_retry',
            'relay::controller::dtmf_transmitter::authority_tests::real_udp_permission_is_fresh_per_packet_and_revocation_does_not_wait_for_tone_or_gap',
            'actual_rfc4733_adapter_preserves_observed_duration_and_live_connection',
            'dropping_owned_sequence_after_actual_rtp_stops_current_tone_and_suffix']


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def sources():
    paths = [ROOT / 'scripts/check-gate6-dtmf-write-authority.py', ROOT / 'scripts/prepare-dependencies.sh',
             ROOT / 'scripts/prepare-dependencies.py', ROOT / 'scripts/format.sh',
             ROOT / 'dependencies/source-lock.json', ROOT / 'contracts/native-signals-profile.md']
    paths += [p for p in (ROOT / '.deps').rglob('*')
              if p.is_file() and (p.suffix == '.rs' or p.name in ('Cargo.toml', 'Cargo.lock'))]
    paths += list((ROOT / 'patches/rvoip').glob('*.patch'))
    return {str(path.relative_to(ROOT)): digest(path) for path in sorted(set(paths))}


def main():
    directory = ROOT / '.local/gate-6/dtmf-write-authority' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    before = sources()
    cargo = ['cargo', 'test', '--locked', '--offline', '-j1', '--target-dir', 'target', '--manifest-path', '.deps/rvoip/Cargo.toml']
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('upstream-format', ['rustfmt', '--edition', '2021', '--config', 'skip_children=true', '--check'] + [str((UPSTREAM / p).relative_to(ROOT)) for p in CHANGED], 0),
        ('shared-and-udp', cargo + ['-p', 'rvoip-media-core', '--lib', 'dtmf', '--', '--test-threads=1'], 24),
        ('sip-network', cargo + ['-p', 'rvoip-sip', '--features', 'rvoip-core/opus', '--test', 'adapter_dtmf_network', '--', '--test-threads=1'], 1),
        ('webrtc-wire', cargo + ['-p', 'rvoip-webrtc', '--test', 'dtmf_wire', '--', '--test-threads=1'], 6),
        ('progress', ['python3', 'scripts/check-gate6-progress.py'], 0),
        ('diff', ['git', 'diff', '--check'], 0),
    ]
    steps = []
    for name, command, minimum in commands:
        log = directory / (name + '.log')
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, stdout=output, stderr=subprocess.STDOUT)
        raw = log.read_text(errors='replace')
        tests = re.findall(r'^test (.+?) \.\.\. ok$', raw, re.M)
        binaries = {path: digest(ROOT / path) for path in re.findall(r'Running [^\n]+\((target/debug/deps/[^\)]+)\)', raw)}
        step = dict(name=name, command=command, exit_code=result.returncode, tests_passed=tests,
                    log=str(log.relative_to(ROOT)), log_sha256=digest(log), test_binaries_sha256=binaries,
                    passed=result.returncode == 0 and len(tests) >= minimum)
        steps.append(step)
        print(json.dumps(dict(step=name, passed=step['passed'], tests=len(tests))), flush=True)
        if not step['passed']:
            break
    observed = {t for s in steps for t in s['tests_passed']}
    missing = sorted(set(REQUIRED) - observed)
    report = dict(task='G6-024', implementation_status='in-progress',
                  qualification='Shared fresh-per-packet DTMF permission/custody primitive and real UDP/RTP sequence integration. Default denial, invalid whole intent, exact observed packet/scope matching, actual read-guard release between writes, queued revocation, later authorized restoration, owned IO cancellation/deadline and typed unknown outcome. A submitted prefix or completed first digit preserves whole-sequence uncertainty after later refusal. Actual SIP/UDP and WebRTC/SRTP wire regressions share the supported Core Opus backend profile; the shared media tests separately use media-core defaults. All run on the same source.',
                  not_qualified=['General Core/native/gateway DTMF source, method, logical-operation and current-policy installation remains unfinished; raw dispatch is not qualified by this primitive.',
                                 'WebRTC actual driver IO custody is unfinished; its enqueue boundary cannot stand in for an authorized physical write.',
                                 'Production Store/HTTP/SDK/journal dispatch, remaining media profiles, Linux x86_64, remote/browser/provider and full Gate 6 acceptance.'],
                  executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
                  rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(),
                  source_inputs_sha256=before, sources_unchanged=before == sources(), steps=steps,
                  runtime_tests=sum(len(s['tests_passed']) for s in steps), required_cases=REQUIRED,
                  missing_required_cases=missing,
                  passed=len(steps) == len(commands) and before == sources() and not missing and all(s['passed'] for s in steps))
    path = ROOT / 'docs/evidence/gate-6/dtmf-write-authority.json'
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(passed=report['passed'], tests=report['runtime_tests'], report=str(path.relative_to(ROOT)))), flush=True)
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
