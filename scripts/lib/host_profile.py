"""Public task setup and cold/warm concurrency using the shipped host composer."""
import concurrent.futures
import json
import sys
import threading
import time

from connect_sdk import NativeClient
from incoming.host import Host
from .host_profile_peer import Peer
from .host_profile_observation import snapshot, exited, verify_turn
from .local_process import ROOT

HOSTS = 16


def prepare(binary, codex, state, directory, http, mcp):
    """Create real source events and tasks; never seed the Connect database."""
    import tomllib
    peer_id = tomllib.loads((state / 'agent.toml').read_text())['identity_id']
    jobs = []
    try:
        with NativeClient(state / 'operator.toml', executable=str(binary)) as owner, NativeClient(
                state / 'human.toml', executable=str(binary)) as human:
            for index in range(HOSTS):
                cid = owner.create(f'host-conversation-{index}', [peer_id])['id']
                body = f'Concurrent contact {index}: indigo-{index:03}'
                message = human.send(f'host-contact-{index}', cid, json.dumps({
                    'action': 'contact_assistant', 'request_id': f'contact-{index}', 'body': body}))
                jobs.append({'cid': cid, 'message': message, 'body': body})
            events = owner.inbox()['events']
            for index, job in enumerate(jobs):
                event = next(e for e in events if (e.get('message') or {}).get('id') == job['message']['message_id'])
                owner.negotiate_tasks('connect:agent@local', job['cid'])
                job['task'] = owner.task({'action': 'create', 'request_id': f'host-task-{index}',
                                         'definition': {'conversation_id': job['cid'], 'initiating_event_id': event['id'],
                                         'scope': {'operations': ['send', 'history', 'inbox'], 'recipients': [peer_id]},
                                         'expires_at': int(time.time()) + 1800}})['data']
                path = directory / f'host-{index}'
                path.mkdir(mode=0o700)
                job.update(directory=path, peer=Peer())
                for home in ('codex-home', 'user-home'):
                    (path / home).mkdir(mode=0o700)
                shim = path / 'offline-runtime.py'
                source = (ROOT / 'crates/connect-server/tests/fixtures/codex-packaged-runtime.py').read_text()
                assert not any(c.isspace() for c in sys.executable)
                source = '#!' + sys.executable + '\n' + source.split('\n', 1)[1]
                assert source.count('args = sys.argv[1:]') == 1
                source = source.replace('args = sys.argv[1:]',
                                        "Path(__file__).with_suffix('.pid').write_text(str(os.getpid()))\nargs = sys.argv[1:]")
                shim.write_text(source)
                shim.chmod(0o700)
                shim.with_suffix('.json').write_text(json.dumps({
                    'binary': str(codex), 'provider': job['peer'].url,
                    'codex_home': str(path / 'codex-home'), 'user_home': str(path / 'user-home')}))
                job['shim'] = shim
                job['host'] = Host(binary, shim, state / 'operator.toml', http, mcp)
        return jobs
    except BaseException:
        close(jobs)
        raise


def turns(jobs, codex, report, warm=False):
    barrier, release = threading.Barrier(HOSTS + 1), threading.Event()
    for job in jobs:
        job['peer'].gate = barrier, release

    def turn(job):
        start = time.monotonic()
        if warm:
            result = job['host'].command(job['directory'], 'run-codex', '--installation', job['directory'] / 'host',
                                         '--codex-binary', job['shim'], '--timeout-seconds', '120')
        else:
            result = job['host'].run(job['directory'], job['task'])
        assert result['status'] == 'host-turn-completed' and result['task_id'] == job['task']['id']
        if warm:
            assert result['session_id'] == job['session_id'], 'Warm turn replaced the saved host session'
        job['session_id'] = result['session_id']
        verify_turn(job, warm)
        return time.monotonic() - start

    with concurrent.futures.ThreadPoolExecutor(max_workers=HOSTS) as executor:
        futures = [executor.submit(turn, job) for job in jobs]
        try:
            barrier.wait(timeout=60)
            observed = snapshot(jobs, codex)
            time.sleep(1)
            second = snapshot(jobs, codex)
            assert [o['pid'] for o in observed] == [o['pid'] for o in second]
            report.checkpoint(**{('warm' if warm else 'cold') + '_simultaneous_hosts': {
                'count': HOSTS, 'rss_bytes': sum(o['rss_bytes'] for o in second),
                'observations': second, 'scope': 'Codex main processes only; excludes child tools and driver; gated idle sample'}})
        except BaseException:
            barrier.abort()
            raise
        finally:
            release.set()
        for future in futures:
            report.latency(future.result(timeout=155))
    exited(observed)
    report.checkpoint(**{('warm' if warm else 'cold') + '_cleanup': {
        'observed_host_groups': HOSTS, 'live_members_after_completion': [], 'verified_tool_triplets': HOSTS}})
    assert len({job['session_id'] for job in jobs}) == HOSTS
    for job in jobs:
        assert job['peer'].errors == [], job['peer'].errors
        assert len(job['peer'].requests) == (4 if warm else 2)


def verify(binary, state, jobs):
    with NativeClient(state / 'human.toml', executable=str(binary)) as human:
        for job in jobs:
            messages = human.history(job['cid'])['messages']
            assert len(messages) == 2, 'Replay duplicated an effect or another task wrote this conversation'
            assert messages[1]['body'] == f"Received from {job['message']['message_id']}: {job['body']}"


def close(jobs):
    for job in jobs:
        if 'peer' in job:
            job['peer'].close()
