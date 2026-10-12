"""Retained references are not permission to return expired bytes or rewrite tasks."""
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.gate4_phases import phases
from lib.retention_profile import classify_task_status, forbidden, expired_files, terminal_records


def reply(value):
    return 200, json.dumps({'jsonrpc': '2.0', 'id': 1, 'result': value}).encode()


class RetentionVerifierTests(unittest.TestCase):
    def test_expiry_crossing_distinguishes_all_three_authority_checks(self):
        responses = [
            ((401, b''), 'http_denied'),
            (reply({'isError': True, 'structuredContent': {'code': 'forbidden'}}), 'tool_denied'),
            ((200, json.dumps({'jsonrpc': '2.0', 'id': 1, 'error': {
                'code': -32600, 'message': 'Task authority is no longer current'}}).encode()), 'rpc_denied'),
        ]
        for response, expected in responses:
            self.assertEqual(classify_task_status((response, 99.9, 100.1), 'task', 100), expected)
            with self.assertRaisesRegex(AssertionError, 'before its configured expiry'):
                classify_task_status((response, 99, 99.9), 'task', 100)

    def test_success_must_have_started_before_expiry_and_match_the_task(self):
        response = reply({'structuredContent': {'data': {'id': 'task'}}})
        self.assertEqual(classify_task_status((response, 99.9, 100.1), 'task', 100), 'active')
        with self.assertRaisesRegex(AssertionError, 'started after expiry'):
            classify_task_status((response, 100, 100.1), 'task', 100)
        with self.assertRaises(AssertionError):
            classify_task_status((response, 99.9, 100.1), 'another-task', 100)

    def test_unrelated_errors_cannot_count_as_expiry_denials(self):
        for response in [(503, b''),
                         reply({'isError': True, 'structuredContent': {'code': 'transport-uncertain'}}),
                         (200, json.dumps({'jsonrpc': '2.0', 'id': 1, 'error': {
                             'code': -32600, 'message': 'unrelated failure'}}).encode())]:
            with self.assertRaises(AssertionError):
                classify_task_status((response, 99.9, 100.1), 'task', 100)

    def test_expired_read_requires_a_business_denial_not_transport_failure(self):
        denial = reply({'isError': True, 'structuredContent': {'code': 'forbidden'}})
        forbidden(denial)
        for bad in [(503, b''), reply({'isError': False, 'structuredContent': {'code': 'forbidden'}}),
                    reply({'isError': True, 'structuredContent': {'code': 'transport-uncertain'}})]:
            with self.assertRaises(AssertionError):
                forbidden(bad)

    def test_expired_reference_must_remain_without_body(self):
        jobs = [{'artifact': {'id': 'artifact'}, 'message': {'message_id': 'message'}, 'bytes': b'private'}] * 32
        denial = reply({'isError': True, 'structuredContent': {'code': 'forbidden'}})
        sources = [{'source_id': 'artifact', 'state': 'expired', 'body': None, 'provenance': 'bytes-require-artifact-get'},
                   {'source_id': 'message', 'state': 'available'}]
        for altered in (sources, sources[1:], [{**sources[0], 'body': 'old content'}, sources[1]]):
            recall = reply({'isError': False, 'structuredContent': {'data': {'sources': altered}}})
            with patch('lib.retention_profile.burst', side_effect=[[denial] * 32, [recall] * 32]):
                if altered == sources:
                    expired_files(jobs, Mock())
                else:
                    with self.assertRaises(AssertionError):
                        expired_files(jobs, Mock())

    def test_expired_record_and_message_identity_must_survive_restart(self):
        owner = Mock()
        record = {'id': 'task', 'state': 'expired', 'conversation_id': 'cid', 'scope': {}, 'expires_at': 42, 'revision': 1}
        owner.task.return_value = {'data': record}
        owner.history.return_value = {'messages': [{'id': 'message'}]}
        job = {'task': record.copy(), 'message': {'message_id': 'message'}}
        terminal_records(owner, [job])
        owner.task.return_value = {'data': {**record, 'revision': 2}}
        with self.assertRaisesRegex(AssertionError, 'changed after restart'):
            terminal_records(owner, [job])

    def test_retention_follows_normal_build_in_all_modes(self):
        for source_only in (True, False):
            names = [phase[0] for phase in phases(source_only)]
            self.assertEqual(names.count('task-artifact-retention'), 1)
            self.assertLess(names.index('current-binaries'), names.index('task-artifact-retention'))


if __name__ == '__main__':
    unittest.main()
