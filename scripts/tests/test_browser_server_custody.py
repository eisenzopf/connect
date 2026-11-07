"""A failed server shutdown must retain its verdict and release proven custody."""
import copy
import os
from pathlib import Path
import runpy
import signal
import subprocess
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
RUNNER = runpy.run_path(str(ROOT / 'scripts/rehearse-gate6-browser-matrix.py'))
record = RUNNER['record_server_custody']
from lib.vm_workload_lease import cleanup_evidence


def failed_report(server):
    return dict(format='connect/browser-matrix-component/1', passed=False,
                server_cleanup_failed=True, servers=[server],
                browser_owner_started=True, browser_owner_joined=True,
                packet_capture=dict(original_joined=True, observed=True))


class BrowserServerCustody(unittest.TestCase):
    def test_nonzero_exit_and_absent_original_group_release_only_custody(self):
        server = SimpleNamespace(pid=12345, returncode=1)
        receipt = dict(original_joined=False)
        with patch.object(os, 'killpg', side_effect=ProcessLookupError):
            record(server, receipt)
        self.assertEqual(receipt['original_exit_code'], 1)
        self.assertTrue(receipt['original_process_group_joined'])
        self.assertFalse(receipt['original_joined'])
        report = failed_report(receipt)
        original = copy.deepcopy(report)
        self.assertTrue(cleanup_evidence(report)['confirmed'])
        self.assertEqual(report, original)
        self.assertFalse(report['passed'])
        self.assertTrue(report['server_cleanup_failed'])

    def test_exit_code_without_absent_group_keeps_lease_blocked(self):
        receipt = dict(original_joined=True)
        with patch.object(os, 'killpg', return_value=None):
            record(SimpleNamespace(pid=12345, returncode=0), receipt)
        self.assertFalse(receipt['original_process_group_joined'])
        self.assertFalse(cleanup_evidence(failed_report(receipt))['confirmed'])

    def test_unwaited_owner_never_uses_group_absence_as_join(self):
        receipt = dict(original_joined=False)
        with patch.object(os, 'killpg') as observe:
            record(SimpleNamespace(pid=12345, returncode=None), receipt)
        observe.assert_not_called()
        self.assertFalse(cleanup_evidence(failed_report(receipt))['confirmed'])

    def test_unknown_group_observation_keeps_lease_blocked(self):
        receipt = dict(original_joined=False)
        with patch.object(os, 'killpg', side_effect=PermissionError):
            record(SimpleNamespace(pid=12345, returncode=1), receipt)
        self.assertEqual(receipt['process_group_observation_error'], 'PermissionError')
        self.assertFalse(cleanup_evidence(failed_report(receipt))['confirmed'])

    def test_incomplete_or_malformed_physical_receipt_keeps_lease_blocked(self):
        valid = dict(original_joined=False, original_process_group_joined=True,
                     original_pid=12345, original_exit_code=1)
        for key, value in [('original_pid', None), ('original_pid', True),
                           ('original_pid', 1), ('original_exit_code', None),
                           ('original_exit_code', False),
                           ('original_process_group_joined', None)]:
            with self.subTest(key=key, value=value):
                receipt = {**valid, key: value}
                self.assertFalse(cleanup_evidence(failed_report(receipt))['confirmed'])

    @unittest.skipUnless(os.name == 'posix', 'Original process-group observation requires POSIX')
    def test_actual_original_nonzero_process_group_is_observed_retired(self):
        child = subprocess.Popen([sys.executable, '-c', 'raise SystemExit(1)'],
                                 stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                 stderr=subprocess.DEVNULL, start_new_session=True)
        self.assertEqual(child.wait(timeout=5), 1)
        receipt = dict(original_joined=False)
        record(child, receipt)
        self.assertTrue(receipt['original_process_group_joined'])
        self.assertEqual(receipt['original_exit_code'], 1)
        self.assertTrue(cleanup_evidence(failed_report(receipt))['confirmed'])
        self.assertFalse(receipt['original_joined'])

    @unittest.skipUnless(os.name == 'posix', 'Original process-group cleanup requires POSIX')
    def test_actual_forced_group_retirement_retains_failed_graceful_verdict(self):
        child = subprocess.Popen([sys.executable, '-c',
                                  'import time; print("ready", flush=True); time.sleep(20)'],
                                 stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                 stderr=subprocess.DEVNULL, start_new_session=True, text=True)
        try:
            self.assertEqual(child.stdout.readline(), 'ready\n')
            os.killpg(child.pid, signal.SIGKILL)
            self.assertEqual(child.wait(timeout=5), -signal.SIGKILL)
            receipt = dict(original_joined=False, forced_cleanup=True)
            record(child, receipt)
            self.assertTrue(receipt['original_process_group_joined'])
            self.assertEqual(receipt['original_exit_code'], -signal.SIGKILL)
            self.assertTrue(cleanup_evidence(failed_report(receipt))['confirmed'])
            self.assertFalse(receipt['original_joined'])
            self.assertTrue(receipt['forced_cleanup'])
        finally:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait(timeout=5)
            child.stdout.close()


if __name__ == '__main__':
    unittest.main()
