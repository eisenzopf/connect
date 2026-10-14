"""Failure-path cleanup guards; synthetic workers are never behavior evidence."""
import importlib.util
import pathlib
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import Mock

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
spec = importlib.util.spec_from_file_location('connected_app_checker', ROOT / 'scripts/check-connected-app.py')
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


class ConnectedAppCleanupTests(unittest.TestCase):
    def fixture(self, directory):
        journey = checker.Journey.__new__(checker.Journey)
        source = pathlib.Path(directory) / 'selected-input'
        source.write_bytes(b'nonexecuted guard input')
        journey.inputs = [source]
        journey.processes = [SimpleNamespace(pid=pid, poll=lambda: 0) for pid in (1, 2, 3)]
        journey.handles = [Mock()]
        journey.report = {'status': 'passed', 'input_sha256': {str(source): checker.digest(source)}}
        return journey, source

    def test_one_failed_worker_does_not_skip_other_owned_cleanup(self):
        with tempfile.TemporaryDirectory() as directory:
            journey, _ = self.fixture(directory)
            journey.stop = Mock(side_effect=[AssertionError('unexpected exit'), None, None])
            journey.finish()
            self.assertEqual([call.args[0].pid for call in journey.stop.call_args_list], [1, 2, 3])
            self.assertEqual(journey.report['status'], 'failed')
            self.assertEqual(journey.report['cleanup_errors'], [{'pid': 1, 'error_class': 'AssertionError'}])
            self.assertEqual(journey.report['remaining_owned_processes'], [])
            journey.handles[0].close.assert_called_once()

    def test_input_changed_during_cleanup_cannot_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            journey, source = self.fixture(directory)
            journey.stop = Mock(side_effect=lambda _: source.write_bytes(b'changed during cleanup'))
            journey.finish()
            self.assertEqual(journey.report['status'], 'failed')
            self.assertFalse(journey.report['tested_inputs_stable'])

    def test_missing_input_after_cleanup_cannot_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            journey, source = self.fixture(directory)
            journey.stop = Mock(side_effect=lambda _: source.unlink(missing_ok=True))
            journey.finish()
            self.assertEqual(journey.report['status'], 'failed')
            self.assertFalse(journey.report['tested_inputs_stable'])


if __name__ == '__main__':
    unittest.main()
