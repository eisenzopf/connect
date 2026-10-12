"""Public task/grant setup and persisted intents for the retention workload."""
import base64
import hashlib
import json
import time
import tomllib

from .local_process import run
from .mcp_load_http import ADMISSIONS, Endpoint, encode

PERMISSIONS = ['history', 'artifact_begin', 'artifact_append', 'artifact_finish', 'artifact_get']
LIVE_BYTES = b'live artifact control' * 100


def prepare(binary, state, directory, http, mcp, owner):
    peer = tomllib.loads((state / 'agent.toml').read_text())['identity_id']
    jobs, task_expiry = [], int(time.time()) + 45
    for index in range(ADMISSIONS):
        cid = owner.create(f'retention-conversation-{index}', [peer])['id']
        message = owner.send(f'retention-message-{index}', cid, f'Retained context {index}.')
        owner.negotiate_tasks('connect:agent@local', cid)
        task = owner.task({'action': 'create', 'request_id': f'retention-task-{index}', 'definition': {
            'conversation_id': cid, 'scope': {'operations': PERMISSIONS, 'recipients': [peer]},
            'expires_at': task_expiry}})['data']
        path = directory / f'task-{index}'
        path.mkdir(mode=0o700)
        binding = {'kind': 'host_session', 'adapter': 'retention-load', 'external_id': f'task-{index}'}
        intent = {'profile': 'connect-task-access/1', 'command': {'action': 'issue', 'request_id': f'retention-grant-{index}',
                  'definition': {'task_id': task['id'], 'binding': binding, 'operations': PERMISSIONS, 'expires_at': task_expiry}}}
        (path / 'grant-intent.json').write_text(json.dumps(intent))
        run('task-access', 'issue', '--config', state / 'operator.toml', '--http-address', http,
            '--input', path / 'grant-intent.json', '--output', path / 'grant.json', binary=binary)
        token = json.loads((path / 'grant.json').read_text())['grant']['bearer']
        data = bytes((offset + index) % 251 for offset in range(65536))
        jobs.append({'index': index, 'task': task, 'directory': path, 'endpoint': Endpoint(mcp, token),
                     'bytes': data, 'message': message})
    cid = jobs[0]['task']['conversation_id']
    live = owner.artifact_begin('retention-live', cid, 'live.bin', 'application/octet-stream',
                                len(LIVE_BYTES), hashlib.sha256(LIVE_BYTES).hexdigest())
    owner.artifact_append('retention-live-append', cid, live['id'], 0, base64.b64encode(LIVE_BYTES).decode())
    owner.artifact_finish('retention-live-finish', cid, live['id'])
    return jobs, live, task_expiry


def call(job, tool, arguments):
    """Persist effect IDs before dispatch; never rewrite intent on replay."""
    if 'request_id' in arguments:
        path = job['directory'] / (arguments['request_id'] + '.json')
        value = json.dumps({'task_id': job['task']['id'], 'tool': tool, 'arguments': arguments}, sort_keys=True)
        if path.exists():
            assert path.read_text() == value, 'Changed replay intent'
        else:
            path.write_text(value)
    return job['endpoint'].request(encode(tool, arguments))


def deny_renewal(binary, state, http, jobs):
    """Current owner authority cannot extend an expired task via a new grant."""
    for job in jobs:
        path = job['directory']
        original = json.loads((path / 'grant-intent.json').read_text())
        original['command']['request_id'] = f'expired-renewal-{job["index"]}'
        original['command']['definition']['expires_at'] = int(time.time()) + 120
        (path / 'expired-renewal-intent.json').write_text(json.dumps(original))
        result = run('task-access', 'issue', '--config', state / 'operator.toml', '--http-address', http,
                     '--input', path / 'expired-renewal-intent.json', '--output', path / 'expired-renewal-grant.json',
                     binary=binary, success=False)
        assert '(forbidden)' in result.stderr, 'Expired task renewal did not fail for current authority'
