#!/usr/bin/env python3
"""Run bounded local checks, retaining private logs and a machine-readable result."""
import argparse
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

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--ci', action='store_true', help='Source checks without private archived-binary migration qualification')
parser.add_argument('--package', type=pathlib.Path, help='Also qualify a local candidate directory')
parser.add_argument('--previous-binary', type=pathlib.Path, default=ROOT/'.local/milestones/m1/connect')
args = parser.parse_args()
if args.ci and args.package:
    parser.error("Package qualification requires the full archived-binary recovery check")
os.umask(0o077)
directory = ROOT/'.local/checks'/datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
directory.mkdir(parents=True, mode=0o700)
report = {'scope': 'local macOS arm64; no remote deployment or live providers',
          'archived_binary_qualification': 'not run (explicit CI source-check mode)' if args.ci else 'required',
          'source_revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
          'steps': [], 'result': 'running'}
report['source_inputs'] = {
    str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
    for directory in ['crates', 'examples', 'scripts', 'patches', 'dependencies', 'sdk', 'contracts', '.github']
    for path in sorted((ROOT/directory).rglob('*'))
    if path.is_file() and not {'__pycache__', 'node_modules', 'dist'}.intersection(path.parts) and path.suffix != '.pyc'
}
for name in ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml']:
    report['source_inputs'][name] = hashlib.sha256((ROOT/name).read_bytes()).hexdigest()
environment = dict(os.environ, CONNECT_PREVIOUS_BINARY=str(args.previous_binary.resolve()))
steps = [
    ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 600),
    ('format', ['bash', 'scripts/format.sh', '--check'], 120),
    ('lint', ['cargo', 'clippy', '--locked', '--workspace', '--all-targets', '--', '-D', 'warnings'], 2400),
    ('typescript-sdk-install', ['npm', 'ci', '--prefix', 'sdk/typescript'], 300),
    ('typescript-sdk-build', ['npm', 'run', 'build', '--prefix', 'sdk/typescript'], 120),
    ('typescript-sdk-format', ['npm', 'run', 'format:check', '--prefix', 'sdk/typescript'], 120),
    ('typescript-sdk-failures', ['node', '--test', 'sdk/tests/clients.test.mjs'], 120),
    ('python-sdk-failures', ['env', 'PYTHONPATH=sdk/python', 'python3', '-m', 'unittest', 'discover', '-s', 'sdk/tests', '-p', 'test_*.py'], 120),
    ('workspace', ['cargo', 'test', '--locked', '--workspace'], 2400),
    ('connector-example', ['cargo', 'run', '--locked', '-p', 'connect-connector-api', '--example', 'echo'], 600),
    ('upstream', ['bash', 'scripts/check-baseline.sh'], 2400),
    ('dispatch', ['python3', 'scripts/check-dispatch.py'], 600),
    ('typescript-probe', ['python3', 'scripts/check-typescript.py'], 600),
    ('datagram-probe', ['cargo', 'run', '--locked', '-p', 'connect-server', '--example', 'quic_audio'], 600),
    ('identity-process', ['python3', 'scripts/check-identity.py'], 600),
    ('recovery-process', ['python3', 'scripts/check-recovery.py'] + (['--current-only'] if args.ci else []), 600),
    ('crash-replay', ['python3', 'scripts/check-crash-replay.py'], 600),
    ('examples', ['python3', 'scripts/check-examples.py'], 600),
    ('example-recovery', ['python3', 'scripts/check-example-recovery.py'], 600),
]
if args.package:
    steps.append(('package', ['python3', 'scripts/check-package.py', str(args.package.resolve())], 1800))

def save():
    (directory/'result.json').write_text(json.dumps(report, indent=2)+'\n')

save()
print('Private results:', directory, flush=True)
for name, command, timeout in steps:
    print('RUN', name, flush=True)
    started = time.monotonic()
    result = {'name': name, 'command': command, 'log': name+'.log'}
    with (directory/result['log']).open('w') as log:
        process = subprocess.Popen(command, cwd=ROOT, env=environment, stdout=log,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        try:
            result['exit_code'] = process.wait(timeout=timeout)
        except BaseException as error:
            # Every child belongs to this newly created process group.
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            result.update(exit_code=process.returncode, error=type(error).__name__)
    result['seconds'] = round(time.monotonic()-started, 2)
    result['result'] = 'passed' if result['exit_code'] == 0 and 'error' not in result else 'failed'
    report['steps'].append(result)
    save()
    print(result['result'].upper(), name, f"({result['seconds']}s)", flush=True)
    if result['result'] != 'passed':
        report['result'] = 'failed'
        report['not_run'] = [entry[0] for entry in steps[len(report['steps']):]]
        save()
        print('Inspect private log:', directory/result['log'], file=sys.stderr)
        sys.exit(1)
report['result'] = 'passed'
save()
print('PASS all local checks; result:', directory/'result.json', flush=True)
