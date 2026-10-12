#!/usr/bin/env python3
"""Qualify 16 real native reference runners, lease admission and durable follow-through."""
import argparse
import datetime
import hashlib
import os
from pathlib import Path
import platform
import signal
import subprocess
import sys
import time
import tomllib
import traceback

from lib.local_process import ROOT, BINARY, Server, port, run
from lib.profile_metrics import ProfileReport

sys.path.insert(0, str(ROOT / 'sdk/python'))
from connect_sdk import NativeClient
from lib import runner_profile
from lib.runner_profile_worker import stop_all

BUDGET = 240


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inputs():
    paths = ['scripts/check-runner-capacity.py', 'scripts/lib/runner_profile.py',
             'scripts/lib/runner_profile_worker.py', 'scripts/lib/profile_metrics.py',
             'scripts/lib/local_process.py']
    paths += [str(path.relative_to(ROOT)) for path in (ROOT / 'sdk/python/connect_sdk').glob('*.py')]
    return {path: digest(ROOT / path) for path in paths}


def expired(_signal, _frame):
    raise TimeoutError('Runner concurrency profile exceeded 240 seconds')


def qualify(binary, example, directory, report):
    state = directory / 'state'
    run('init', '--directory', state, '--example', 'review-loop', '--bind', f'127.0.0.1:{port()}', binary=binary)
    http = f'127.0.0.1:{port()}'
    owner = tomllib.loads((state / 'operator.toml').read_text())['identity_id']
    peer = tomllib.loads((state / 'agent.toml').read_text())['identity_id']
    run('issue', '--config', state / 'operator.toml', '--server-config', state / 'server.toml',
        '--identity-id', peer, '--recipients', owner, '--output', state / 'human.toml', binary=binary)
    environment = {key: os.environ[key] for key in ('PATH', 'HOME', 'LANG', 'TMPDIR') if key in os.environ}
    workers = []
    try:
        with Server(state, binary=binary, env=environment, extra_args=('--http-bind', http),
                    log_path=directory / 'server-cold.log') as server, report.sample(server.process.pid):
            with NativeClient(state / 'operator.toml', executable=str(binary)) as owner, NativeClient(
                    state / 'human.toml', executable=str(binary)) as human:
                workers = runner_profile.prepare(binary, example, state, directory, http, owner)
                runner_profile.cold(owner, human, workers, environment, report)
        with Server(state, binary=binary, env=environment, extra_args=('--http-bind', http),
                    log_path=directory / 'server-warm.log') as server, report.sample(server.process.pid):
            with NativeClient(state / 'human.toml', executable=str(binary)) as human:
                runner_profile.warm(human, workers, environment, report)
        assert report.rss, 'Missing server memory measurements'
        report.checkpoint(all_started_workers_reaped=True)
    finally:
        stop_all(workers, primary_error=sys.exception())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=BINARY)
    parser.add_argument('--example-binary', type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    example = (args.example_binary or binary.with_name('connect-example')).resolve()
    if not binary.is_file() or not example.is_file():
        parser.error('Build or select both connect and connect-example before this profile')
    os.umask(0o077)
    directory = ROOT / '.local/runner-capacity' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    directory.mkdir(parents=True, mode=0o700)
    source = inputs()
    report = ProfileReport(directory, {
        'scope': 'local macOS arm64; real reference workers, public native SDK and protected CLI; no providers',
        'budget_seconds': BUDGET, 'source_revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        'source_inputs_sha256': source, 'binary': str(binary), 'example_binary': str(example),
        'limits': {'simultaneous_workers_tested': 16, 'step_lease_ceiling': 16, 'step_lease_seconds': 30},
        'excluded': ['16 is not a worker process admission ceiling', 'throughput SLO', 'whole G4-030 qualification'],
    })
    started, error = time.monotonic(), None
    previous = signal.signal(signal.SIGALRM, expired)
    signal.setitimer(signal.ITIMER_REAL, BUDGET)
    try:
        assert (platform.system(), platform.machine()) == ('Darwin', 'arm64')
        before = digest(binary), digest(example)
        report.data.update(binary_sha256=before[0], example_sha256=before[1])
        report.save()
        qualify(binary, example, directory, report)
        assert inputs() == source and before == (digest(binary), digest(example)), 'Source or binary changed during qualification'
    except BaseException as caught:
        error = caught
        if hasattr(caught, 'worker_startup'):
            report.checkpoint(worker_startup=caught.worker_startup)
        traceback.print_exc()
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)
        report.data['elapsed_seconds'] = round(time.monotonic() - started, 3)
        report.finish('failed' if error else 'passed', directory / 'state', error)
    print(('FAIL' if error else 'PASS') + ' runner capacity profile: ' + str(directory / 'result.json'))
    return int(error is not None)


if __name__ == '__main__':
    sys.exit(main())
