"""Reject false authority and receipt success in the grant lifecycle profile."""
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.gate4_phases import phases
from lib.grant_load_profile import expired, replay, cancellation_boundary, verify


def accepted(content):
    return 200, json.dumps({'jsonrpc': '2.0', 'id': 1, 'result': {
        'isError': False, 'structuredContent': content}}).encode()


class GrantLoadVerifierTests(unittest.TestCase):
    def setUp(self):
        self.receipt = {'data': {'outcome': {'data': {'message_id': 'original'}}}}
        self.jobs = [{'index': index, 'receipt': self.receipt} for index in range(32)]

    def test_expired_grant_cannot_return_an_old_successful_receipt(self):
        responses = [(401, b'')] * 32
        with patch('lib.grant_load_profile.burst', return_value=responses):
            expired(self.jobs, Mock())
        responses[17] = accepted(self.receipt)
        with patch('lib.grant_load_profile.burst', return_value=responses):
            with self.assertRaisesRegex(AssertionError, 'Expired grant'):
                expired(self.jobs, Mock())

    def test_new_receipt_is_not_an_original_replay(self):
        responses = [accepted(self.receipt)] * 32
        with patch('lib.grant_load_profile.burst', return_value=responses):
            replay(self.jobs, Mock())
        responses[-1] = accepted({'data': {'outcome': {'data': {'message_id': 'replacement'}}}})
        with patch('lib.grant_load_profile.burst', return_value=responses):
            with self.assertRaises(AssertionError):
                replay(self.jobs, Mock())

    def test_cancelled_success_and_survivor_denial_both_fail_isolation(self):
        baseline = [(401, b'') if index % 2 == 0 else accepted(self.receipt) for index in range(32)]
        with patch('lib.grant_load_profile.burst', return_value=baseline):
            cancellation_boundary(self.jobs, Mock())
        for index, value in ((0, accepted(self.receipt)), (1, (401, b''))):
            responses = baseline.copy()
            responses[index] = value
            with patch('lib.grant_load_profile.burst', return_value=responses):
                with self.assertRaises(AssertionError):
                    cancellation_boundary(self.jobs, Mock())

    def test_retained_history_requires_the_original_message_id(self):
        job = {**self.jobs[0], 'task': {'id': 'task', 'conversation_id': 'cid', 'scope': {}, 'owner_id': 'owner', 'expires_at': 100},
               'send': {'body': 'question'}}
        owner = Mock()
        owner.task.return_value = {'data': {'id': 'task', 'conversation_id': 'cid', 'scope': {}, 'owner_id': 'owner', 'expires_at': 100, 'state': 'running'}}
        owner.history.return_value = {'messages': [{'id': 'original', 'body': 'question'}]}
        verify(owner, [job])
        owner.history.return_value['messages'][0]['id'] = 'new-id'
        with self.assertRaises(AssertionError):
            verify(owner, [job])

    def test_lifecycle_profile_runs_after_normal_build_in_both_modes(self):
        for source_only in (True, False):
            names = [phase[0] for phase in phases(source_only)]
            self.assertEqual(names.count('grant-lifecycle-load'), 1)
            self.assertLess(names.index('current-binaries'), names.index('grant-lifecycle-load'))


if __name__ == '__main__':
    unittest.main()
