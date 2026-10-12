#!/usr/bin/env python3
"""Qualify grant expiry, renewal and retained receipts across 32 local tasks."""
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
from lib import grant_load_profile as profile

sys.path.insert(0, str(ROOT / 'sdk/python'))
from connect_sdk import NativeClient

BUDGET = 180


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inputs():
    paths = ['scripts/check-grant-lifecycle.py', 'scripts/lib/mcp_load_http.py', 'scripts/lib/grant_load_profile.py',
             'scripts/lib/local_process.py', 'scripts/lib/profile_metrics.py']
    paths += [str(path.relative_to(ROOT)) for path in (ROOT / 'sdk/python/connect_sdk').glob('*.py')]
    return {path: digest(ROOT / path) for path in paths}


def expired(_signal, _frame):
    raise TimeoutError('Grant lifecycle profile exceeded 180 seconds')


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
            jobs, expiry = profile.prepare(binary, state, directory, http, mcp, owner)
        profile.boundary(jobs, expiry, report)
        profile.renew(binary, state, http, mcp, jobs, report)
        with NativeClient(state / 'operator.toml', executable=str(binary)) as owner:
            profile.verify(owner, jobs)
        profile.cancel_half(jobs, report)
    with report.phase('restart-preserves-expiry-cancellation-and-original-effects'):
        with Server(state, binary=binary, env=environment, extra_args=('--http-bind', http),
                    log_path=directory / 'server-warm.log') as server, report.sample(server.process.pid):
            profile.expired(jobs, report)
            profile.cancellation_boundary(jobs, report)
            with NativeClient(state / 'operator.toml', executable=str(binary)) as owner:
                profile.verify(owner, jobs, cancelled=True)
            report.checkpoint(restart_old_grants_denied=True, restart_cancelled_grants_denied=True,
                              retained_original_messages=32, surviving_original_receipts=16)
    assert report.rss, 'Missing server memory observations'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=BINARY)
    args = parser.parse_args()
    binary = args.binary.resolve()
    if not binary.is_file():
        parser.error('Build or select a connect binary before this profile')
    os.umask(0o077)
    directory = ROOT / '.local/grant-lifecycle' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    directory.mkdir(parents=True, mode=0o700)
    source = inputs()
    report = ProfileReport(directory, {
        'scope': 'local macOS arm64; separate server, public native setup, protected grants, MCP expiry/renewal load; no providers',
        'budget_seconds': BUDGET, 'source_revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        'source_inputs_sha256': source, 'binary': str(binary),
        'profile': {'tasks': 32, 'initial_grant_seconds': 12, 'renewed_grant_seconds': 120},
        'excluded': ['artifact age retention', 'retired voice setup metadata', 'whole G4-030 qualification'],
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
    print(('FAIL' if error else 'PASS') + ' grant lifecycle profile: ' + str(directory / 'result.json'))
    return int(error is not None)


if __name__ == '__main__':
    sys.exit(main())
