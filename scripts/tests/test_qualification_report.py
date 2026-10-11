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
                            for count in (1, 3, 1, 1))
        result = self.check_log(summaries)
        self.assertEqual(result['rust_tests'], {'passed': 6, 'failed': 0, 'ignored': 0})

    def test_older_five_check_set_cannot_pass_new_runner(self):
        with self.assertRaises(ValueError):
            self.check_log('test result: ok. 5 passed; 0 failed; 0 ignored;\n')

    def test_ignored_packaged_check_cannot_count_as_execution(self):
        with self.assertRaises(ValueError):
            self.check_log('test result: ok. 5 passed; 0 failed; 1 ignored;\n')

    def test_missing_summary_cannot_attest_host_execution(self):
        with self.assertRaises(ValueError):
            self.check_log('Finished compiling installed-host tests\n')


if __name__ == '__main__':
    unittest.main()
