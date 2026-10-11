#!/usr/bin/env python3
"""Measure full-size MCP request bodies and deadlines on a separate local server."""
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
import traceback

from lib.local_process import ROOT, BINARY, Server, port, run
from lib.profile_metrics import ProfileReport
from lib.mcp_load_http import Endpoint
from lib import mcp_load_profile

sys.path.insert(0, str(ROOT / 'sdk/python'))
from connect_sdk import NativeClient

BUDGET = 180


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inputs():
    paths = ['scripts/check-mcp-load.py', 'scripts/lib/mcp_load_http.py', 'scripts/lib/mcp_load_profile.py',
             'scripts/lib/local_process.py', 'scripts/lib/profile_metrics.py']
    paths += [str(path.relative_to(ROOT)) for path in (ROOT / 'sdk/python/connect_sdk').glob('*.py')]
    return {path: digest(ROOT / path) for path in paths}


def expired(_signal, _frame):
    raise TimeoutError('MCP load profile exceeded 180 seconds')


def qualify(binary, directory, report):
    state = directory / 'state'
    run('init', '--directory', state, '--example', 'review-loop', '--bind', f'127.0.0.1:{port()}', binary=binary)
    http, mcp = f'127.0.0.1:{port()}', f'127.0.0.1:{port()}'
    with (state / 'server.toml').open('a') as stream:
        stream.write(f'\n[mcp]\nbind = "{mcp}"\n')
    peer = tomllib.loads((state / 'agent.toml').read_text())['identity_id']
    environment = {key: os.environ[key] for key in ('PATH', 'HOME', 'LANG', 'TMPDIR') if key in os.environ}
    with Server(state, binary=binary, env=environment, extra_args=('--http-bind', http),
                log_path=directory / 'server.log') as server, report.sample(server.process.pid):
        with NativeClient(state / 'operator.toml', executable=str(binary)) as owner:
            cid = owner.create('load-conversation', [peer])['id']
            owner.negotiate_tasks('connect:agent@local', cid)
            scope = {'operations': ['history', 'inbox'], 'recipients': [peer]}
            task = owner.task({'action': 'create', 'request_id': 'load-task', 'definition': {
                'conversation_id': cid, 'scope': scope, 'expires_at': int(time.time()) + 600}})['data']
            source_ids = set()
            for index in range(32):
                receipt = owner.send(f'load-source-{index}', cid, f'Source {index}: ' + 'x' * 8000)
                source_ids.add(receipt['message_id'])
            issue = {'profile': 'connect-task-access/1', 'command': {'action': 'issue', 'request_id': 'load-grant',
                     'definition': {'task_id': task['id'], 'binding': {'kind': 'host_session', 'adapter': 'mcp-load', 'external_id': 'load-profile'},
                                    'operations': scope['operations'], 'expires_at': int(time.time()) + 180}}}
            (directory / 'issue.json').write_text(json.dumps(issue))
            run('task-access', 'issue', '--config', state / 'operator.toml', '--http-address', http,
                '--input', directory / 'issue.json', '--output', directory / 'grant.json', binary=binary)
            token = json.loads((directory / 'grant.json').read_text())['grant']['bearer']
        # The load deliberately lasts beyond native idle expiry. Setup does not
        # keep an unused QUIC observer alive; verify through a fresh public client.
        endpoint = Endpoint(mcp, token)
        maximum = mcp_load_profile.framing(endpoint, task, report)
        mcp_load_profile.slow_bodies(endpoint, maximum, task, report)
        mcp_load_profile.pending_waits(endpoint, report)
        mcp_load_profile.recall(endpoint, source_ids, report)
        with NativeClient(state / 'operator.toml', executable=str(binary)) as observer:
            assert {message['id'] for message in observer.history(cid)['messages']} == source_ids
        report.checkpoint(no_message_effects_from_load=True)
    assert report.rss, 'Missing server memory observations'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=BINARY)
    args = parser.parse_args()
    binary = args.binary.resolve()
    if not binary.is_file():
        parser.error('Build or select a connect binary before this profile')
    os.umask(0o077)
    directory = ROOT / '.local/mcp-load' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    directory.mkdir(parents=True, mode=0o700)
    source = inputs()
    report = ProfileReport(directory, {
        'scope': 'local macOS arm64; separate server, native task setup and raw HTTP load probes; no providers',
        'budget_seconds': BUDGET, 'source_revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        'source_inputs_sha256': source, 'binary': str(binary),
        'limits': {'request_body_bytes': 512 * 1024, 'request_admissions': 32, 'http_deadline_seconds': 10, 'tool_projection_bytes': 32 * 1024},
        'excluded': ['raw load probes are not a replacement MCP SDK', 'provider latency', 'whole G4-030 qualification'],
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
    print(('FAIL' if error else 'PASS') + ' MCP load profile: ' + str(directory / 'result.json'))
    return int(error is not None)


if __name__ == '__main__':
    sys.exit(main())
