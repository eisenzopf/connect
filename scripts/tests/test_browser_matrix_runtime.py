"""Browser pins share the approved platform contract; preserve original failures."""
import copy
import json
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
RUNNER = runpy.run_path(str(ROOT / 'scripts/rehearse-gate6-browser-matrix.py'))
runtime_spec = RUNNER['runtime_spec']
read_observation = RUNNER['read_browser_observation']


class BrowserMatrixRuntime(unittest.TestCase):
    def test_approved_mac_chrome157_and_linux_chrome156_use_client_platform_pins(self):
        contract = json.loads((ROOT / 'contracts/gate6-transport-matrix.json').read_text())
        for linux, platform, product, expected in [
            (False, 'macos-arm64', 'Google Chrome', '157.0.8103.52'),
            (True, 'ubuntu24-arm64', 'Google Chrome for Testing', '156.0.8078.4'),
        ]:
            for transport in ['chrome-webtransport', 'chrome-wss-webrtc']:
                with self.subTest(platform=platform, transport=transport):
                    args = SimpleNamespace(transport=transport, chrome=Path('/exact/pinned/chrome'), firefox=None)
                    spec = runtime_spec(args, linux)
                    self.assertEqual(contract['expected_browsers_by_client_platform'][platform]['chrome'], expected)
                    self.assertEqual(spec['expectedBrowserVersion'], product + ' ' + expected)
                    self.assertEqual(spec['browserBinary'], '/exact/pinned/chrome')
        for linux in [False, True]:
            spec = runtime_spec(SimpleNamespace(transport='firefox-wss-webrtc', chrome=None,
                                               firefox=Path('/exact/pinned/firefox')), linux)
            self.assertEqual(spec['expectedBrowserVersion'], 'Mozilla Firefox 158.0.1')
            self.assertEqual(spec['browserBinary'], '/exact/pinned/firefox')

    def test_runtime_uses_the_actual_contract_table_instead_of_a_second_literal(self):
        contract = copy.deepcopy(json.loads((ROOT / 'contracts/gate6-transport-matrix.json').read_text()))
        contract['expected_browsers_by_client_platform']['macos-arm64']['chrome'] = '157.0.8103.40'
        contract['expected_browsers_by_client_platform']['ubuntu24-arm64']['chrome'] = '156.0.8078.5'
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'contracts').mkdir()
            (root / 'contracts/gate6-transport-matrix.json').write_text(json.dumps(contract))
            args = SimpleNamespace(transport='chrome-webtransport', chrome=None, firefox=None)
            with patch.dict(runtime_spec.__globals__, ROOT=root):
                self.assertEqual(runtime_spec(args, False)['expectedBrowserVersion'], 'Google Chrome 157.0.8103.40')
                self.assertEqual(runtime_spec(args, True)['expectedBrowserVersion'], 'Google Chrome for Testing 156.0.8078.5')

    def test_missing_observation_preserves_original_nonzero_exit_and_private_error(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            log = output / 'browser.log'
            message = 'AssertionError: actual Chrome157 differs from original expected Chrome156'
            log.write_text(message)
            command = ['node', 'browser_matrix_admission.mjs', 'original-input.json']
            with self.assertRaises(subprocess.CalledProcessError) as caught:
                read_observation(output, 1, command, log)
            self.assertEqual(caught.exception.returncode, 1)
            self.assertEqual(caught.exception.cmd, command)
            self.assertTrue(any(message in note for note in caught.exception.__notes__))
            self.assertFalse((output / 'browser-observation.json').exists())

    def test_successful_owner_without_observation_still_fails_and_existing_failure_is_retained(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            with self.assertRaises(FileNotFoundError):
                read_observation(output, 0, ['original'], output / 'browser.log')
            observed = {'passed': False, 'failure': {'name': 'AssertionError', 'stage': 'actual-browser'}}
            (output / 'browser-observation.json').write_text(json.dumps(observed))
            self.assertEqual(read_observation(output, 1, ['original'], output / 'browser.log'), observed)

    def test_missing_or_large_private_log_never_masks_exit_or_loads_an_unbounded_tail(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            log = output / 'guest-job.log'
            for exists in [False, True]:
                with self.subTest(exists=exists):
                    if exists:
                        log.write_text('excluded-prefix' + 'x' * 20000 + 'original-terminal-error')
                    with self.assertRaises(subprocess.CalledProcessError) as caught:
                        read_observation(output, -6, ['original-guest-browser'], log)
                    self.assertEqual(caught.exception.returncode, -6)
                    notes = '\n'.join(caught.exception.__notes__)
                    if exists:
                        self.assertIn('original-terminal-error', notes)
                        self.assertNotIn('excluded-prefix', notes)
                        self.assertLessEqual(len(notes), 8250)
                    else:
                        self.assertIn('Original browser log unavailable: FileNotFoundError', notes)


if __name__ == '__main__':
    unittest.main()
