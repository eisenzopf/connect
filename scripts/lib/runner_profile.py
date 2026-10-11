"""Real reference workers compete for leases, follow replies, and replay after restart."""
import json
import time
import tomllib

from .local_process import run
from .runner_profile_worker import Worker, settle, snapshot, stop_all

QUESTION = 'Please review draft-002 and reply to this question.'
ANSWER = 'Your draft response is recorded.'
COUNT = 16


def prepare(binary, example, state, directory, http, owner):
    peer_id = tomllib.loads((state / 'agent.toml').read_text())['identity_id']
    workers = []
    for index in range(COUNT + 1):
        cid = owner.create(f'runner-conversation-{index}', [peer_id])['id']
        owner.negotiate_tasks('connect:agent@local', cid)
        scope = {'operations': ['send', 'history', 'inbox', 'request_status'], 'recipients': [peer_id]}
        task = owner.task({'action': 'create', 'request_id': f'runner-task-{index}',
                           'definition': {'conversation_id': cid, 'scope': scope,
                                          'expires_at': int(time.time()) + 1800}})['data']
        path = directory / f'worker-{index}'
        path.mkdir(mode=0o700)
        intent = {'profile': 'connect-task-access/1', 'command': {
            'action': 'issue', 'request_id': f'runner-install-{index}', 'definition': {
                'task_id': task['id'], 'binding': {'kind': 'native', 'adapter': 'reference-runner',
                                                 'external_id': f'worker-{index}'},
                'operations': scope['operations'], 'expires_at': int(time.time()) + 300}}}
        (path / 'issue.json').write_text(json.dumps(intent))
        run('task-access', 'issue', '--config', state / 'operator.toml', '--http-address', http,
            '--input', path / 'issue.json', '--output', path / 'grant.json', binary=binary)
        run('task-access', 'export-native', '--config', state / 'operator.toml',
            '--grant-file', path / 'grant.json', '--output', path / 'invocation.toml', binary=binary)
        workers.append(Worker(example, state, path, task))
    return workers


def claim(owner, worker):
    return owner.task({'action': 'claim_get', 'task_id': worker.task['id'], 'step_id': 'draft-question'})['data']


def messages(human, worker):
    return human.history(worker.task['conversation_id'])['messages']


def questions(human, workers):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        for worker in workers:
            worker.alive()
        histories = [messages(human, worker) for worker in workers]
        if all(len(history) == 1 and history[0]['body'] == QUESTION for history in histories):
            for worker, history in zip(workers, histories):
                worker.question_id = history[0]['id']
            return [history[0] for history in histories]
        time.sleep(.02)
    raise TimeoutError('Sixteen workers did not produce their questions within 15 seconds')


def reply(human, worker, question):
    human.negotiate_tasks('connect:operator@local', worker.task['conversation_id'])
    result = human.task({'action': 'reply', 'request_id': 'profile-answer-' + worker.task['id'],
                         'conversation_id': worker.task['conversation_id'],
                         'reply_to': question['id'], 'body': 'Approved draft-002.'})
    assert result['kind'] == 'reply_accepted'
    assert result['data']['reply']['task_id'] == worker.task['id']
    return result


def verify(human, workers):
    for worker in workers:
        history = messages(human, worker)
        assert [message['body'] for message in history] == [QUESTION, 'Approved draft-002.', ANSWER]
        assert history[0]['id'] == worker.question_id, 'Restart replaced the original question'


def cold(owner, human, workers, environment, report):
    cohort = workers[:COUNT]
    with report.phase('sixteen-workers-and-seventeenth-denial'):
        start = time.monotonic()
        for worker in cohort:
            worker.start(environment)
        questions(human, cohort)
        leases = [claim(owner, worker) for worker in cohort]
        observed = snapshot(cohort)
        assert all(lease is not None and lease['expires_at'] > int(time.time()) for lease in leases)
        report.latency(time.monotonic() - start)
        overflow = workers[-1]
        overflow.start(environment)
        assert overflow.process.wait(timeout=10) == 1, 'Overflow did not fail explicitly'
        assert 'Concurrent step lease capacity exceeded' in overflow.log_path.read_text()
        overflow.stop(expected=1)
        assert all(lease['expires_at'] > int(time.time()) for lease in leases), 'Leases expired during admission proof'
        assert messages(human, overflow) == [] and claim(owner, overflow) is None
        pending = overflow.inspect()
        assert pending['pending_commands'] == 0 and pending['rejected_commands'] == 1
        assert pending['business_effects'] == 0 and pending['accepted_commands'] == 0
        original = overflow.claim_intent()
        report.checkpoint(simultaneous_workers=observed, overflow_denied_without_effect=True,
                          cold_cohort_rss_bytes=sum(row['rss_bytes'] for row in observed))
    with report.phase('released-slot-original-intent-and-sixteen-replies'):
        workers[0].stop()
        owner.task({'action': 'release_claim', 'task_id': workers[0].task['id'],
                    'step_id': 'draft-question', 'worker_id': leases[0]['worker_id'],
                    'expected_revision': leases[0]['revision'], 'request_id': 'profile-release-first'})
        overflow.start(environment)
        questions(human, [overflow])
        assert overflow.claim_intent() == original, 'Capacity retry changed original claim intent'
        assert claim(owner, overflow)['worker_id'] == json.loads(original)['worker_id']
        cohort = workers[1:]
        observed = snapshot(cohort)
        assert all(claim(owner, worker)['expires_at'] > int(time.time()) for worker in cohort)
        for worker, question in zip(cohort, questions(human, cohort)):
            worker.reply = reply(human, worker, question)
        settle(cohort, 1)
        verify(human, cohort)
        report.checkpoint(released_slot_reused=True, original_overflow_intent_preserved=True,
                          replacement_cohort_rss_bytes=sum(row['rss_bytes'] for row in observed),
                          simultaneous_follow_through=COUNT)
    stop_all(workers)


def warm(human, workers, environment, report):
    # Original 16 workers resume their own states; worker zero had no reply yet.
    cohort = workers[:COUNT]
    with report.phase('sixteen-workers-after-server-and-app-restart'):
        for worker in cohort:
            worker.start(environment)
        question = messages(human, workers[0])[0]
        workers[0].reply = reply(human, workers[0], question)
        for worker in cohort[1:]:
            assert reply(human, worker, messages(human, worker)[0]) == worker.reply
        settle(cohort, 1)
        snapshot(cohort)
        verify(human, workers)
        report.checkpoint(restart_business_effects=[worker.inspect()['business_effects'] for worker in workers],
                          duplicate_replies_returned_original_receipts=True, question_and_follow_up_once=True)
    stop_all(workers)
