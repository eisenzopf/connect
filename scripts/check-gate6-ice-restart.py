#!/usr/bin/env python3
"""Verify the local G6-054 original-peer ICE slice, without remote calls.

Run with the verification Python (jsonschema/mypy), an already selected Cargo
target directory and isolated TMPDIR. This does not qualify the full matrix.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def inputs():
    names = subprocess.check_output(
        ['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'], cwd=ROOT
    ).decode().split('\0')
    paths = {ROOT / name for name in names if name and (
        name.startswith(('crates/', 'sdk/', 'contracts/', 'patches/', 'dependencies/', 'scripts/'))
        or name in ('Cargo.toml', 'Cargo.lock', 'AGENTS.md'))}
    # Include actual prepared upstream code, not merely the patch declarations.
    for extension in ('*.rs', 'Cargo.toml', 'Cargo.lock', '*.json'):
        paths.update((ROOT / '.deps').rglob(extension))
    return {str(p.relative_to(ROOT)): sha(p) for p in sorted(paths) if p.is_file()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    assert output.is_relative_to((ROOT / '.local').resolve()), 'Use ignored .local output'
    output.mkdir(parents=True, exist_ok=False)
    before = inputs()
    (output / 'inputs-before.json').write_text(json.dumps(before, indent=2) + '\n')
    cargo = ['cargo', 'test', '--offline', '--locked', '-j1']
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], None),
        ('format', ['bash', 'scripts/format.sh', '--check'], None),
        ('envelope', [sys.executable, 'scripts/generate-gate6-schemas.py', '--check'], None),
        ('browser-schema', [sys.executable, 'scripts/generate-browser-canonical.py', '--check'], None),
        ('http-reference', [sys.executable, 'scripts/generate-canonical-http-openapi.py', '--check'], None),
        ('transport-only-sdp', cargo + ['--manifest-path', '.deps/rvoip/Cargo.toml', '-p', 'rvoip-rtc', '--lib', 'transport_restart', '--', '--test-threads=1'], 2),
        ('live-ice-generation', cargo + ['--manifest-path', '.deps/rvoip/Cargo.toml', '-p', 'rvoip-rtc', '--lib', 'ice_generation_tests', '--', '--test-threads=1'], 3),
        ('protocol', cargo + ['-p', 'connect-protocol', '--lib', '--', '--test-threads=1'], 64),
        ('protocol-schema', cargo + ['-p', 'connect-protocol', '--features', 'schema', '--lib', '--', '--test-threads=1'], 65),
        ('sdk-receipts', cargo + ['-p', 'connect-client', '--lib', 'ice_receipt_tests', '--', '--test-threads=1'], 2),
        ('sdk-oracle', cargo + ['-p', 'connect-client', '--test', 'gate6_browser_canonical', '--', '--test-threads=1'], 1),
        ('store-original-custody', cargo + ['-p', 'connect-store', '--lib', 'connection_control::tests::native_wire::webrtc_ice', '--', '--test-threads=1'], 13),
        ('signed-rtc', cargo + ['-p', 'connect-server', '--features', 'test-support', '--test', 'gate6_canonical_webtransport', 'signed_rtc_ice_restart', '--', '--test-threads=1'], 2),
        ('typescript', ['sdk/typescript/node_modules/.bin/tsc', '-p', 'sdk/typescript/tsconfig.json'], None),
        ('sdk-components', ['node', '--test'] + ['sdk/tests/' + name + '.test.mjs' for name in (
            'browser_webrtc_restart', 'browser_webrtc_signaling', 'browser_sdk_webrtc', 'browser_voice',
            'browser_sdk_streams', 'node_connections', 'browser_canonical')], None),
        ('python-receipts', [sys.executable, '-m', 'unittest', 'test_python_connections'], None),
        ('contracts', [sys.executable, 'scripts/check-gate6-contracts.py', '--output', str(output / 'contracts.json')], None),
        ('sdk-contract', [sys.executable, 'scripts/check-gate6-sdk-contract.py', '--output', str(output / 'sdk-contract')], None),
        ('sdk-outlines', [sys.executable, 'scripts/check-gate6-sdk-outlines.py', '--output', str(output / 'sdk-outlines')], None),
        ('diff', ['git', 'diff', '--check'], None),
    ]
    steps = []
    env = dict(os.environ, PYTHONPATH='sdk/python:sdk/tests')
    for name, command, expected in commands:
        log = output / (name + '.log')
        started = datetime.datetime.now(datetime.timezone.utc)
        with log.open('w') as stream:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=stream, stderr=subprocess.STDOUT)
        raw = log.read_text(errors='replace')
        counts = re.findall(r'^test result: ok\. (\d+) passed; 0 failed;', raw, re.M)
        passed = result.returncode == 0 and (expected is None or counts == [str(expected)])
        steps.append(dict(name=name, command=command, exit_code=result.returncode, passed=passed,
                          expected_tests=expected, actual_tests=sum(map(int, counts)),
                          seconds=(datetime.datetime.now(datetime.timezone.utc)-started).total_seconds(),
                          log=str(log.relative_to(ROOT)), log_sha256=sha(log)))
        print(json.dumps(steps[-1]), flush=True)
        if not passed:
            break
    after = inputs()
    changed = sorted(name for name in before.keys() | after.keys() if before.get(name) != after.get(name))
    report = dict(task='G6-054', full_task_qualified=False, acceptance_qualified=False,
                  scope='Local signed WSS original-peer PCMU restart, private answer recovery, two generations, current authority withdrawal and original close; typed SDK component/metadata restoration and schema/reference checks.',
                  not_qualified=['Distinct-network, TURN/expiry/relay and fault matrix',
                                 'Actual browser engine restart and audio/video quality matrix',
                                 'Linux hosting, named remote/provider or full Gate 6 acceptance'],
                  source_inputs_sha256=before, sources_unchanged=not changed, changed_inputs=changed,
                  steps=steps, passed=not changed and len(steps)==len(commands) and all(s['passed'] for s in steps))
    (output / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(passed=report['passed'], inputs=len(before), changed_inputs=changed)))
    raise SystemExit(0 if report['passed'] else 1)


if __name__ == '__main__':
    main()
