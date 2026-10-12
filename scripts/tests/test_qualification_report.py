"""Missing installed-host behavior must fail even if Cargo itself succeeds."""
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from lib.qualification_report import check_test_results


class HostCoverageTests(unittest.TestCase):
    def check_log(self, text):
        with tempfile.TemporaryDirectory() as directory:
            directory = pathlib.Path(directory)
            (directory / 'host.log').write_text(text)
            result = {'name': 'installed-host', 'status': 'passed', 'log': 'host.log'}
            check_test_results(result, directory)
            return result

    def test_all_runtime_artifacts_include_packaged_launcher_check(self):
        summaries = ''.join(f'test result: ok. {count} passed; 0 failed; 0 ignored;\n'
                            for count in (1, 3, 1, 1, 1))
        result = self.check_log(summaries)
        self.assertEqual(result['rust_tests'], {'passed': 7, 'failed': 0, 'ignored': 0})

    def test_older_six_check_set_cannot_pass_new_runner(self):
        with self.assertRaises(ValueError):
            self.check_log('test result: ok. 6 passed; 0 failed; 0 ignored;\n')

    def test_ignored_packaged_check_cannot_count_as_execution(self):
        with self.assertRaises(ValueError):
            self.check_log('test result: ok. 6 passed; 0 failed; 1 ignored;\n')

    def test_missing_summary_cannot_attest_host_execution(self):
        with self.assertRaises(ValueError):
            self.check_log('Finished compiling installed-host tests\n')


class IncomingCoverageTests(unittest.TestCase):
    def check_log(self, name, body):
        with tempfile.TemporaryDirectory() as directory:
            directory = pathlib.Path(directory)
            (directory / 'incoming.log').write_text(body)
            result = {'name': name, 'status': 'passed', 'log': 'incoming.log'}
            check_test_results(result, directory)
            return result

    def test_each_required_example_phase_counts_actual_tests(self):
        for name, count in [('agent-incoming-unit', 15), ('agent-incoming-public', 1),
                            ('agent-incoming-host', 1), ('voice-rehearsal-unit', 10),
                            ('chat-rehearsal-unit', 18)]:
            with self.subTest(name=name):
                result = self.check_log(name, f'Ran {count} tests in 1.0s\n\nOK\n')
                self.assertEqual(result['python_tests']['passed'], count)

    def test_missing_wrong_count_and_skipped_runs_cannot_pass(self):
        for body in ['no tests', 'Ran 0 tests in 0.0s\n\nOK\n', 'Ran 1 test in 0.0s\n\nOK (skipped=1)\n']:
            with self.subTest(body=body), self.assertRaises(ValueError):
                self.check_log('agent-incoming-host', body)

    def test_host_is_only_omitted_for_explicit_source_only_scope(self):
        from lib.gate4_phases import phases
        full = [name for name, _, _ in phases()]
        partial = [name for name, _, _ in phases(True)]
        self.assertIn('agent-incoming-unit', partial)
        self.assertIn('voice-rehearsal-unit', partial)
        self.assertIn('chat-rehearsal-unit', partial)
        self.assertIn('agent-incoming-public', partial)
        self.assertNotIn('agent-incoming-host', partial)
        self.assertIn('agent-incoming-host', full)
        self.assertLess(full.index('current-binaries'), full.index('agent-incoming-public'))
        self.assertLess(full.index('agent-incoming-public'), full.index('agent-incoming-host'))


if __name__ == '__main__':
    unittest.main()
