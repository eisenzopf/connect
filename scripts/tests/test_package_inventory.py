"""Package inventory rejects missing, altered and unbound deliverable assets."""
import json
import pathlib
import shutil
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from lib.package_inventory import (GATE5_RUNTIME, GATE5_SOURCE, copy_gate5_assets,
                                   digest, inventory_gate5, standalone_api_manifest,
                                   verify_baseline)


class PackageInventoryTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.base = pathlib.Path(self.temporary.name)
        self.source = self.base / 'source'
        self.release = self.base / 'release'
        # Copy actual public examples/API; never author a duplicate fake adapter.
        for directory in ('examples/connector-template', 'examples/connector-contribution',
                          'examples/independent-uctp-client', 'examples/native-video', 'crates/connect-connector-api'):
            shutil.copytree(ROOT / directory, self.source / directory,
                            ignore=shutil.ignore_patterns('target', '__pycache__', '*.pyc'))
        for name in set(GATE5_SOURCE) | {name for name in GATE5_RUNTIME if not name.startswith(('bin/', 'sdk/typescript/dist/')) and name != 'connector-kit-source.json'} | {'Cargo.toml', 'scripts/lib/local_process.py', 'scripts/lib/qualification_process.py', 'scripts/lib/phase_supervisor.py'}:
            if (self.source / name).is_file():
                continue
            target = self.source / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, target)
        for name in GATE5_RUNTIME:
            if name.startswith(('examples/connector-', 'examples/independent-', 'examples/native-video/', 'crates/')) or name == 'connector-kit-source.json':
                continue
            target = self.release / name
            target.parent.mkdir(parents=True, exist_ok=True)
            if name.startswith('bin/'):
                target.write_bytes(b'inventory fixture only; never executed')
            else:
                shutil.copy2(ROOT / name, target)
        copy_gate5_assets(self.source, self.release)

    def test_complete_inventory_binds_public_api_and_examples(self):
        result = inventory_gate5(self.release, self.source)
        self.assertGreater(result['kit_files'], 20)
        self.assertFalse(result['behavior_qualified'])
        import tomllib
        standalone = tomllib.loads(standalone_api_manifest(self.source))
        workspace = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']
        self.assertEqual(standalone['dependencies']['serde'], workspace['dependencies']['serde'])
        self.assertIn('workspace', standalone)

    def test_missing_video_asset_is_not_a_gate4_pass(self):
        (self.release / 'sdk/typescript/dist/browser_video.js').unlink()
        with self.assertRaisesRegex(ValueError, 'browser_video.js'):
            inventory_gate5(self.release, self.source)

    def test_changed_contributor_source_rejected_even_with_runtime_hash_manifest(self):
        (self.release / 'examples/connector-contribution/src/lib.rs').write_text('// substituted fixture\n')
        with self.assertRaisesRegex(ValueError, 'contributor source mismatch'):
            inventory_gate5(self.release, self.source)

    def test_missing_fixture_source_is_rejected(self):
        (self.source / 'crates/connect-server/tests/gate5_support/video_codec.rs').unlink()
        with self.assertRaisesRegex(ValueError, 'video_codec.rs'):
            inventory_gate5(self.release, self.source)

    def test_extra_or_missing_kit_source_binding_rejected(self):
        path = self.release / 'connector-kit-source.json'
        kit = json.loads(path.read_text())
        kit['source_files']['../outside'] = '0' * 64
        path.write_text(json.dumps(kit))
        with self.assertRaisesRegex(ValueError, 'source inventory mismatch'):
            inventory_gate5(self.release, self.source)

    def test_api_dependency_drift_rejected(self):
        path = self.release / 'crates/connect-connector-api/Cargo.toml'
        path.write_text(path.read_text().replace('thiserror = "2"', 'thiserror = "1"'))
        with self.assertRaisesRegex(ValueError, 'dependency mismatch'):
            inventory_gate5(self.release, self.source)

    def test_required_asset_symlink_rejected(self):
        path = self.release / 'bin/connect-video'
        path.unlink()
        path.symlink_to(self.source / 'Cargo.toml')
        with self.assertRaisesRegex(ValueError, 'nonordinary'):
            inventory_gate5(self.release, self.source)

    def test_public_baseline_hash_revision_schema_and_patch_bound(self):
        baseline = self.base / 'connect-baseline'
        baseline.write_bytes(b'nonexecuted binary digest fixture')
        catalog = json.loads((self.source / 'dependencies/recovery-baselines.json').read_text())['baselines']['1']
        manifest = {'schema': 1, 'source_revision': catalog['revision'],
                    'source_repository': 'https://github.com/eisenzopf/connect.git',
                    'baseline_kind': catalog['kind'], 'patch': catalog['patch'],
                    'patch_sha256': catalog['patch_sha256'], 'binary_sha256': digest(baseline)}
        self.assertEqual(verify_baseline(baseline, manifest, self.source, 1)['kind'], 'identity-only-recovery-fixture')
        for key, wrong in [('schema', 16), ('source_revision', '0'*40), ('patch_sha256', None), ('binary_sha256', '0'*64), ('source_repository', 'https://invalid.example'), ('baseline_kind', 'unmodified'), ('patch', None)]:
            with self.subTest(key=key), self.assertRaises(ValueError):
                verify_baseline(baseline, dict(manifest, **{key: wrong}), self.source, 1)


    def native_cleanup_fixture(self, work):
        """Nonexecuted artifacts for verifier guards, never native process proof."""
        from lib.package_gate5 import native_cleanup_inputs
        server = self.release / 'bin/connect'
        if not server.exists():
            server.write_bytes(b'guard-only nonexecuted server')
        ffmpeg = self.base / 'selected-tool'
        if not ffmpeg.exists():
            ffmpeg.write_bytes(b'guard-only selected tool; never executed')
        attempt = work / 'guard-only-attempt'
        attempt.mkdir(parents=True, exist_ok=True)
        inputs = native_cleanup_inputs(self.release, self.source, ffmpeg)
        proof = {'checker_profile': 'connect-native-video-cleanup/1', 'status': 'passed',
            'source_revision': 'fixture-revision', 'tested_inputs_stable': True, 'not_run': [],
            'selected_runtime': str(self.release.resolve()), 'selected_source': str(self.source.resolve()),
            'selected_server_binary': str(server.resolve()), 'selected_video_binary': str((self.release / 'bin/connect-video').resolve()),
            'selected_ffmpeg': str(ffmpeg.resolve()), 'private_attempt_directory': str(attempt),
            'selected_binary_sha256': {'server': digest(server), 'video': digest(self.release / 'bin/connect-video'),
                'ffmpeg': digest(ffmpeg)}, 'input_sha256': inputs, 'input_sha256_after': dict(inputs), 'cases': []}
        for name in ('interrupt', 'capture-refusal'):
            context = {'conversation_id': name + '-conversation', 'session_id': name + '-session',
                'session_state': 'accepted', 'history_preserved': True, 'terminal_session_events': 0,
                'history_message_ids': [name + '-before']}
            later = dict(context, history_message_ids=[name + '-before', name + '-after'])
            case = {'case': name, 'status': 'passed', 'continuity': context,
                'continued_communication': dict(later), 'after_reattachment': dict(later)}
            for key in ('fault', 'fresh_reattachment'):
                row = {'status': 'passed', 'fault': name if key == 'fault' else 'interrupt',
                    'conversation_id': context['conversation_id'], 'session_id': context['session_id'],
                    'readiness': {'complete_incoming_keyframes': 2}, 'recovery_signals': [],
                    'cleanup': {'both_exited_within5seconds': True, 'remaining_owned_processes': [],
                        'forced_cleanup_required': False, 'participants': []}}
                for slot in (0, 1):
                    folder = attempt / name / key.replace('fresh_reattachment', 'fresh-reattachment') / ('reviews' if slot == 0 else 'agent')
                    folder.mkdir(parents=True, exist_ok=True)
                    (folder / 'observation.json').write_text('{"scope":"guard-only"}')
                    (folder / 'incoming.ivf').write_bytes(b'guard-only; not actual VP8')
                    failed = name == 'capture-refusal' and key == 'fault' and slot == 0
                    reason = 'child-failure' if failed else 'interrupt' if slot == (0 if key == 'fault' else 1) else 'peer-terminal'
                    (folder / 'observation.json').write_text(json.dumps({'end_reason': reason, 'success': not failed, 'teardown_seconds': .1, 'scope': 'guard-only'}))
                    row['cleanup']['participants'].append({'slot': slot, 'exit_code': 1 if failed else 0,
                        'end_reason': reason, 'success': not failed, 'trigger_to_exit_seconds': .2,
                        'teardown_seconds': .1, 'report_sha256': digest(folder / 'observation.json'),
                        'incoming_ivf_sha256': digest(folder / 'incoming.ivf')})
                if name == 'capture-refusal' and key == 'fault':
                    shim = attempt / name / 'fault/ffmpeg-refuse-audio.py'
                    shim.write_text('# guard-only unexecuted shim\n')
                    row.update(shim_sha256_before=digest(shim), shim_sha256_after=digest(shim),
                        shim_events=[{'kind': kind, 'hardware_invoked': False, 'audio_only': True}
                            for kind in ('blocked-avfoundation', 'capture-refusal', 'forward-generated-codec')])
                case[key] = row
            proof['cases'].append(case)
        return proof

    def test_native_cleanup_verifier_requires_both_faults_and_same_session_reattachments(self):
        import copy
        from lib.package_gate5 import verify_native_cleanup
        work = self.base / 'native-proof'
        proof = self.native_cleanup_fixture(work)
        self.assertEqual(verify_native_cleanup(proof, self.release, self.source,
            self.base / 'selected-tool', 'fixture-revision', work)['status'], 'passed')
        changes = {
            'missing fault': lambda p: p['cases'].pop(),
            'duplicate fault': lambda p: p['cases'].__setitem__(1, copy.deepcopy(p['cases'][0])),
            'missing reattachment': lambda p: p['cases'][0].pop('fresh_reattachment'),
            'another Session': lambda p: p['cases'][0]['fresh_reattachment'].__setitem__('session_id', 'foreign-session'),
            'lost history': lambda p: p['cases'][0]['after_reattachment'].__setitem__('history_message_ids', ['changed']),
            'resurrected terminal': lambda p: p['cases'][0]['after_reattachment'].__setitem__('terminal_session_events', 1),
            'no actual readiness': lambda p: p['cases'][0]['fault']['readiness'].__setitem__('complete_incoming_keyframes', 0),
        }
        for label, change in changes.items():
            bad = copy.deepcopy(proof)
            change(bad)
            with self.subTest(label=label), self.assertRaises(ValueError):
                verify_native_cleanup(bad, self.release, self.source, self.base / 'selected-tool', 'fixture-revision', work)

    def test_native_cleanup_verifier_rejects_substituted_or_changed_selected_inputs(self):
        import copy
        from lib.package_gate5 import verify_native_cleanup
        work = self.base / 'native-proof'
        proof = self.native_cleanup_fixture(work)
        changes = {
            'different revision': lambda p: p.__setitem__('source_revision', 'another-revision'),
            'different server': lambda p: p['selected_binary_sha256'].__setitem__('server', '0' * 64),
            'different video': lambda p: p['selected_binary_sha256'].__setitem__('video', '0' * 64),
            'different ffmpeg': lambda p: p['selected_binary_sha256'].__setitem__('ffmpeg', '0' * 64),
            'different source': lambda p: p.__setitem__('selected_source', str(self.base)),
            'before after mismatch': lambda p: p['input_sha256_after'].__setitem__(str(self.source / 'Cargo.toml'), '0' * 64),
            'missing marker observer': lambda p: p['input_sha256'].pop(str((self.source / 'scripts/video/marker_calibration.py').resolve())),
            'not run': lambda p: p.__setitem__('not_run', ['capture-refusal']),
        }
        for label, change in changes.items():
            bad = copy.deepcopy(proof)
            change(bad)
            with self.subTest(label=label), self.assertRaises(ValueError):
                verify_native_cleanup(bad, self.release, self.source, self.base / 'selected-tool', 'fixture-revision', work)
        (self.source / 'examples/native-video/src/main.rs').write_text('// changed after proof\n')
        with self.assertRaisesRegex(ValueError, 'before/after inputs'):
            verify_native_cleanup(proof, self.release, self.source, self.base / 'selected-tool', 'fixture-revision', work)

    def test_native_cleanup_verifier_rejects_forced_cleanup_leaks_false_success_and_deadlines(self):
        import copy
        from lib.package_gate5 import verify_native_cleanup
        work = self.base / 'native-proof'
        proof = self.native_cleanup_fixture(work)
        changes = {
            'forced group signal': lambda r: r.__setitem__('recovery_signals', [{'signal': 'SIGKILL'}]),
            'forced cleanup': lambda r: r['cleanup'].__setitem__('forced_cleanup_required', True),
            'descendant leak': lambda r: r['cleanup'].__setitem__('remaining_owned_processes', [{'pid': 99}]),
            'late process exit': lambda r: r['cleanup']['participants'][0].__setitem__('trigger_to_exit_seconds', 5.01),
            'nonfinite time': lambda r: r['cleanup']['participants'][0].__setitem__('teardown_seconds', float('nan')),
            'boolean time': lambda r: r['cleanup']['participants'][0].__setitem__('trigger_to_exit_seconds', True),
            'false success': lambda r: r['cleanup']['participants'][0].__setitem__('success', False),
            'wrong reason': lambda r: r['cleanup']['participants'][0].__setitem__('end_reason', 'deadline'),
        }
        for label, change in changes.items():
            bad = copy.deepcopy(proof)
            change(bad['cases'][0]['fault'])
            with self.subTest(label=label), self.assertRaises(ValueError):
                verify_native_cleanup(bad, self.release, self.source, self.base / 'selected-tool', 'fixture-revision', work)
        bad = copy.deepcopy(proof)
        bad['cases'][1]['fault']['shim_events'][0]['hardware_invoked'] = True
        with self.assertRaisesRegex(ValueError, 'before hardware'):
            verify_native_cleanup(bad, self.release, self.source, self.base / 'selected-tool', 'fixture-revision', work)
        (work / 'guard-only-attempt/interrupt/fresh-reattachment/agent/incoming.ivf').write_bytes(b'changed artifact')
        with self.assertRaisesRegex(ValueError, 'artifact changed'):
            verify_native_cleanup(proof, self.release, self.source, self.base / 'selected-tool', 'fixture-revision', work)

    def test_native_cleanup_binds_archived_source_inside_private_local_directory(self):
        from lib.package_gate5 import verify_native_cleanup
        archive = self.base / '.local/archived-source'
        archive.parent.mkdir()
        self.source.rename(archive)
        self.source = archive
        work = self.base / '.local/native-proof'
        proof = self.native_cleanup_fixture(work)
        self.assertIn(str((archive / 'examples/native-video/src/main.rs').resolve()), proof['input_sha256'])
        self.assertIn(str((archive / 'crates/connect-connector-api/src/lib.rs').resolve()), proof['input_sha256'])
        self.assertIn(str((archive / 'scripts/video/source_identity.mjs').resolve()), proof['input_sha256'])
        self.assertEqual(verify_native_cleanup(proof, self.release, archive,
            self.base / 'selected-tool', 'fixture-revision', work)['status'], 'passed')

    def test_installed_native_cleanup_failure_is_required_and_saved_as_failure(self):
        def remove_reattachment(proof):
            proof['cases'][1].pop('fresh_reattachment')
        with self.assertRaisesRegex(ValueError, 'structured observation'):
            self.installed_hook(independent=True, media=True, cleanup_change=remove_reattachment)
        saved = json.loads((self.base / 'qualification/result.json').read_text())
        self.assertEqual(saved['status'], 'failed')
        self.assertNotIn('native_example_cleanup', saved)
        self.assertEqual(saved['steps'][-1]['name'], 'installed-native-example-cleanup')

    def installed_hook(self, *, bad_count=False, independent=False, wrong_binary=False, media=False, smoke=False, cleanup_change=None):
        from unittest.mock import patch
        from lib.package_gate5 import qualify
        directory = self.base / 'qualification'
        (self.release / 'bin/connect').write_bytes(b'nonexecuted installed fixture binary')
        calls = []
        (self.base / 'selected-tool').write_bytes(b'guard-only selected tool; never executed')
        def phase(name, command, timeout, output, cwd, env):
            calls.append((name, command, timeout, cwd, dict(env)))
            log = name + '.log'
            text = 'test result: ok. 0 passed; 0 failed; 0 ignored;\n' if bad_count else 'test result: ok. 4 passed; 0 failed; 0 ignored;\n'
            if name == 'independent-client-vectors':
                text = '\nRan 6 tests in 0.1s\n\nOK\n'
            if name == 'native-storage-installed-cli':
                text = 'test result: ok. 1 passed; 0 failed; 0 ignored;\n' * 2
            if name == 'source-media-failure-fixture':
                text = 'test result: ok. 7 passed; 0 failed; 0 ignored;\n'
            if name == 'installed-media-failures':
                from lib.media_failure_proof import EXTERNAL_FAILURE_TESTS
                text = 'test result: ok. 7 passed; 0 failed; 0 ignored;\n'
                for case in sorted(EXTERNAL_FAILURE_TESTS):
                    text += 'INSTALLED_VIDEO_FAILURE_RESULT ' + json.dumps({
                        'case': case, 'status': 'passed', 'binary_rechecked': True,
                        'selected_server_binary': str((self.release / 'bin/connect').resolve()),
                        'selected_server_sha256': digest(self.release / 'bin/connect'),
                        'source_revision': 'fixture-revision',
                        'fixture_source_sha256': digest(self.source / 'crates/connect-server/tests/gate5_support/installed_video.rs'),
                        'test_source_sha256': digest(self.source / 'crates/connect-server/tests/gate5_installed_video_failures.rs'),
                        'public_cli_calls': 3, 'lifecycle_limit_seconds': 5,
                        'startup_seconds': [.1], 'shutdown_seconds': [.1],
                    }) + '\n'

            (output / log).write_text(text)
            if name == 'installed-independent-control':
                evidence = {'status': 'passed-control-only', 'tested_inputs_stable': True,
                    'server_sha256': '0'*64 if wrong_binary else digest(self.release / 'bin/connect'),
                    'client_sha256': digest(self.release / 'examples/independent-uctp-client/client.py'),
                    'source_revision': 'fixture-revision'}
                (output / 'independent-control.json').write_text(json.dumps(evidence))
            if name == 'installed-public-setup':
                evidence = {'status': 'passed', 'tested_inputs_stable': True,
                    'binary_sha256': digest(self.release / 'bin/connect'),
                    'candidate_schema': 18,
                    'source_revision': 'fixture-revision'}
                (output / 'public-setup.json').write_text(json.dumps(evidence))
            if name == 'installed-native-example-cleanup':
                evidence = self.native_cleanup_fixture(output / 'native-example-cleanup-state')
                if cleanup_change:
                    cleanup_change(evidence)
                (output / 'native-example-cleanup.json').write_text(json.dumps(evidence))
            if name == 'installed-reference-client':
                evidence = {'result': 'passed', 'tested_inputs_stable': True,
                    'source_revision': 'fixture-revision', 'runtime_root': str(self.release),
                    'binary_sha256': digest(self.release / 'bin/connect'),
                    'video': {'status': 'passed', 'failures': [
                        {'failure': 'capture-ended', 'cleanupMs': 100},
                        {'failure': 'ice-failed', 'cleanupMs': 100}]}}
                pathlib.Path(env['CONNECT_REFERENCE_OUTPUT']).write_text(json.dumps(evidence))
            if name.startswith('installed-decoded-video'):
                label = name.removeprefix('installed-')
                evidence = {'status': 'passed-mute-recovery' if '--mute-recovery' in command else 'passed-impairment' if '--impairment' in command else 'passed-clean',
                    'qualification_seconds': 8 if smoke else int(command[command.index('--seconds') + 1]), 'tested_inputs_stable': True,
                    'source_revision': 'fixture-revision', 'selected_runtime': str(self.release),
                    'selected_assets': str(self.release), 'input_sha256': {
                        str(self.release / 'bin/connect'): digest(self.release / 'bin/connect'),
                        str(self.release / 'bin/connect-video'): digest(self.release / 'bin/connect-video')},
                    'paths': [{'path': route} for route in ('native-native', 'native-browser', 'browser-browser')],
                    'unresolved': []}
                (output / (label + '.json')).write_text(json.dumps(evidence))
            return {'name': name, 'status': 'passed', 'log': log}
        with patch('lib.package_gate5.run_phase', side_effect=phase):
            result = qualify(self.release, self.source, {'source_revision': 'fixture-revision', 'schema': 18},
                             directory, self.base / 'selected-python' if independent else None,
                             dict.fromkeys(('node', 'ffmpeg', 'chrome', 'playwright'), self.base / 'selected-tool') if media else None)
        return result, calls

    def test_installed_hook_remains_incomplete_and_binds_owned_paths(self):
        result, calls = self.installed_hook()
        self.assertEqual(result['status'], 'incomplete')
        self.assertFalse(result['whole_gate_qualified'])
        self.assertEqual(len(result['unresolved']), 4)
        for name, command, timeout, cwd, env in calls:
            self.assertEqual(env['CONNECT_BINARY'], str(self.release / 'bin/connect'))
            self.assertEqual(cwd, self.source if (name.startswith(('native-storage-', 'source-media-', 'external-media-', 'native-example-')) or name == 'installed-media-failures') else self.release)
            self.assertTrue(pathlib.Path(env['PRISM_TEST_STATE_DIR']).is_relative_to(self.base / 'qualification'))
            if '-cold-' in name or name.endswith('-fixture-build'):
                self.assertEqual(timeout, 2400)
            elif name == 'installed-native-example-cleanup':
                self.assertEqual(timeout, 300)
            elif name == 'installed-public-setup':
                self.assertEqual(timeout, 180)
            else:
                self.assertEqual(timeout, 120)
            if name.endswith('-contract'):
                self.assertEqual(command[command.index('--public-root') + 1], str(self.release))
                self.assertEqual(command[command.index('--target-dir') + 1], env['CARGO_TARGET_DIR'])

    def test_success_exit_without_executed_connector_tests_rejected(self):
        with self.assertRaisesRegex(ValueError, 'four required checks'):
            self.installed_hook(bad_count=True)
        saved = json.loads((self.base / 'qualification/result.json').read_text())
        self.assertEqual(saved['status'], 'failed')
        self.assertFalse(saved['whole_gate_qualified'])

    def test_clean_and_impaired_routes_are_separate_required_phases(self):
        result, calls = self.installed_hook(independent=True, media=True)
        names = {name for name, *_ in calls}
        self.assertTrue({'installed-decoded-video', 'installed-decoded-video-impaired', 'installed-decoded-video-mute-recovery'} <= names)
        self.assertEqual(result['status'], 'incomplete')  # historical recovery not selected
        self.assertEqual(result['decoded_video']['status'], 'passed-clean')
        self.assertEqual(result['decoded_video_impaired']['status'], 'passed-impairment')
        self.assertEqual(result['decoded_video_mute_recovery']['status'], 'passed-mute-recovery')
        self.assertFalse(result['whole_gate_qualified'])
        self.assertEqual(result['native_example_cleanup']['status'], 'passed')
        cleanup = next(command for name, command, *_ in calls if name == 'installed-native-example-cleanup')
        self.assertEqual(cleanup[cleanup.index('--runtime') + 1], str(self.release))
        self.assertEqual(cleanup[cleanup.index('--source-path') + 1], str(self.source))
        self.assertEqual(cleanup[cleanup.index('--ffmpeg') + 1], str(self.base / 'selected-tool'))

    def test_external_runtime_failure_binding_rejects_missing_or_substituted_cases(self):
        from lib.media_failure_proof import verify_external_failures
        result, calls = self.installed_hook()
        directory = self.base / 'qualification'
        log = directory / 'installed-media-failures.log'
        phase = {'log': log.name, 'rust_tests': {'passed': 7, 'failed': 0, 'ignored': 0}}
        saved = log.read_text()
        lines = saved.splitlines()
        # Seven successes cannot substitute for a missing individual observation.
        log.write_text('\n'.join(lines[:-1]) + '\n')
        with self.assertRaisesRegex(ValueError, 'Missing or duplicated'):
            verify_external_failures(phase, directory, self.release / 'bin/connect', self.source, 'fixture-revision')
        log.write_text(saved.replace('"selected_server_sha256": "', '"selected_server_sha256": "substituted-'))
        with self.assertRaisesRegex(ValueError, 'selected inputs'):
            verify_external_failures(phase, directory, self.release / 'bin/connect', self.source, 'fixture-revision')
        log.write_text(saved.replace('"startup_seconds": [0.1]', '"startup_seconds": [5.1]'))
        with self.assertRaisesRegex(ValueError, 'deadline'):
            verify_external_failures(phase, directory, self.release / 'bin/connect', self.source, 'fixture-revision')

    def test_smoke_duration_cannot_qualify_installed_media(self):
        with self.assertRaisesRegex(ValueError, 'every candidate route/input'):
            self.installed_hook(independent=True, media=True, smoke=True)
        self.assertEqual(json.loads((self.base / 'qualification/result.json').read_text())['status'], 'failed')

    def test_independent_pass_for_another_binary_rejected(self):
        with self.assertRaisesRegex(ValueError, 'candidate inputs'):
            self.installed_hook(independent=True, wrong_binary=True)
        saved = json.loads((self.base / 'qualification/result.json').read_text())
        self.assertEqual(saved['status'], 'failed')
        self.assertFalse(saved['whole_gate_qualified'])


if __name__ == '__main__':
    unittest.main()
