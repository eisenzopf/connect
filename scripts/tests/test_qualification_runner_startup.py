"""Keep startup failure evidence distinct from cleanup and observation failures."""
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.profile_metrics import ProfileReport
from lib.runner_profile import questions
from lib.runner_profile_worker import observe_startup, stop_all


class RunnerStartupTests(unittest.TestCase):
    def test_cleanup_preserves_original_timeout_and_attempts_every_worker(self):
        original = TimeoutError('question deadline')
        first, second = Mock(), Mock()
        first.stop.side_effect = AssertionError('unexpected signal exit')
        stop_all([first, second], primary_error=original)
        second.stop.assert_called_once_with()
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            report = ProfileReport(directory, {})
            report.finish('failed', directory, original)
            saved = json.loads((directory / 'result.json').read_text())
        self.assertEqual(saved['error']['type'], 'TimeoutError')
        self.assertEqual(saved['error']['detail'], 'question deadline')
        self.assertIn('unexpected signal exit', saved['error']['notes'][0])

    def test_question_timeout_captures_workers_before_cleanup(self):
        worker = Mock()
        snapshot = {'workers': [{'pid': 123, 'database_exists': False}]}
        with patch('lib.runner_profile.time.monotonic', side_effect=[0, 16]), patch(
                'lib.runner_profile.observe_startup', return_value=snapshot) as observe:
            with self.assertRaises(TimeoutError) as caught:
                questions(Mock(), [worker])
        observe.assert_called_once_with([worker])
        worker.stop.assert_not_called()
        self.assertEqual(caught.exception.worker_startup, snapshot)
        self.assertIn('1 workers', str(caught.exception))

    def test_observation_failure_does_not_replace_question_timeout(self):
        with patch('lib.runner_profile.time.monotonic', side_effect=[0, 16]), patch(
                'lib.runner_profile.observe_startup', side_effect=OSError('observation unavailable')):
            with self.assertRaises(TimeoutError) as caught:
                questions(Mock(), [Mock()])
        self.assertIn('OSError', caught.exception.__notes__[0])

    def test_startup_observation_reads_metadata_not_private_contents(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            log = directory / 'worker.log'
            log.write_text('private content')
            worker = SimpleNamespace(directory=directory, database=directory / 'absent.sqlite',
                                     log_path=log, process=Mock(pid=123))
            worker.process.poll.return_value = None
            with patch('lib.runner_profile_worker.subprocess.run', return_value=SimpleNamespace(
                    stdout='123 1 S 00:16 0:00.01 1024\n', returncode=0)) as run:
                result = observe_startup([worker])
        self.assertFalse(result['workers'][0]['database_exists'])
        self.assertEqual(result['workers'][0]['log_bytes'], len('private content'))
        self.assertNotIn('private content', json.dumps(result))
        self.assertEqual(run.call_args.kwargs['timeout'], 3)
        self.assertNotIn('args', run.call_args.args[0])


if __name__ == '__main__':
    unittest.main()
