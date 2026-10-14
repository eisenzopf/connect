"""Guard-only tests; synthetic process observations are never native media proof."""
import importlib.util
import json
import pathlib
import sys
import tempfile
import time
import types
import unittest
from unittest.mock import patch

SCRIPTS = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
spec = importlib.util.spec_from_file_location('native_cleanup_checker', SCRIPTS / 'check-native-video-cleanup.py')
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


class CleanupGuards(unittest.TestCase):
    def setUp(self):
        private = SCRIPTS.parent / '.local/gate5/checker-guard-fixtures'
        private.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=private)
        self.work = pathlib.Path(self.temp.name)
        self.peers = []
        for slot, reason in enumerate(('interrupt', 'peer-terminal')):
            report = self.work / f'peer-{slot}.json'
            report.write_text(json.dumps({'end_reason': reason, 'success': True, 'teardown_seconds': .1}))
            video = self.work / f'peer-{slot}.ivf'
            video.write_bytes(b'guard-only-not-a-real-keyframe')
            self.peers.append(types.SimpleNamespace(process=types.SimpleNamespace(poll=lambda: 0),
                report=report, video=video, exit_at=.2))

    def tearDown(self):
        self.temp.cleanup()

    def observe(self, **kwargs):
        with patch.object(checker, 'owned_rows', return_value=[]), patch.object(checker.time, 'monotonic', return_value=.3):
            return checker.exits(self.peers, 0, self.work, **kwargs)

    def edit(self, slot, **kwargs):
        report = json.loads(self.peers[slot].report.read_text())
        report.update(kwargs)
        self.peers[slot].report.write_text(json.dumps(report))

    def test_exact_interrupt_and_peer_terminal_acceptance(self):
        self.assertTrue(self.observe()['both_exited_within5seconds'])

    def test_process_exit_budget_cannot_borrow_small_reported_teardown(self):
        self.peers[1].exit_at = 5.01
        with self.assertRaises(TimeoutError):
            self.observe()

    def test_surviving_owned_descendant_fails_even_when_both_examples_exited(self):
        with patch.object(checker, 'owned_rows', return_value=[{'pid': 999, 'owned_group': 998}]), \
                patch.object(checker.time, 'monotonic', return_value=.3):
            with self.assertRaises(RuntimeError):
                checker.exits(self.peers, 0, self.work)

    def test_unreported_or_wrong_terminal_reason_fails(self):
        self.edit(1, end_reason='deadline')
        with self.assertRaises(RuntimeError):
            self.observe()

    def test_capture_failure_requires_process_failure_and_typed_failure_report(self):
        self.peers[0].process.poll = lambda: 1
        self.edit(0, end_reason='child-failure', success=False)
        self.assertTrue(self.observe(expected_failure=True)['both_exited_within5seconds'])
        self.peers[0].process.poll = lambda: 0
        with self.assertRaises(RuntimeError):
            self.observe(expected_failure=True)

    def test_capture_failure_does_not_accept_generic_error_or_failed_counterpart(self):
        self.peers[0].process.poll = lambda: 1
        self.edit(0, end_reason='error', success=False)
        with self.assertRaises(RuntimeError):
            self.observe(expected_failure=True)
        self.edit(0, end_reason='child-failure')
        self.edit(1, success=False)
        with self.assertRaises(RuntimeError):
            self.observe(expected_failure=True)

    def test_non_exited_child_remains_timeout_and_retains_partial_state(self):
        self.peers[1].exit_at = None
        self.peers[1].process.poll = lambda: None
        with patch.object(checker, 'owned_rows', return_value=[]), patch.object(checker.time, 'monotonic', return_value=6):
            with self.assertRaises(TimeoutError):
                checker.exits(self.peers, 0, self.work)
        partial = json.loads((self.work / 'exit-state.json').read_text())
        self.assertIsNone(partial['participants'][1]['exit_code'])

    def test_actual_exited_child_keeps_anchor_until_cleanup_and_is_never_signalled_after_reap(self):
        child = checker.OwnedChild([sys.executable, '-c', 'pass'])
        peer = types.SimpleNamespace(process=child, close_log=lambda: None)
        try:
            deadline = time.monotonic() + 5
            while child.poll() is None and time.monotonic() < deadline:
                time.sleep(.005)
            self.assertEqual(child.poll(), 0)
            self.assertFalse(child.reaped)
            self.assertIsNone(child.child.returncode, 'Popen has not reaped the reserved leader')
            receipt = child.receipt()
            self.assertEqual(receipt['owner_pid'], child.pid)
            self.assertNotEqual(receipt['native_pid'], child.pid)
            self.assertEqual(receipt['exit_code'], 0)
            with patch.object(checker.os, 'killpg') as group_signal:
                self.assertEqual(checker.cleanup_owned([peer]), [])
                group_signal.assert_not_called()
            self.assertTrue(child.reaped)
            with patch.object(checker.os, 'killpg') as group_signal, patch.object(checker.os, 'kill') as signal_pid:
                with self.assertRaises(RuntimeError):
                    child.kill_group()
                with self.assertRaises(ProcessLookupError):
                    child.send_signal(checker.signal.SIGINT)
                group_signal.assert_not_called()
                signal_pid.assert_not_called()
        finally:
            if not child.reaped:
                checker.cleanup_owned([peer])

    def test_waitid_unavailable_still_preserves_actual_negative_signal_status(self):
        ready = self.work / 'native-ready'
        program = "import pathlib,time;pathlib.Path(%r).write_text('ready');time.sleep(120)" % str(ready)
        with patch.object(checker.os, 'waitid', side_effect=AssertionError('waitid unavailable'), create=True):
            child = checker.OwnedChild([sys.executable, '-c', program])
            peer = types.SimpleNamespace(process=child, close_log=lambda: None)
            try:
                deadline = time.monotonic() + 5
                while not ready.exists() and time.monotonic() < deadline:
                    time.sleep(.005)
                self.assertTrue(ready.exists())
                child.send_signal(checker.signal.SIGTERM)
                while child.poll() is None and time.monotonic() < deadline:
                    time.sleep(.005)
                self.assertEqual(child.poll(), -checker.signal.SIGTERM)
                self.assertIsNone(child.child.returncode)
                self.assertEqual(checker.cleanup_owned([peer]), [])
                self.assertEqual(child.poll(), -checker.signal.SIGTERM)
            finally:
                if not child.reaped:
                    checker.cleanup_owned([peer])

    def test_interrupt_goes_to_native_child_and_retains_its_exact_exit_status(self):
        ready = self.work / 'interrupt-ready'
        program = ("import pathlib,signal,sys,time;signal.signal(signal.SIGINT,lambda *_:sys.exit(17));"
                   "pathlib.Path(%r).write_text('ready');time.sleep(120)") % str(ready)
        child = checker.OwnedChild([sys.executable, '-c', program])
        peer = types.SimpleNamespace(process=child, close_log=lambda: None)
        try:
            deadline = time.monotonic() + 5
            while not ready.exists() and time.monotonic() < deadline:
                time.sleep(.005)
            self.assertTrue(ready.exists())
            child.send_signal(checker.signal.SIGINT)
            while child.poll() is None and time.monotonic() < deadline:
                time.sleep(.005)
            self.assertEqual(child.poll(), 17)
            self.assertFalse(child.reaped)
            self.assertEqual(checker.cleanup_owned([peer]), [])
            self.assertEqual(child.poll(), 17)
        finally:
            if not child.reaped:
                checker.cleanup_owned([peer])

    def test_parent_pipe_eof_retires_native_and_its_descendant_without_an_unanchored_signal(self):
        ready = self.work / 'descendant-ready'
        program = ("import pathlib,subprocess,sys,time;"
                   "p=subprocess.Popen([sys.executable,'-c','import time;time.sleep(120)']);"
                   "pathlib.Path(%r).write_text(str(p.pid));time.sleep(120)") % str(ready)
        child = checker.OwnedChild([sys.executable, '-c', program])
        peer = types.SimpleNamespace(process=child, close_log=lambda: None)
        try:
            deadline = time.monotonic() + 5
            while not ready.exists() and time.monotonic() < deadline:
                time.sleep(.005)
            self.assertTrue(ready.exists())
            before = checker.owned_rows([peer])
            self.assertGreaterEqual(len(before), 2)
            self.assertNotIn(child.pid, [row['pid'] for row in before])
            child.child.stdin.close()  # Same EOF as abrupt parent disappearance.
            while checker.owned_rows([peer]) and time.monotonic() < deadline:
                time.sleep(.01)
            self.assertEqual(checker.owned_rows([peer]), [])
            child.wait(timeout=5)
            self.assertTrue(child.reaped)
            self.assertIsNone(child.code, 'Forced owner loss cannot invent native exit success')
            self.assertEqual(child.supervisor_exit_code, -checker.signal.SIGKILL)
        finally:
            if not child.reaped:
                checker.cleanup_owned([peer])

    def test_unknown_native_status_cannot_qualify_a_failure_report(self):
        self.peers[0].process.poll = lambda: None
        self.edit(0, end_reason='child-failure', success=False)
        with self.assertRaises(RuntimeError):
            self.observe(expected_failure=True)

    def test_observation_failure_cleans_anchored_groups_before_reaping_any_leader(self):
        sequence = []
        peers = []
        for pid in (41, 42):
            process = types.SimpleNamespace(pid=pid,
                kill_group=lambda pid=pid: sequence.append(('signal', pid)),
                wait=lambda timeout, pid=pid: sequence.append(('reap', pid)))
            peers.append(types.SimpleNamespace(process=process, close_log=lambda: None))
        with patch.object(checker, 'owned_rows', side_effect=OSError('synthetic ps failure')):
            recovery = checker.cleanup_owned(peers)
        self.assertEqual(sequence, [('signal', 41), ('signal', 42), ('reap', 41), ('reap', 42)])
        self.assertEqual(recovery[0], {'observation_error_class': 'OSError'})


if __name__ == '__main__':
    unittest.main()
