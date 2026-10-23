#!/usr/bin/env python3
"""Qualify RTP feedback payload ownership and reproduce pinned dependency sources."""
import datetime
import hashlib
import json
import os
import pathlib
import platform
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
CASE = 'nack::responder::tests::negotiated_audio_nack_does_not_replay_telephone_event_or_unknown_payloads_on_its_ssrc'
REPAIR_CASES = ['peer_connection::handler::endpoint::rtx_feedback_tests::declared_rtx_first_packet_reaches_primary_track_without_feedback_reset',
                'peer_connection::handler::endpoint::rtx_feedback_tests::declared_rtx_source_clock_registry_preserves_transport_receipts_and_primary_recovery']


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def source_tree(directory):
    lock = json.loads((ROOT / 'dependencies/source-lock.json').read_text())
    result = {}
    for name in lock['sources']:
        for folder, directories, files in os.walk(directory / name):
            directories[:] = [d for d in directories if d != '.git']
            for filename in files:
                p = pathlib.Path(folder) / filename
                if filename == '.connect-prepared.json':
                    continue
                relative = str(p.relative_to(directory))
                result[relative] = ({'link': os.readlink(p)} if p.is_symlink() else
                                    {'sha256': digest(p), 'mode': p.stat().st_mode & 0o777})
    return result


def sources():
    paths = [ROOT / n for n in ('scripts/check-gate6-media-feedback.py', 'scripts/prepare-dependencies.py',
             'scripts/prepare-dependencies.sh', 'scripts/lib/prepared_archive.py', 'scripts/tests/test_prepared_archive.py',
             'scripts/format.sh', 'dependencies/source-lock.json', 'contracts/native-media-profile.md')]
    paths += [p for p in (ROOT / '.deps').rglob('*') if p.is_file() and (p.suffix == '.rs' or p.name in ('Cargo.toml', 'Cargo.lock'))]
    paths += list((ROOT / 'patches').rglob('*.patch'))
    return {str(p.relative_to(ROOT)): digest(p) for p in sorted(set(paths))}


def fresh(directory):
    result = subprocess.run(['bash', 'scripts/prepare-dependencies.sh', '--destination', str(directory / 'fresh'), '--cache', str(ROOT / '.deps')], cwd=ROOT)
    if result.returncode:
        raise SystemExit(result.returncode)
    current, reproduced = source_tree(ROOT / '.deps'), source_tree(directory / 'fresh')
    different = sorted(k for k in set(current) | set(reproduced) if current.get(k) != reproduced.get(k))
    report = dict(source_files=len(current), different=different, current=current, reproduced=reproduced, passed=not different)
    (directory / 'reproduction.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(source_files=len(current), different=different, passed=not different)), flush=True)
    if different:
        raise SystemExit(1)


def main():
    directory = ROOT / '.local/gate-6/media-feedback' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    before = sources()
    cargo = ['cargo', 'test', '--locked', '--offline', '-j1', '--target-dir', 'target', '--manifest-path', '.deps/rvoip/Cargo.toml']
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('interceptor-format', ['rustfmt', '--edition', '2024', '--config', 'skip_children=true', '--check', '.deps/rtc-interceptor/src/nack/responder.rs'], 0),
        ('archive-reproduction-tests', ['python3', '-m', 'unittest', 'discover', '-s', 'scripts/tests', '-p', 'test_prepared_archive.py', '-v'], 8),
        ('fresh-source-reproduction', ['python3', 'scripts/check-gate6-media-feedback.py', '--fresh', str(directory)], 0),
        ('interceptor-library', cargo + ['-p', 'rtc-interceptor', '--lib', '--', '--test-threads=1'], 135),
        ('rtc-repair', cargo + ['-p', 'rvoip-rtc', '--lib', 'rtx_feedback_tests', '--', '--test-threads=1'], 2),
        ('webrtc-wire', cargo + ['-p', 'rvoip-webrtc', '--test', 'dtmf_wire', '--', '--test-threads=1'], 6),
        ('progress', ['python3', 'scripts/check-gate6-progress.py'], 0),
        ('diff', ['git', 'diff', '--check'], 0),
    ]
    steps = []
    for name, command, minimum in commands:
        log = directory / (name + '.log')
        with log.open('w') as output:
            r = subprocess.run(command, cwd=ROOT, stdout=output, stderr=subprocess.STDOUT)
        raw = log.read_text(errors='replace')
        tests = re.findall(r'^test (.+?) \.\.\. ok$', raw, re.M)
        if name == 'archive-reproduction-tests':
            tests = re.findall(r'^(test_\S+) \([^\n]+\) \.\.\. ok$', raw, re.M)
        binaries = {p: digest(ROOT / p) for p in re.findall(r'Running [^\n]+\((target/debug/deps/[^\)]+)\)', raw)}
        step = dict(name=name, command=command, exit_code=r.returncode, tests_passed=tests,
                    log=str(log.relative_to(ROOT)), log_sha256=digest(log), test_binaries_sha256=binaries,
                    passed=r.returncode == 0 and len(tests) >= minimum)
        steps.append(step)
        print(json.dumps(dict(step=name, passed=step['passed'], tests=len(tests))), flush=True)
        if not step['passed']:
            break
    seen = {t for s in steps for t in s['tests_passed']}
    missing = sorted(set([CASE] + REPAIR_CASES) - seen)
    reproduction_path = directory / 'reproduction.json'
    reproduced = json.loads(reproduction_path.read_text()) if reproduction_path.exists() else {'passed': False}
    report = dict(task='G6-024', related_task='G6-015', implementation_status='in-progress',
                  qualification='Registered primary payload owns its NACK/RTX cache; original auxiliary packets still forward unchanged. Explicit Opus/NACK registration exercises telephone-event payloads and unregistered payloads, with/without RTX. Default WebRTC audio still advertises TWCC only; no live browser optional-audio-NACK negotiation is claimed. Full interceptor and declared RTC repair regressions, actual default WebRTC/SRTP DTMF, parent-Git archive patch application and clean pinned source reproduction.',
                  not_qualified=['General Core/native/SIP/WebRTC current-authorized DTMF dispatch, exact source/method/logical-operation correlation and WebRTC actual driver IO custody.',
                                 'Remaining media profiles, Linux x86_64, remote/native/browser/provider lanes, bounded G6-093 rehearsals, full Gate 6 acceptance and publication.'],
                  platform=platform.platform(), rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(),
                  source_inputs_sha256=before, sources_unchanged=before == sources(), steps=steps,
                  runtime_checks=sum(len(s['tests_passed']) for s in steps), required_cases=[CASE] + REPAIR_CASES,
                  missing_required_cases=missing, reproduced_source_files=reproduced.get('source_files'),
                  reproduction_report=str(reproduction_path.relative_to(ROOT)),
                  reproduction_sha256=digest(reproduction_path) if reproduction_path.exists() else None,
                  passed=len(steps) == len(commands) and before == sources() and reproduced['passed'] and not missing and all(s['passed'] for s in steps))
    path = ROOT / 'docs/evidence/gate-6/media-feedback.json'
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(passed=report['passed'], checks=report['runtime_checks'], reproduced=report['reproduced_source_files'])), flush=True)
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == '--fresh':
        fresh(pathlib.Path(sys.argv[2]))
    else:
        main()
