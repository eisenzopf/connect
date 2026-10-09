#!/usr/bin/env python3
"""Run Gate 3's bounded offline checks without provider or browser account effects."""
import datetime
import hashlib
import json
import os
import pathlib
import signal
import subprocess
import sys
import time

from lib.local_process import ROOT

os.umask(0o077)


def source_fingerprint():
    """Track source bytes so a long run cannot silently mix implementations."""
    paths = subprocess.check_output(['git', 'ls-files', '-z'], cwd=ROOT).split(b'\0')
    digest = hashlib.sha256()
    for path in sorted(filter(None, paths)):
        file = ROOT / os.fsdecode(path)
        digest.update(path + b'\0')
        digest.update(hashlib.sha256(file.read_bytes()).digest() if file.is_file() else b'missing')
    return digest.hexdigest()


fingerprint = source_fingerprint()
directory = ROOT / '.local' / 'gate-3-checks' / datetime.datetime.now(
    datetime.timezone.utc
).strftime('%Y%m%dT%H%M%S.%fZ')
directory.mkdir(parents=True, mode=0o700)
report = {
    'scope': 'Gate 3 offline; local macOS arm64; no provider calls, account changes, or remote deployment',
    'source_revision': subprocess.check_output(
        ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True
    ).strip(),
    'dirty': bool(subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT)),
    'started_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
    'steps': [],
    'result': 'running',
    'source_fingerprint_sha256': fingerprint,
    'live_qualification': 'not run; separately invoked and required for provider-route closure',
}
for name in [
    'Cargo.toml',
    'Cargo.lock',
    'contracts/connect-control-v1.md',
    'dependencies/source-lock.json',
]:
    report.setdefault('inputs', {})[name] = hashlib.sha256((ROOT / name).read_bytes()).hexdigest()

steps = [
    ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 600),
    ('format', ['bash', 'scripts/format.sh', '--check'], 120),
    ('lint', ['cargo', 'clippy', '--locked', '--workspace', '--all-targets', '--', '-D', 'warnings'], 2400),
    ('contract-fixtures', ['cargo', 'test', '--locked', '-p', 'connect-server', '--test', 'contract_fixtures'], 600),
    ('durable-model', ['cargo', 'test', '--locked', '-p', 'connect-store'], 900),
    ('native-media-parity', ['cargo', 'test', '--locked', '-p', 'connect-server', '--test', 'gate3_parity'], 900),
    ('native-media-faults', ['cargo', 'test', '--locked', '-p', 'connect-server', '--test', 'gate3_media_faults', '--test', 'gate3_media_authorization'], 900),
    ('native-sdk-media-helper', ['cargo', 'test', '--locked', '-p', 'connect-server', '--test', 'gate3_sdk_media_helper'], 900),
    ('media-pressure-smoke', ['cargo', 'test', '--locked', '-p', 'connect-server', '--test', 'gate3_media_pressure', '--', '--include-ignored', '--nocapture'], 1200),
    ('vapi-duplex-media', ['cargo', 'test', '--locked', '-p', 'connect-server', '--test', 'gate3_vapi_media', '--test', 'gate3_vapi_queue', '--', '--nocapture'], 900),
    ('vapi-observer-lifecycle', ['cargo', 'test', '--locked', '-p', 'connect-server', '--lib', 'vapi_observer'], 600),
    ('provider-chat-context', ['cargo', 'test', '--locked', '-p', 'connect-connectors', '--lib', '--test', 'adapters'], 600),
    ('provider-chat-flow', ['cargo', 'test', '--locked', '-p', 'connect-server', '--test', 'provider_flow'], 600),
    ('vapi-adapter-failures', ['env', 'CARGO_TARGET_DIR=target/upstream', 'cargo', 'test', '--locked', '--manifest-path', '.deps/rvoip/Cargo.toml', '-p', 'rvoip-vapi', '--test', 'mock_transport'], 1200),
    ('transport-queue-budgets', ['env', 'CARGO_TARGET_DIR=target/upstream', 'cargo', 'test', '--locked', '--manifest-path', '.deps/rvoip/Cargo.toml', '-p', 'rvoip-sip', '-p', 'rvoip-webrtc', '--lib', 'queue_budget_tests'], 1200),
    ('sip-duplex-media', ['cargo', 'test', '--locked', '-p', 'connect-server', '--test', 'gate3_sip_media'], 900),
    ('sip-registration-lifecycle', ['cargo', 'test', '--locked', '-p', 'connect-server', '--lib', 'sip_registration'], 600),
    ('sip-codec-failures', ['env', 'CARGO_TARGET_DIR=target/upstream', 'cargo', 'test', '--locked', '--manifest-path', '.deps/rvoip/Cargo.toml', '-p', 'rvoip-sip', '--test', 'adapter_renegotiate'], 1200),
    ('webrtc-duplex-media', ['cargo', 'test', '--locked', '-p', 'connect-server', '--test', 'gate3_webrtc_media'], 900),
    ('webrtc-security', ['cargo', 'test', '--locked', '-p', 'connect-server', '--test', 'gate3_webrtc_security'], 600),
    ('sip-webrtc-interop', ['cargo', 'test', '--locked', '-p', 'connect-server', '--test', 'gate3_sip_webrtc'], 900),
    ('artifact-recovery', ['cargo', 'test', '--locked', '-p', 'connect-server', '--test', 'artifact_recovery'], 900),
    ('artifact-lifecycle', ['cargo', 'test', '--locked', '-p', 'connect-store', '--test', 'gate3_artifact_lifecycle'], 600),
    ('transport-authority', ['cargo', 'test', '--locked', '-p', 'connect-server', '--test', 'transport_parity'], 900),
    ('language-sdk-facade', ['cargo', 'test', '--locked', '-p', 'connect-server', '--test', 'language_sdks'], 900),
    ('typescript-sdk', ['npm', 'test', '--prefix', 'sdk/typescript'], 300),
    ('python-sdk', ['env', 'PYTHONPATH=sdk/python', 'python3', '-m', 'unittest', 'discover', '-s', 'sdk/tests', '-p', 'test_*.py'], 300),
]


def save():
    (directory / 'result.json').write_text(json.dumps(report, indent=2) + '\n')


save()
print('Private Gate 3 results:', directory, flush=True)
for index, (name, command, timeout) in enumerate(steps):
    print('RUN', name, flush=True)
    started = time.monotonic()
    result = {'name': name, 'command': command, 'log': name + '.log'}
    with (directory / result['log']).open('w') as log:
        process = subprocess.Popen(
            command,
            cwd=ROOT,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        try:
            result['exit_code'] = process.wait(timeout=timeout)
        except BaseException as error:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            result.update(exit_code=process.returncode, error=type(error).__name__)
    result['seconds'] = round(time.monotonic() - started, 2)
    result['result'] = (
        'passed' if result['exit_code'] == 0 and 'error' not in result else 'failed'
    )
    report['steps'].append(result)
    save()
    print(result['result'].upper(), name, f"({result['seconds']}s)", flush=True)
    if result['result'] != 'passed':
        report['result'] = 'failed'
        report['not_run'] = [step[0] for step in steps[index + 1 :]]
        report['finished_at'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        save()
        print('Inspect private log:', directory / result['log'], file=sys.stderr)
        sys.exit(1)

report['source_stable'] = source_fingerprint() == fingerprint
report['result'] = 'passed' if report['source_stable'] else 'failed-source-changed'
report['finished_at'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
save()
if not report['source_stable']:
    print('Source changed during checks; retained results require a stable-source rerun', file=sys.stderr)
    sys.exit(1)
print('PASS Gate 3 offline checks; result:', directory / 'result.json', flush=True)
