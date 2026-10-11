#!/usr/bin/env python3
"""Qualify artifact and task age retention under 32-task load and restart."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import signal
import subprocess
import sys
import time
import traceback

from lib.local_process import ROOT, BINARY, Server, port, run
from lib.profile_metrics import ProfileReport
from lib import retention_profile as profile
from lib.retention_setup import prepare, deny_renewal

sys.path.insert(0, str(ROOT / 'sdk/python'))
from connect_sdk import NativeClient

BUDGET = 180


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inputs():
    paths = ['scripts/check-retention-load.py', 'scripts/lib/mcp_load_http.py', 'scripts/lib/retention_profile.py', 'scripts/lib/retention_setup.py',
             'scripts/lib/local_process.py', 'scripts/lib/profile_metrics.py']
    paths += [str(path.relative_to(ROOT)) for path in (ROOT / 'sdk/python/connect_sdk').glob('*.py')]
    return {path: digest(ROOT / path) for path in paths}


def expired(_signal, _frame):
    raise TimeoutError('Retention profile exceeded 180 seconds')


def qualify(binary, directory, report):
    state = directory / 'state'
    run('init', '--directory', state, '--example', 'review-loop', '--bind', f'127.0.0.1:{port()}', binary=binary)
    http, mcp = f'127.0.0.1:{port()}', f'127.0.0.1:{port()}'
    with (state / 'server.toml').open('a') as stream:
        stream.write(f'\n[mcp]\nbind = "{mcp}"\n')
    environment = {key: os.environ[key] for key in ('PATH', 'HOME', 'LANG', 'TMPDIR') if key in os.environ}
    with Server(state, binary=binary, env=environment, extra_args=('--http-bind', http),
                log_path=directory / 'server-cold.log') as server, report.sample(server.process.pid):
        with NativeClient(state / 'operator.toml', executable=str(binary)) as owner:
            jobs, live, task_expiry = prepare(binary, state, directory, http, mcp, owner)
        expiry = profile.upload(jobs, report)
        profile.expire_under_load(jobs, expiry, report)
        # Read-only physical observations supplement public API behavior.
        assert all((state / 'state.blobs' / (job['artifact']['id'] + '.blob')).is_file() for job in jobs)
    with Server(state, binary=binary, env=environment, extra_args=('--http-bind', http),
                log_path=directory / 'server-after-file-expiry.log') as server, report.sample(server.process.pid):
        with report.phase('restart-cleans-expired-bytes-but-preserves-sources'):
            profile.expired_files(jobs, report)
            with NativeClient(state / 'operator.toml', executable=str(binary)) as owner:
                profile.cleanup(state, jobs, live, owner, report)
        profile.task_expiry(jobs, task_expiry, report)
        with NativeClient(state / 'operator.toml', executable=str(binary)) as owner:
            profile.terminal_records(owner, jobs)
    with report.phase('restart-preserves-expired-task-records-and-conversations'):
        with Server(state, binary=binary, env=environment, extra_args=('--http-bind', http),
                    log_path=directory / 'server-after-task-expiry.log') as server, report.sample(server.process.pid):
            with NativeClient(state / 'operator.toml', executable=str(binary)) as owner:
                profile.terminal_records(owner, jobs)
                profile.cleanup(state, jobs, live, owner, report)
            deny_renewal(binary, state, http, jobs)
            report.checkpoint(retained_expired_task_records=32, original_messages_retained=32, expired_task_renewals_denied=32)
    assert report.rss, 'Missing server memory observations'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=BINARY)
    args = parser.parse_args()
    binary = args.binary.resolve()
    if not binary.is_file():
        parser.error('Build or select a connect binary before this profile')
    os.umask(0o077)
    directory = ROOT / '.local/retention-load' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    directory.mkdir(parents=True, mode=0o700)
    source = inputs()
    report = ProfileReport(directory, {
        'scope': 'local macOS arm64; separate server, public native setup, protected grants, MCP artifact/task expiry load; no providers',
        'budget_seconds': BUDGET, 'source_revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        'source_inputs_sha256': source, 'binary': str(binary),
        'profile': {'tasks': 32, 'task_seconds': 45, 'artifact_seconds': 12, 'artifact_bytes_each': 65536, 'chunk_bytes': 32768},
        'excluded': ['full 256 MiB artifact storage quota', 'retired voice setup metadata', 'whole G4-030 qualification'],
    })
    started, error = time.monotonic(), None
    previous = signal.signal(signal.SIGALRM, expired)
    signal.setitimer(signal.ITIMER_REAL, BUDGET)
    try:
        assert (platform.system(), platform.machine()) == ('Darwin', 'arm64')
        before = digest(binary)
        report.data['binary_sha256'] = before
        report.save()
        qualify(binary, directory, report)
        assert inputs() == source and digest(binary) == before, 'Source or executable changed during qualification'
    except BaseException as caught:
        error = caught
        traceback.print_exc()
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)
        report.data['elapsed_seconds'] = round(time.monotonic() - started, 3)
        report.finish('failed' if error else 'passed', directory / 'state', error)
    print(('FAIL' if error else 'PASS') + ' retention profile: ' + str(directory / 'result.json'))
    return int(error is not None)


if __name__ == '__main__':
    sys.exit(main())
