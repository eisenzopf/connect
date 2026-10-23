"""Measure real reply admission, journal paging and the maximum native task wait."""
import json
import time

from connect_sdk import ConnectError

from .task_capacity import denied, record

INBOX = 1000


def inbox(profile, task_id, after=None, limit=100, wait_ms=0, action='inbox'):
    result = profile.command({'action': action, 'task_id': task_id, 'after': after,
                              'limit': limit, 'wait_ms': wait_ms})
    assert result['kind'] == 'inbox'
    return result['data']


def drain(profile, task_id):
    cursor, replies, maximum, peak = None, [], 0, 0
    for _ in range(100):
        page = inbox(profile, task_id, cursor)
        events = page['events']
        maximum = max(maximum, len(events))
        size = sum(len(json.dumps(e, ensure_ascii=False, separators=(',', ':')).encode())
                   for e in events)
        peak = max(peak, size)
        assert len(events) <= 100 and size <= 32768, 'Task page exceeded frozen bounds'
        replies.extend(e['data']['data'] for e in events if e['data']['kind'] == 'reply')
        if not events:
            return page['next'], replies, maximum, peak
        assert page['next'] != cursor, 'Journal page did not advance'
        cursor = page['next']
    raise AssertionError('Task journal did not exhaust within its known fixture size')


def qualify(profile, peer):
    task_id = profile.tasks[0]['id']
    cid = profile.definition['conversation_id']
    question = profile.command({
        'action': 'invoke', 'request_id': 'profile-question', 'task_id': task_id,
        'step_id': 'profile-question', 'reply_to': None, 'wait_for_reply': False,
        'operation': {'op': 'send', 'request_id': 'profile-question',
                      'conversation_id': cid, "message": {'to': 'all', 'content_type': 'text/plain', 'body': 'Q' * 16384, 'body_encoding': 'utf8', 'label': 'connect-messages', 'attachments': [], 'in_reply_to_msg': None, 'msg_id': None}},
    })
    assert question['kind'] == 'tool' and question['data']['status'] == 'completed'
    message_id = question['data']['outcome']['data']['message_id']
    peer.negotiate_tasks('connect:operator@local', cid)

    def reply(index):
        return {'action': 'reply', 'request_id': f'profile-reply-{index}',
                'conversation_id': cid, 'reply_to': message_id, 'body': f'Answer {index}'}

    def answer(index):
        command = reply(index)
        profile.report.pending = command
        started = time.monotonic()
        try:
            result = peer.task(command)
        except ConnectError as error:
            if error.code not in ('transport-uncertain', 'protocol'):
                profile.report.pending = None
            raise
        else:
            profile.report.pending = None
            return result
        finally:
            profile.report.latency(time.monotonic() - started)

    first = answer(0)
    assert first['kind'] == 'reply_accepted'
    for index in range(1, INBOX):
        assert answer(index)['data']['reply']['task_id'] == task_id
    cursor, replies, maximum, peak = drain(profile, task_id)
    assert len(replies) == INBOX and len({r['event_id'] for r in replies}) == INBOX
    denied(lambda: answer(INBOX))
    assert answer(0) == first, 'Matching reply receipt changed at capacity'
    assert not inbox(profile, task_id, cursor)['events'], 'Rejected/duplicate reply changed journal'
    current = record(profile.command({'action': 'get', 'task_id': task_id}))
    profile.command({'action': 'consume_reply', 'task_id': task_id,
                     'request_id': 'profile-consume-first',
                     'event_id': first['data']['reply']['event_id'],
                     'expected_revision': current['revision']})
    assert answer(INBOX)['kind'] == 'reply_accepted'
    cursor, replies, _, _ = drain(profile, task_id)
    # Consumption remains in history; it does not erase or recreate the reply.
    assert len({r['event_id'] for r in replies}) == INBOX + 1
    denied(lambda: inbox(profile, task_id, cursor, limit=101), 'invalid-request')
    started = time.monotonic()
    page = inbox(profile, task_id, cursor, wait_ms=10000, action='wait')
    elapsed = time.monotonic() - started
    assert not page['events'] and page['next'] == cursor
    assert 9.5 <= elapsed <= 12, 'Empty task wait did not honor its bounded wait interval'
    profile.report.checkpoint(
        unconsumed_reply_ceiling=INBOX, excess_reply_denied_without_event=True,
        consumed_slot_reused=True, inbox_page_max_events=maximum,
        inbox_page_peak_projection_bytes=peak, native_wait_seconds=round(elapsed, 3),
        native_wait_requested_ms=10000,
    )
