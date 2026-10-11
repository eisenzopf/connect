#!/usr/bin/env python3
"""Measure 16 actual local Codex hosts through Connect; scripted Responses only."""
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
import tomllib

from lib.local_process import ROOT, BINARY, Server, port, run
from lib.profile_metrics import ProfileReport

sys.path[:0] = [str(ROOT / 'sdk/python'), str(ROOT / 'examples/agent-presence'),
               str(ROOT / 'examples/agent-presence/tests')]
from lib import host_profile

BUDGET = 480


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inputs():
    paths = ['scripts/check-host-capacity.py', 'scripts/lib/host_profile.py', 'scripts/lib/host_profile_peer.py',
             'scripts/lib/host_profile_observation.py',
             'scripts/lib/profile_metrics.py', 'scripts/lib/local_process.py',
             'examples/agent-presence/tests/incoming_runtime_peer.py',
             'crates/connect-server/tests/fixtures/codex-packaged-runtime.py',
             'crates/connect-server/tests/fixtures/codex-workflow-tools.js']
    paths += [str(p.relative_to(ROOT)) for folder in ('sdk/python/connect_sdk', 'examples/agent-presence/incoming')
              for p in (ROOT / folder).glob('*.py')]
    return {p: digest(ROOT / p) for p in paths}


def expired(_signal, _frame):
    raise TimeoutError('Host concurrency profile exceeded 480 seconds')


def qualify(binary, codex, directory, report):
    state = directory / 'state'
    run('init', '--directory', state, '--example', 'review-loop', '--bind', f'127.0.0.1:{port()}', binary=binary)
    http, mcp = f'127.0.0.1:{port()}', f'127.0.0.1:{port()}'
    with (state / 'server.toml').open('a') as stream:
        stream.write(f'\n[mcp]\nbind = "{mcp}"\n')
    owner = tomllib.loads((state / 'operator.toml').read_text())
    peer = tomllib.loads((state / 'agent.toml').read_text())
    run('issue', '--config', state / 'operator.toml', '--server-config', state / 'server.toml',
        '--identity-id', peer['identity_id'], '--recipients', owner['identity_id'],
        '--output', state / 'human.toml', binary=binary)
    env = {key: os.environ[key] for key in ('PATH', 'HOME', 'LANG', 'TMPDIR') if key in os.environ}
    jobs = []
    try:
        with Server(state, binary=binary, env=env, extra_args=('--http-bind', http),
                    log_path=directory / 'server-cold.log') as server, report.sample(server.process.pid):
            with report.phase('cold-sixteen-hosts'):
                jobs = host_profile.prepare(binary, codex, state, directory, http, mcp)
                host_profile.turns(jobs, codex, report)
                host_profile.verify(binary, state, jobs)
        with Server(state, binary=binary, env=env, extra_args=('--http-bind', http),
                    log_path=directory / 'server-warm.log') as server, report.sample(server.process.pid):
            with report.phase('warm-sixteen-exact-resumes-after-server-restart'):
                host_profile.turns(jobs, codex, report, warm=True)
                host_profile.verify(binary, state, jobs)
                for job in jobs:
                    result = job['host'].run(job['directory'], job['task'])
                    assert result['session_id'] == job['session_id']
                    assert len(job['peer'].requests) == 4, 'Checkpoint replay launched another turn'
        wire = json.dumps([job['peer'].requests for job in jobs])
        assert owner['credential'] not in wire
        for job in jobs:
            grant = json.loads((job['directory'] / 'host/task-grant.json').read_text())
            assert grant['grant']['bearer'] not in wire
        assert report.rss, 'Missing server memory observations'
    finally:
        host_profile.close(jobs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=BINARY)
    parser.add_argument('--codex-binary', type=Path, default=os.environ.get('CONNECT_TEST_CODEX_BINARY'),
                        help='Explicit installed Codex, or CONNECT_TEST_CODEX_BINARY from the qualification runner')
    args = parser.parse_args()
    if args.codex_binary is None:
        parser.error('Select --codex-binary or set CONNECT_TEST_CODEX_BINARY explicitly')
    os.umask(0o077)
    directory = ROOT / '.local/host-capacity' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    directory.mkdir(parents=True, mode=0o700)
    binary, codex = args.binary.resolve(), args.codex_binary.resolve()
    source = inputs()
    report = ProfileReport(directory, {'scope': 'local macOS arm64; 16 actual Codex hosts, scripted loopback Responses; no inference or providers',
                            'budget_seconds': BUDGET, 'source_revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                            'source_inputs_sha256': source, 'binary': str(binary), 'codex_binary': str(codex),
                            'limits': {'simultaneous_hosts_tested': 16, 'host_turn_seconds': 120},
                            'excluded': ['16 is not a host admission ceiling', 'reference-runner lease capacity', 'throughput SLO', 'whole Gate 4 qualification']})
    started, error = time.monotonic(), None
    previous = signal.signal(signal.SIGALRM, expired)
    signal.setitimer(signal.ITIMER_REAL, BUDGET)
    try:
        assert (platform.system(), platform.machine()) == ('Darwin', 'arm64')
        before = digest(binary), digest(codex)
        report.data.update(binary_sha256=before[0], codex_sha256=before[1],
                           codex_version=subprocess.check_output([str(codex), '--version'], text=True, timeout=15).strip())
        report.save()
        qualify(binary, codex, directory, report)
        assert inputs() == source and before == (digest(binary), digest(codex)), 'Source or executable changed during qualification'
    except BaseException as caught:
        error = caught
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)
        report.data['elapsed_seconds'] = round(time.monotonic() - started, 3)
        report.finish('failed' if error else 'passed', directory / 'state', error)
    print(('FAIL' if error else 'PASS') + ' host capacity profile: ' + str(directory / 'result.json'))
    return int(error is not None)


if __name__ == '__main__':
    sys.exit(main())
