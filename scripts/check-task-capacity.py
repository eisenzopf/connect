#!/usr/bin/env python3
"""Measure declared task ceilings against an isolated server via native Python SDK.

Prepare dependencies and build connect first. This full-size profile is explicit,
account-free and bounded to 900 seconds; it never edits an existing installation.
"""
import argparse
import datetime
import hashlib
import json
import os
import pathlib
import platform
import signal
import subprocess
import sys
import time
import tomllib

from lib.local_process import BINARY, ROOT, Server, port, run
from lib.profile_metrics import ProfileReport

sys.path.insert(0, str(ROOT / 'sdk/python'))
from connect_sdk import NativeClient  # noqa: E402
from lib.task_capacity import TaskCapacity  # noqa: E402
from lib.task_inbox_profile import qualify as qualify_inbox  # noqa: E402

BUDGET_SECONDS = 900


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inputs():
    paths = [pathlib.Path(__file__), ROOT / 'scripts/lib/task_capacity.py',
             ROOT / 'scripts/lib/profile_metrics.py', ROOT / 'scripts/lib/local_process.py',
             ROOT / 'scripts/lib/task_inbox_profile.py',
             ROOT / 'Cargo.lock', ROOT / 'dependencies/source-lock.json']
    paths.extend((ROOT / 'sdk/python/connect_sdk').glob('*.py'))
    return {str(p.relative_to(ROOT)): digest(p) for p in paths}


def budget_expired(_signal, _frame):
    raise TimeoutError('Task capacity profile exceeded its 900-second budget')


def qualify(binary, directory, report):
    state = directory / 'state'
    endpoint = f'127.0.0.1:{port()}'
    run('init', '--directory', state, '--example', 'review-loop', '--bind', endpoint,
        binary=binary)
    peer = tomllib.loads((state / 'agent.toml').read_text())['identity_id']
    owner = tomllib.loads((state / 'operator.toml').read_text())['identity_id']
    run('issue', '--config', state / 'operator.toml', '--server-config', state / 'server.toml',
        '--identity-id', peer, '--recipients', owner, '--output', state / 'respondent.toml',
        binary=binary)
    environment = {key: value for key, value in os.environ.items()
                   if key not in ('TELNYX_API_KEY', 'VAPI_PRIVATE_KEY', 'VAPI_API_KEY')}
    with Server(state, binary=binary, env=environment,
                log_path=directory / 'server-first.log') as server:
        with report.sample(server.process.pid), NativeClient(
                state / 'operator.toml', executable=str(binary)) as client:
            conversation = client.create('profile-conversation', [peer])
            client.negotiate_tasks('connect:agent@local', conversation['id'])
            definition = {'conversation_id': conversation['id'],
                          'scope': {'operations': ['send', 'history', 'inbox'], 'recipients': [peer]},
                          'expires_at': int(time.time()) + 1800, 'initiating_event_id': None}
            # Fixed input + deterministic IDs preserve the original admission
            # intent if a request becomes uncertain. No uncertain request retries.
            (directory / 'task-definition.json').write_text(
                json.dumps(definition, indent=2) + '\n')
            profile = TaskCapacity(client, definition, report)
            with report.phase('active-1024-and-overflow'):
                profile.active()
            with report.phase('sixteen-step-leases-and-release'):
                profile.claims()
            with report.phase('inbox-1000-pages-and-maximum-wait'):
                with NativeClient(state / 'respondent.toml', executable=str(binary)) as peer_client:
                    qualify_inbox(profile, peer_client)
            with report.phase('retained-10000-and-cancellation'):
                profile.retained()
    with report.phase('restart-at-retained-capacity'):
        with Server(state, binary=binary, env=environment,
                    log_path=directory / 'server-restart.log') as server:
            with report.sample(server.process.pid), NativeClient(
                    state / 'operator.toml', executable=str(binary)) as client:
                profile.after_restart(client)
    assert report.rss, 'No server RSS sample; resource measurement is incomplete'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=pathlib.Path, default=BINARY)
    args = parser.parse_args()
    os.umask(0o077)
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    directory = ROOT / '.local/task-capacity' / stamp
    directory.mkdir(parents=True, mode=0o700)
    binary = args.binary.resolve()
    source = inputs()
    report = ProfileReport(directory, {
        'scope': 'local macOS arm64; separate server and native SDK helper; no live providers',
        'budget_seconds': BUDGET_SECONDS,
        'source_revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        'source_inputs_sha256': source,
        'binary': str(binary),
        'limits': {'active_tasks': 1024, 'retained_tasks': 10000, 'step_leases': 16,
                   'unconsumed_replies': 1000, 'task_page': 100, 'task_wait_ms': 10000},
        'excluded': ['MCP payload/deadline profile', 'grant expiry lifecycle',
                     'sixteen simultaneous runner processes', 'throughput SLO',
                     'whole G4-030 qualification'],
    })
    started = time.monotonic()
    previous = signal.signal(signal.SIGALRM, budget_expired)
    signal.setitimer(signal.ITIMER_REAL, BUDGET_SECONDS)
    error = None
    try:
        assert (platform.system(), platform.machine()) == ('Darwin', 'arm64'), 'Unqualified platform'
        assert binary.is_file(), 'Build the connect binary before running this profile'
        before = digest(binary)
        report.data['binary_sha256'] = before
        report.save()
        qualify(binary, directory, report)
        assert digest(binary) == before, 'Binary changed during qualification'
        assert inputs() == source, 'Profile or SDK sources changed during qualification'
    except BaseException as caught:
        error = caught
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)
        report.data['elapsed_seconds'] = round(time.monotonic() - started, 3)
        report.finish('failed' if error else 'passed', directory / 'state', error)
    print(('FAIL' if error else 'PASS') + ' task capacity profile: ' + str(directory / 'result.json'))
    return 1 if error else 0


if __name__ == '__main__':
    sys.exit(main())
