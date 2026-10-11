"""Reject premature or duplicated completion in the real runner profile."""
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import Mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.gate4_phases import phases
from lib.runner_profile import QUESTION, ANSWER, verify
from lib.runner_profile_worker import settle, stop_all


class RunnerProfileObservationTests(unittest.TestCase):
    def test_history_requires_exactly_one_question_reply_and_follow_up(self):
        worker = SimpleNamespace(task={'conversation_id': 'conversation'}, question_id='question-id')
        human = Mock()
        expected = [QUESTION, 'Approved draft-002.', ANSWER]
        for bodies in (expected, expected + [ANSWER], expected[:2], [ANSWER, expected[1], QUESTION]):
            human.history.return_value = {'messages': [{'body': body, 'id': 'question-id'} for body in bodies]}
            if bodies == expected:
                verify(human, [worker])
            else:
                with self.assertRaises(AssertionError):
                    verify(human, [worker])

    def test_duplicate_business_effect_fails_without_waiting(self):
        worker = Mock()
        worker.inspect.return_value = {'business_effects': 2, 'pending_commands': 0, 'page_pending': False}
        with self.assertRaisesRegex(AssertionError, 'Duplicate business action'):
            settle([worker], 1)

    def test_dead_worker_cannot_pass_with_saved_success_statistics(self):
        worker = Mock()
        worker.alive.side_effect = AssertionError('process exited')
        worker.inspect.return_value = {'business_effects': 1, 'pending_commands': 0, 'page_pending': False}
        with self.assertRaisesRegex(AssertionError, 'process exited'):
            settle([worker], 1)
        worker.inspect.assert_not_called()

    def test_cleanup_attempts_every_worker_and_reports_failure(self):
        first, second = Mock(), Mock()
        first.stop.side_effect = AssertionError('unexpected exit')
        with self.assertRaisesRegex(AssertionError, 'unexpected exit'):
            stop_all([first, second])
        second.stop.assert_called_once_with()

    def test_runner_profile_follows_normal_server_and_example_build(self):
        for source_only in (True, False):
            steps = phases(source_only)
            names = [step[0] for step in steps]
            assert names.count('reference-runner-capacity') == 1
            assert names.index('current-binaries') < names.index('reference-runner-capacity')
            command = steps[names.index('current-binaries')][1]
            assert 'connect' in command and 'connect-example' in command and '--all-features' not in command


if __name__ == '__main__':
    unittest.main()
