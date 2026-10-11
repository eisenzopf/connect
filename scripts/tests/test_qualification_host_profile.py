"""Reject false host success and leaked groups in the concurrency verifier."""
import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.host_profile_observation import exited, verify_turn


class HostProfileObservationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        host = root / 'host'
        host.mkdir()
        (host / 'host-state.json').write_text(json.dumps({'run_id': 'run', 'attempts': 1}))
        self.path = host / 'host-run-1.jsonl'
        self.job = {'directory': root, 'cid': 'conversation', 'task': {'id': 'task', 'initiating_event_id': 'event'},
                    'message': {'message_id': 'source'}, 'body': 'original question'}
        results = [
            {'data': {'id': 'task', 'conversation_id': 'conversation'}},
            {'data': {'sources': [{'source_id': 'source', 'state': 'available', 'body': '{"body":"original question"}',
                                   'provenance': 'conversation-journal:event;timestamp-not-recorded'}]}},
            {'kind': 'tool', 'data': {'status': 'completed', 'task_id': 'task', 'outcome': {
                'kind': 'accepted', 'data': {'request_id': 'incoming-answer', 'conversation_id': 'conversation', 'message_id': 'answer'}}}},
        ]
        self.events = [{'type': 'item.completed', 'item': {'type': 'mcp_tool_call', 'server': 'connect_task',
                       'status': 'completed', 'error': None, 'tool': name, 'result': {'structured_content': result}}}
                       for name, result in zip(('connect_task_status', 'connect_recall', 'connect_send'), results)]
        self.write()

    def write(self):
        self.path.write_text('\n'.join(map(json.dumps, self.events)) + '\n')

    def test_matching_receipt_passes_both_turns(self):
        verify_turn(self.job, False)
        verify_turn(self.job, True)

    def test_completed_prose_without_tool_results_fails(self):
        self.events = [{'type': 'item.completed', 'item': {'type': 'agent_message', 'text': 'Done'}}]
        self.write()
        with self.assertRaises(AssertionError):
            verify_turn(self.job, False)

    def test_failed_tool_cannot_be_masked_by_completed_turn(self):
        self.events[-1]['item']['result']['is_error'] = True
        self.write()
        with self.assertRaises(AssertionError):
            verify_turn(self.job, False)

    def test_wrong_task_or_source_is_rejected(self):
        original = copy.deepcopy(self.events)
        for index, field, value in ((0, 'id', 'other-task'), (2, 'task_id', 'other-task')):
            self.events = copy.deepcopy(original)
            self.events[index]['item']['result']['structured_content']['data'][field] = value
            self.write()
            with self.assertRaises(AssertionError):
                verify_turn(self.job, False)
        self.events = original
        self.events[1]['item']['result']['structured_content']['data']['sources'][0]['provenance'] = 'conversation-journal:other-event'
        self.write()
        with self.assertRaises(AssertionError):
            verify_turn(self.job, False)

    def test_warm_turn_requires_the_original_effect(self):
        verify_turn(self.job, False)
        self.events[-1]['item']['result']['structured_content']['data']['outcome']['data']['message_id'] = 'second-answer'
        self.write()
        with self.assertRaises(AssertionError):
            verify_turn(self.job, True)

    def test_a_live_owned_group_fails_observation(self):
        child = subprocess.Popen([sys.executable, '-c', 'import sys; sys.stdin.read()'],
                                 stdin=subprocess.PIPE, start_new_session=True)
        try:
            with self.assertRaises(AssertionError):
                exited([{'process_group': child.pid}])
        finally:
            child.stdin.close()
            child.wait(timeout=5)
        exited([{'process_group': child.pid}])


if __name__ == '__main__':
    unittest.main()
