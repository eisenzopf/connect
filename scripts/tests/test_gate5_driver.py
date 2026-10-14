"""Actual failed children, deadline/leak cleanup, interruption, and honest lane claims."""
import importlib.util
import json
import pathlib
import subprocess
import sys
import tempfile
import types
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from lib.gate4_phases import phases as retained
from lib.gate5_phases import LANES, phases
from lib.qualification_report import environment

spec = importlib.util.spec_from_file_location('gate5_driver', ROOT / 'scripts/check-gate-5.py')
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)


class Gate5Driver(unittest.TestCase):
    def setUp(self):
        parent = ROOT / '.local/gate5-driver-tests'
        parent.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=parent)
        self.path = pathlib.Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def execute(self, code, timeout=3, free=10 * 1024 ** 3):
        report = {'schema_version': 1, 'status': 'running', 'steps': [], 'source_inputs_sha256': {},
                  'whole_gate_qualified': False, 'selected_lanes': ['source'], 'unresolved': []}
        steps = [{'name': 'source--injected-build', 'lane': 'source',
                  'command': [sys.executable, '-c', code], 'timeout_seconds': timeout},
                 {'name': 'source--later', 'lane': 'source',
                  'command': [sys.executable, '-c', 'print("should-not-run")'], 'timeout_seconds': 3}]
        status = driver.run_plan(steps, report, self.path, environment(), snapshot=lambda: {},
                                disk=lambda _: types.SimpleNamespace(free=free))
        return status, json.loads((self.path / 'result.json').read_text())

    def assert_failure(self, code, timeout=3):
        status, report = self.execute(code, timeout)
        self.assertEqual(status, 1)
        self.assertEqual(report['status'], 'failed')
        self.assertEqual(report['not_run'], ['source--later'])
        self.assertEqual(report['steps'][0]['status'], 'failed')
        self.assertTrue((self.path / report['steps'][0]['log']).is_file())
        self.assertFalse(report['whole_gate_qualified'])
        return report

    def test_failed_phase_retains_output_and_stops_later(self):
        report = self.assert_failure('print("failed build evidence");raise SystemExit(7)')
        self.assertEqual(report['steps'][0]['exit_code'], 7)
        self.assertIn('failed build evidence', (self.path / report['steps'][0]['log']).read_text())

    def test_timeout_does_not_widen_budget(self):
        report = self.assert_failure('import time;time.sleep(60)', .2)
        self.assertEqual(report['steps'][0]['error'], 'TimeoutError')
        self.assertEqual(report['steps'][0]['timeout_seconds'], .2)

    def test_interrupted_build_is_nonzero(self):
        report = self.assert_failure('import os,signal;print("build started",flush=True);os.kill(os.getpid(),signal.SIGINT)')
        self.assertNotEqual(report['steps'][0]['exit_code'], 0)

    def test_disk_pressure_before_build_marks_all_not_run(self):
        status, report = self.execute('raise AssertionError("build must not start")', free=1)
        self.assertEqual(status, 1)
        self.assertEqual(report['steps'], [])
        self.assertEqual(report['not_run'], ['source--injected-build', 'source--later'])
        self.assertIn('disk headroom', report['error']['detail'])

    def test_live_child_leak_is_failed_and_cleaned(self):
        pid_file = self.path / 'child.pid'
        code = ('import subprocess,sys,pathlib\n'
                'child=subprocess.Popen([sys.executable,"-c","import time;time.sleep(60)"])\n'
                f'pathlib.Path({str(pid_file)!r}).write_text(str(child.pid))\n')
        report = self.assert_failure(code)
        self.assertEqual(report['steps'][0]['error'], 'LeakedProcessGroup')
        child = int(pid_file.read_text())
        state = subprocess.run(['ps', '-o', 'stat=', '-p', str(child)], text=True, capture_output=True, timeout=3)
        self.assertTrue(state.returncode != 0 or state.stdout.strip().startswith('Z'))

    def test_source_success_cannot_claim_whole_gate(self):
        status, report = self.execute('print("source passed")')
        self.assertEqual(status, 0)
        self.assertEqual(report['status'], 'passed-selected-lanes')
        self.assertFalse(report['whole_gate_qualified'])

    def test_disabled_upstream_features_cannot_pass_with_zero_tests(self):
        report = {'status': 'running', 'steps': [], 'source_inputs_sha256': {}}
        steps = [{'name': 'source--upstream-webrtc-preallocation', 'lane': 'source',
            'command': [sys.executable, '-c',
                'print("test result: ok. 0 passed; 0 failed; 0 ignored;")'],
            'timeout_seconds': 3}]
        self.assertEqual(driver.run_plan(steps, report, self.path, environment(),
            snapshot=lambda: {}), 1)
        self.assertIn('feature flags', report['steps'][0]['verification_error'])
        selected, _ = phases(['source'])
        for step in selected:
            if 'upstream-webrtc-' in step['name']:
                self.assertIn('signaling-ws,client', step['command'])

    def test_named_lanes_missing_prerequisites_remain_explicit(self):
        schema = json.loads((ROOT / "scripts/lib/gate5-evidence.schema.json").read_text())
        self.assertEqual(set(schema["properties"]["selected_lanes"]["items"]["enum"]), set(LANES))
        steps, unresolved = phases(list(LANES))
        missing = {item['lane'] for item in unresolved}
        self.assertTrue({'installed', 'scripted-host', 'actual-model', 'operating'} <= missing)
        self.assertFalse(any('actual-model' == s['lane'] for s in steps))
        source, _ = phases(['source'])
        actual = {s['name'].split('--', 1)[1]: s['timeout_seconds'] for s in source}
        self.assertEqual({name: seconds for name, _, seconds in retained(source_only=True)},
                         {name: actual[name] for name, _, _ in retained(source_only=True)})
        self.assertNotIn('--allow-model-inference', str(source))
        names = [step['name'] for step in source]
        self.assertEqual(names.index('source--connected-app-public-journey'),
                         names.index('source--current-binaries') + 1)
        for name, filename in [('package-proof-tests', 'test_package_inventory.py'),
                               ('connected-app-checker-tests', 'test_connected_app_checker.py')]:
            selected = next(step for step in source if step['name'] == 'source--' + name)
            self.assertIn(filename, selected['command'])
        first = next(step for step in source if step['name'] == 'source--connected-app-public-journey')
        repeated, _ = phases(['source'])
        second = next(step for step in repeated if step['name'] == first['name'])
        self.assertNotEqual(first['command'][first['command'].index('--output') + 1],
                            second['command'][second['command'].index('--output') + 1])

    def test_gate5_profile_requires_matching_package_and_hashes(self):
        sys.path.insert(0, str(ROOT / 'scripts/live'))
        from agent_journey.common import ProofFailure, sha256, verify_runtime
        runtime = self.path / 'runtime'
        files = {}
        for name in ('bin/connect', 'sdk/python/connect_sdk/native.py', 'skills/connect/SKILL.md',
                     'skills/connect/references/json-cli.md', 'docs/AGENT_ACCESS.md'):
            path = runtime / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('fixture only')
            files[name] = sha256(path)
        (runtime / 'bin/connect').chmod(0o700)
        manifest = {'target': 'aarch64-apple-darwin', 'qualification_profile': 'gate5-local',
                    'source_revision': 'fixture-revision', 'files': files}
        (runtime / 'manifest.json').write_text(json.dumps(manifest))
        report = self.path / 'package.json'
        report.write_text(json.dumps({'result': 'passed', 'manifest': manifest}))
        self.assertEqual(verify_runtime(runtime, report, 'gate5-local'), manifest)
        with self.assertRaises(ProofFailure):
            verify_runtime(runtime, report, 'gate4-local')
        (runtime / 'bin/connect').write_text('changed')
        with self.assertRaisesRegex(ProofFailure, 'runtime-file-hash'):
            verify_runtime(runtime, report, 'gate5-local')

    def test_lane_commands_do_not_allow_a_gate4_package_to_stand_in(self):
        selected, _ = phases(['installed', 'actual-model', 'native-keychain'],
                             pathlib.Path('/host'), pathlib.Path('/candidate'),
                             pathlib.Path('/runtime'), pathlib.Path('/report'), True,
                             independent_python=pathlib.Path('/pinned-python'))
        for step in selected:
            if step['lane'] in ('installed', 'actual-model'):
                self.assertIn('--profile', step['command'])
                self.assertIn('gate5-local', step['command'])
            if step['lane'] == 'installed':
                self.assertIn('--independent-python', step['command'])
                self.assertIn('/pinned-python', step['command'])
        keychain = [s for s in selected if s['lane'] == 'native-keychain']
        self.assertIn('--no-run', keychain[1]['command'])
        self.assertIn('isolated_native_keychain', keychain[2]['command'])
        self.assertEqual(keychain[2]['timeout_seconds'], 120)

    def test_account_secrets_are_removed_from_fixture_environment(self):
        env = environment()
        self.assertFalse(any('API_KEY' in key or 'TOKEN' in key for key in env))
        self.assertNotIn('OPENAI_BASE_URL', env)
        from lib.package_environment import fixture_environment
        selected = fixture_environment({'PATH': '/bin', 'CARGO_TARGET_DIR': '/owned/cache',
            'TELNYX_API_KEY': 'private', 'VAPI_PRIVATE_KEY': 'private',
            'CODEX_API_KEY': 'private', 'OPENAI_BASE_URL': 'https://private.invalid'})
        self.assertEqual(selected['CARGO_TARGET_DIR'], '/owned/cache')
        self.assertFalse(any('API_KEY' in key or 'PRIVATE_KEY' in key for key in selected))
        self.assertNotIn('OPENAI_BASE_URL', selected)


if __name__ == '__main__':
    unittest.main()
