"""Public task continuation after consistent backup/restore, with current auth."""
import time

from connect_sdk import ConnectError, NativeClient
from .local_process import Server
from .recovery_state import preserved, snapshot
from .recovery_upgrade import backup, restore, verify


def record(outcome):
    assert outcome['kind'] == 'record'
    return outcome['data']


def invoke(task_id, cid, request_id, body, waiting=False):
    return {'action': 'invoke', 'task_id': task_id, 'request_id': request_id,
            'step_id': request_id, 'reply_to': None, 'wait_for_reply': waiting,
            'operation': {'op': 'send', 'request_id': request_id,
                          'conversation_id': cid, 'body': body}}


def inbox(client, task_id):
    return client.task({'action': 'inbox', 'task_id': task_id, 'after': None,
                        'limit': 100, 'wait_ms': 0})


def qualify(binary, work, state, saved, schema):
    cid = saved['cid']
    task_backup = work / 'task-backup'
    with Server(state, binary=binary, log_path=work / 'tasks-source.log'):
        with NativeClient(state / 'operator.toml', executable=str(binary)) as client:
            client.negotiate_tasks('connect:agent@local', cid)
            task = record(client.task({'action': 'create', 'request_id': 'recover-task',
                'definition': {'conversation_id': cid, 'initiating_event_id': None,
                               'expires_at': int(time.time()) + 1800,
                               'scope': {'operations': ['send', 'history', 'inbox'],
                                         'recipients': [saved['peer']]}}}))
            question = invoke(task['id'], cid, 'recover-question', 'Approve the blue draft?', True)
            receipt = client.task(question)
            assert receipt['kind'] == 'tool' and receipt['data']['status'] == 'pending', receipt
            questions = [m for m in client.history(cid)['messages']
                         if m['body'] == question['operation']['body']]
            assert len(questions) == 1
            message_id = questions[0]['id']
            waiting = record(client.task({'action': 'get', 'task_id': task['id']}))
            assert waiting['state'] == 'waiting'
            page = inbox(client, task['id'])
            backup(binary, state, task_backup)
            client.send('after-task-snapshot', cid, 'This fact belongs only to the original installation.')
    source = snapshot(state / 'state.sqlite3')
    saved_rows = snapshot(task_backup / 'state.sqlite3')
    restored = work / 'task-restored'
    restore(binary, task_backup, restored)
    preserved(restored / 'state.sqlite3', saved_rows, schema)
    with Server(restored, binary=binary, log_path=work / 'tasks-restored.log'):
        with NativeClient(restored / 'operator.toml', executable=str(binary)) as client:
            assert client.negotiate_task(task['id']) == waiting
            assert client.task(question) == receipt
            assert inbox(client, task['id']) == page
            verify(client, saved)
            assert len(client.history(cid)['messages']) == 2
            with NativeClient(restored / 'agent.toml', executable=str(binary)) as human:
                human.negotiate_tasks('connect:operator@local', cid)
                answer = {'action': 'reply', 'request_id': 'restored-answer',
                          'conversation_id': cid, 'reply_to': message_id,
                          'body': 'Approved blue draft, reference recovery-731.'}
                reply = human.task(answer)
                assert human.task(answer) == reply
                assert reply['data']['reply']['task_id'] == task['id']
            events = inbox(client, task['id'])['data']['events']
            replies = [e['data']['data'] for e in events if e['data']['kind'] == 'reply']
            assert len(replies) == 1
            recall = client.task({'action': 'recall', 'task_id': task['id'],
                                  'after': None, 'limit': 100})
            provenance = 'conversation-journal:' + replies[0]['event_id']
            sources = [s for s in recall['data']['sources']
                       if s['provenance'].split(';')[0] == provenance]
            assert len(sources) == 1 and sources[0]['state'] == 'available'
            follow = invoke(task['id'], cid, 'restored-follow-up',
                            'Recorded review: ' + sources[0]['body'])
            follow['reply_to'] = replies[0]['event_id']
            result = client.task(follow)
            assert result['data']['status'] == 'completed'
            assert client.task(follow) == result
            history = client.history(cid)['messages']
            assert len(history) == 4
            assert sum(m['body'] == follow['operation']['body'] for m in history) == 1
            client.task({'action': 'cancel', 'task_id': task['id'],
                         'request_id': 'restored-cancel', 'reason': 'Recovery authority check'})
            before = snapshot(restored / 'state.sqlite3')
            try:
                client.task(follow)
            except ConnectError as error:
                assert error.code == 'forbidden', error.code
            else:
                raise AssertionError('Cancelled task replay borrowed historical authority')
            preserved(restored / 'state.sqlite3', before, schema)
            assert record(client.task({'action': 'get', 'task_id': task['id']}))['state'] == 'cancelled'
    preserved(state / 'state.sqlite3', source, schema)
    preserved(task_backup / 'state.sqlite3', saved_rows, schema)
    return {'waiting_task_and_cursor_restored': True, 'original_question_receipt_replayed': True,
            'bound_reply_after_restore': True, 'one_source_derived_follow_up': True,
            'current_cancellation_denies_replay': True, 'backup_and_original_preserved': True}
