"""Current grant authority, original effects and autonomous task cancellation under load."""
import hashlib
import json
import time
import tomllib

from .local_process import run
from .mcp_load_http import ADMISSIONS, Endpoint, burst, encode, success

OPERATIONS = ['send', 'history', 'task_cancel']


def issue(binary, state, http, job, generation, expiry):
    path = job['directory']
    intent = {'profile': 'connect-task-access/1', 'command': {
        'action': 'issue', 'request_id': f'grant-{generation}-{job["index"]}',
        'definition': {'task_id': job['task']['id'], 'binding': job['binding'],
                       'operations': OPERATIONS, 'expires_at': expiry}}}
    input_path, output_path = path / f'{generation}-intent.json', path / f'{generation}-grant.json'
    input_path.write_text(json.dumps(intent))
    run('task-access', 'issue', '--config', state / 'operator.toml', '--http-address', http,
        '--input', input_path, '--output', output_path, binary=binary)
    return json.loads(output_path.read_text())['grant']


def prepare(binary, state, directory, http, mcp, owner):
    peer = tomllib.loads((state / 'agent.toml').read_text())['identity_id']
    jobs = []
    for index in range(ADMISSIONS):
        cid = owner.create(f'grant-load-{index}', [peer])['id']
        owner.negotiate_tasks('connect:agent@local', cid)
        task = owner.task({'action': 'create', 'request_id': f'grant-task-{index}', 'definition': {
            'conversation_id': cid, 'scope': {'operations': OPERATIONS, 'recipients': [peer]},
            'expires_at': int(time.time()) + 600}})['data']
        path = directory / f'task-{index}'
        path.mkdir(mode=0o700)
        jobs.append({'index': index, 'directory': path, 'task': task,
                     'binding': {'kind': 'host_session', 'adapter': 'grant-load', 'external_id': f'task-{index}'},
                     'send': {'request_id': f'original-send-{index}', 'step_id': 'question', 'body': f'Original question {index}.'}})
    for job in jobs:
        job['cancel'] = {'request_id': f'cancel-own-{job["index"]}', 'reason': 'profile completed'}
        (job['directory'] / 'work-intent.json').write_text(json.dumps({
            'task_id': job['task']['id'], 'send': job['send'], 'cancel': job['cancel']}))
    expiry = int(time.time()) + 12
    for job in jobs:
        grant = issue(binary, state, http, job, 'initial', expiry)
        job.update(initial=grant, old_endpoint=Endpoint(mcp, grant['bearer']))
    return jobs, expiry


def expired(jobs, report):
    for operation in ('status', 'receipt-replay'):
        results = burst(lambda i: jobs[i]['old_endpoint'].request(encode(
            'connect_task_status' if operation == 'status' else 'connect_send',
            {} if operation == 'status' else jobs[i]['send'])), report)
        assert all(result[0] == 401 for result in results), 'Expired grant admitted work or receipt replay'


def boundary(jobs, expiry, report):
    with report.phase('original-effects-and-continuous-load-through-expiry'):
        results = burst(lambda i: jobs[i]['old_endpoint'].request(encode('connect_send', jobs[i]['send'])), report)
        for job, response in zip(jobs, results):
            receipt = success(response)['structuredContent']
            assert receipt['data']['status'] == 'completed' and receipt['data']['task_id'] == job['task']['id']
            accepted = receipt['data']['outcome']
            assert accepted['kind'] == 'accepted'
            assert accepted['data']['conversation_id'] == job['task']['conversation_id']
            assert accepted['data']['request_id'] == job['send']['request_id']
            job['receipt'] = receipt
        observations = {200: 0, 401: 0}
        while time.time() < expiry + .05:
            results = burst(lambda i: jobs[i]['old_endpoint'].request(encode('connect_task_status', {})), report)
            for job, response in zip(jobs, results):
                assert response[0] in observations, response[0]
                observations[response[0]] += 1
                if response[0] == 200:
                    assert success(response)['structuredContent']['data']['id'] == job['task']['id']
            time.sleep(.05)
        expired(jobs, report)
        assert observations[200] >= ADMISSIONS
        report.checkpoint(original_effects=32, expiry_load_responses=observations,
                          expired_status_and_receipt_denied=64)


def renew(binary, state, http, mcp, jobs, report):
    with report.phase('explicit-renewal-and-original-receipt-replay'):
        # Resume of a saved export is not renewal and cannot revive its bearer.
        path = jobs[0]['directory'] / 'initial-grant.json'
        before = hashlib.sha256(path.read_bytes()).hexdigest()
        result = run('task-access', 'resume', '--config', state / 'operator.toml', '--http-address', http,
                     '--grant-file', path, binary=binary)
        assert json.loads(result.stdout)['status'] == 'saved-not-refreshed'
        assert hashlib.sha256(path.read_bytes()).hexdigest() == before
        for job in jobs:
            grant = issue(binary, state, http, job, 'renewed', int(time.time()) + 120)
            assert grant['grant']['id'] != job['initial']['grant']['id']
            assert grant['grant']['task_id'] == job['task']['id']
            assert grant['grant']['binding'] == job['initial']['grant']['binding']
            assert grant['grant']['operations'] == job['initial']['grant']['operations']
            job.update(renewed=grant, endpoint=Endpoint(mcp, grant['bearer']))
        replay(jobs, report)
        expired(jobs, report)
        report.checkpoint(explicit_renewals=32, original_receipts_preserved=32,
                          saved_export_not_refreshed=True, old_grants_still_denied=True)


def replay(jobs, report):
    results = burst(lambda i: jobs[i]['endpoint'].request(encode('connect_send', jobs[i]['send'])), report)
    assert all(success(response)['structuredContent'] == job['receipt'] for job, response in zip(jobs, results))


def verify(owner, jobs, cancelled=False):
    for job in jobs:
        cid = job['task']['conversation_id']
        owner.negotiate_task(job['task']['id'])
        record = owner.task({'action': 'get', 'task_id': job['task']['id']})['data']
        assert record['id'] == job['task']['id'] and record['conversation_id'] == cid
        assert record['scope'] == job['task']['scope']
        assert record['owner_id'] == job['task']['owner_id'] and record['expires_at'] == job['task']['expires_at']
        expected_cancelled = cancelled and job['index'] % 2 == 0
        assert record['state'] == 'cancelled' if expected_cancelled else record['state'] in ('ready', 'running', 'waiting')
        messages = owner.history(cid)['messages']
        assert len(messages) == 1 and messages[0]['body'] == job['send']['body']
        assert messages[0]['id'] == job['receipt']['data']['outcome']['data']['message_id']


def cancel_half(jobs, report):
    with report.phase('autonomous-own-task-cancel-and-sibling-isolation'):
        def action(index):
            job = jobs[index]
            if index % 2 == 0:
                return job['endpoint'].request(encode('connect_task_cancel', job['cancel']))
            return job['endpoint'].request(encode('connect_send', job['send']))
        for job, response in zip(jobs, burst(action, report)):
            data = success(response)['structuredContent']
            if job['index'] % 2 == 0:
                assert data['data']['id'] == job['task']['id'] and data['data']['state'] == 'cancelled'
            else:
                assert data == job['receipt']
        cancellation_boundary(jobs, report)
        report.checkpoint(autonomous_cancellations=16, independent_survivors=16)


def cancellation_boundary(jobs, report):
    results = burst(lambda i: jobs[i]['endpoint'].request(encode('connect_send', jobs[i]['send'])), report)
    for job, response in zip(jobs, results):
        if job['index'] % 2 == 0:
            assert response[0] == 401, 'Cancelled task replay admitted'
        else:
            assert success(response)['structuredContent'] == job['receipt']
