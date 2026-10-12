"""Real process failures, inherited-child cleanup and independent-worker isolation."""
import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from lib.qualification_process import run_phase
from lib.qualification_report import environment


class PhaseProcess(unittest.TestCase):
    def setUp(self):
        parent = ROOT / '.local/qualification-process-tests'
        parent.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=parent)
        self.directory = pathlib.Path(self.temporary.name)

    def tearDown(self):
        self.temporary.cleanup()

    def run_code(self, name, code, seconds=5):
        return run_phase(name, [sys.executable, '-c', code], seconds,
                         self.directory, ROOT, environment())

    def assert_terminated(self, pid):
        state = subprocess.run(['ps', '-o', 'stat=', '-p', str(pid)],
                               text=True, capture_output=True, timeout=3)
        self.assertTrue(state.returncode != 0 or state.stdout.strip().startswith('Z'), state.stdout)

    def test_success_preserves_command_output_and_exit(self):
        result = self.run_code('success', 'print("phase-result")')
        self.assertEqual(result['status'], 'passed')
        self.assertEqual(result['exit_code'], 0)
        self.assertEqual(result['leftover_group_members'], [])
        self.assertIn('phase-result', (self.directory / result['log']).read_text())
        self.assertEqual(result['group_cleanup'], 'signalled')

    def test_missing_executable_is_failed_evidence(self):
        result = run_phase('missing', [str(self.directory / 'missing')], 5,
                           self.directory, ROOT, environment())
        self.assertEqual(result['status'], 'failed')
        self.assertEqual(result['error'], 'FileNotFoundError')
        self.assertIsNone(result['exit_code'])
        self.assertEqual(result['group_cleanup'], 'signalled')

    def test_nonzero_is_not_a_pass(self):
        result = self.run_code('nonzero', 'raise SystemExit(7)')
        self.assertEqual(result['status'], 'failed')
        self.assertEqual(result['exit_code'], 7)

    def child_program(self, wait):
        pid_file = self.directory / 'descendant.pid'
        code = ('import pathlib, subprocess, sys, time\n'
                'child=subprocess.Popen([sys.executable,"-c","import time;time.sleep(120)"])\n'
                f'pathlib.Path({str(pid_file)!r}).write_text(str(child.pid))\n')
        if wait:
            code += 'time.sleep(120)\n'
        return code, pid_file

    def test_deadline_stops_inherited_child_without_stopping_another_worker(self):
        independent = subprocess.Popen([sys.executable, '-c', 'import time;time.sleep(120)'],
                                       start_new_session=True)
        try:
            code, pid_file = self.child_program(True)
            result = self.run_code('timeout', code, 1)
            self.assertEqual(result['status'], 'failed')
            self.assertEqual(result['error'], 'TimeoutError')
            self.assertLess(result['seconds'], 6)
            self.assert_terminated(int(pid_file.read_text()))
            self.assertIsNone(independent.poll())
        finally:
            independent.kill()
            independent.wait(timeout=5)

    def test_normal_exit_with_live_descendant_is_failed_and_cleaned(self):
        code, pid_file = self.child_program(False)
        result = self.run_code('leaked', code)
        self.assertEqual(result['status'], 'failed')
        self.assertEqual(result['error'], 'LeakedProcessGroup')
        self.assertIn(int(pid_file.read_text()), result['leftover_group_members'])
        self.assert_terminated(int(pid_file.read_text()))


if __name__ == '__main__':
    unittest.main()
