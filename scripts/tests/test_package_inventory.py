"""Package inventory rejects missing, altered and unbound deliverable assets."""
import json
import pathlib
import shutil
import subprocess
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
        for name in set(GATE5_SOURCE) | {name for name in GATE5_RUNTIME if not name.startswith(('bin/', 'sdk/typescript/dist/')) and name != 'connector-kit-source.json'} | {'Cargo.toml', 'scripts/lib/local_process.py', 'scripts/lib/qualification_process.py', 'scripts/lib/qualification_report.py', 'scripts/lib/phase_supervisor.py'}:
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

    def test_installed_extraction_canonicalizes_root_without_loosening_media_binding(self):
        import ast
        import copy
        import hashlib
        tree = ast.parse((ROOT / 'scripts/check-package.py').read_text())
        extraction = next(node for node in ast.walk(tree) if isinstance(node, ast.With)
            and any(isinstance(item.context_expr, ast.Call)
                    and isinstance(item.context_expr.func, ast.Attribute)
                    and item.context_expr.func.attr == 'TemporaryDirectory'
                    for item in node.items))
        assignment = extraction.body[0]
        self.assertIsInstance(assignment, ast.Assign)
        self.assertEqual([target.id for target in assignment.targets], ['work'])
        physical = self.base / 'installed-root'
        physical.mkdir()
        alias = self.base / 'installed-alias'
        alias.symlink_to(physical, target_is_directory=True)
        scope = {'pathlib': pathlib, 'temporary': str(alias)}
        exec(compile(ast.Module(body=[assignment], type_ignores=[]), '<actual extraction root>', 'exec'), scope)
        self.assertEqual(scope['work'], physical.resolve(strict=True))

        # Exercise the real parent binding predicate against canonical child
        # observations, rather than accepting basename/hash-only substitutes.
        release = scope['work'] / 'candidate'
        (release / 'bin').mkdir(parents=True)
        for name in ('connect', 'connect-video'):
            (release / 'bin' / name).write_bytes(name.encode())
        hashes = {name: hashlib.sha256((release / 'bin' / name).read_bytes()).hexdigest()
                  for name in ('connect', 'connect-video')}
        module = ast.parse((ROOT / 'scripts/lib/package_gate5.py').read_text())
        binding = next(node.test for node in ast.walk(module) if isinstance(node, ast.If)
            and any(isinstance(row, ast.Raise) and isinstance(row.exc, ast.Call)
                    and row.exc.args and isinstance(row.exc.args[0], ast.Constant)
                    and row.exc.args[0].value == 'Installed decoded paths did not bind every candidate route/input'
                    for row in node.body))
        predicate = compile(ast.Expression(binding), '<actual installed media binding>', 'eval')
        observed = {'status': 'passed-clean', 'tested_inputs_stable': True,
                    'source_revision': 'candidate-revision', 'selected_runtime': str(release),
                    'selected_assets': str(release)}
        context = {'observed': observed, 'expected_status': 'passed-clean',
                   'manifest': {'source_revision': 'candidate-revision'}, 'release': release,
                   'bound': {str(release / 'bin' / name): checksum for name, checksum in hashes.items()},
                   'report': {'installed_binary_sha256': hashes['connect'],
                              'installed_video_sha256': hashes['connect-video']}}
        self.assertFalse(eval(predicate, context))
        for field in ('selected_runtime', 'selected_assets'):
            bad = dict(context, observed=copy.deepcopy(observed))
            bad['observed'][field] = str(self.base / 'foreign' / 'candidate')
            self.assertTrue(eval(predicate, bad), field)
        for name in hashes:
            bad = dict(context, bound=dict(context['bound']))
            bad['bound'][str(release / 'bin' / name)] = '0' * 64
            self.assertTrue(eval(predicate, bad), name)

    def copy_recovery_inputs(self):
        # These are inherited Gate4 qualification helpers, outside the explicit
        # Gate5 asset subset copied by this inventory fixture's setUp.
        for name in ('recovery_upgrade', 'recovery_tasks', 'recovery_interruption',
                     'recovery_restore_interruption', 'recovery_state', 'profile_metrics'):
            shutil.copy2(ROOT / f'scripts/lib/{name}.py', self.source / f'scripts/lib/{name}.py')
        shutil.copy2(ROOT / 'scripts/check-recovery.py', self.source / 'scripts/check-recovery.py')

    def recovery_hashes_through_source_alias(self):
        alias = self.base / 'source-alias'
        if not alias.exists():
            alias.symlink_to(self.source, target_is_directory=True)
        # A separate interpreter imports the actual copied helpers through the
        # alias. No package binary, recovery state, provider or media is invoked.
        probe = """import json, pathlib, runpy, sys
sys.path.insert(0, str(pathlib.Path(sys.argv[1]).parent))
checker = runpy.run_path(sys.argv[1])
assert pathlib.Path(checker['__file__']) != pathlib.Path(checker['__file__']).resolve()
print(json.dumps(checker['source_input_hashes'](), sort_keys=True))
"""
        return subprocess.run([sys.executable, '-B', '-c', probe,
                               str(alias / 'scripts/check-gate5-recovery.py')],
                              text=True, capture_output=True, timeout=15)

    def test_recovery_hashes_bind_actual_sources_through_symlinked_source_root(self):
        self.copy_recovery_inputs()
        expected_names = {'scripts/check-gate5-recovery.py', 'scripts/check-recovery.py',
                          'scripts/lib/recovery_upgrade.py', 'scripts/lib/recovery_tasks.py',
                          'scripts/lib/recovery_interruption.py',
                          'scripts/lib/recovery_restore_interruption.py',
                          'scripts/lib/recovery_state.py', 'scripts/lib/local_process.py',
                          'scripts/lib/profile_metrics.py'}
        result = self.recovery_hashes_through_source_alias()
        self.assertEqual(result.returncode, 0, result.stderr)
        before = json.loads(result.stdout)
        self.assertEqual(before, {name: digest(self.source / name) for name in expected_names})
        changed = 'scripts/lib/recovery_upgrade.py'
        target = self.source / changed
        target.write_bytes(target.read_bytes() + b'\n# retained input mutation\n')
        result = self.recovery_hashes_through_source_alias()
        self.assertEqual(result.returncode, 0, result.stderr)
        after = json.loads(result.stdout)
        self.assertNotEqual(before[changed], after[changed])
        self.assertEqual(after[changed], digest(target))
        self.assertEqual({k: v for k, v in before.items() if k != changed},
                         {k: v for k, v in after.items() if k != changed})

    def test_recovery_source_hashing_rejects_symlink_escape_even_with_identical_bytes(self):
        self.copy_recovery_inputs()
        target = self.source / 'scripts/lib/recovery_upgrade.py'
        outside = self.base / 'foreign-recovery-upgrade.py'
        outside.write_bytes(target.read_bytes())
        target.unlink()
        target.symlink_to(outside)
        result = self.recovery_hashes_through_source_alias()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('ValueError', result.stderr)
        self.assertEqual(result.stdout, '')

    def test_prepared_crate_notices_bind_selected_source_and_preserve_license_text(self):
        from lib.package_inventory import copy_prepared_crate_notices
        crate = self.source / '.deps/fixture'
        crate.mkdir(parents=True)
        (crate / 'Cargo.toml').write_text('[package]\nname="fixture"\nversion="1"\n')
        (crate / 'LICENSE').write_text('fixture license bytes\n')
        source = {'kind': 'crate', 'version': '1', 'license': 'MIT',
                  'url': 'https://static.crates.io/crates/fixture/fixture-1.crate', 'sha256': 'a' * 64}
        retained = self.source / 'LICENSE-APACHE'; retained.write_text('retained upstream text\n')
        source['license_files'] = [{'path': 'LICENSE-APACHE', 'sha256': digest(retained), 'url': 'https://example.test/pinned/LICENSE-APACHE'}]
        patches = [{'target': 'fixture', 'path': 'patches/fixture.patch', 'sha256': 'b' * 64}]
        (self.source / 'dependencies').mkdir(exist_ok=True)
        (self.source / 'dependencies/source-lock.json').write_text(json.dumps({'sources': {'fixture': source}, 'patches': patches}))
        package = {'name': 'fixture', 'version': '1', 'manifest_path': str(crate / 'Cargo.toml'), 'source': None}
        notices = copy_prepared_crate_notices(self.source, self.release, {'packages': [package]})
        self.assertEqual(notices[0]['source'], source)
        self.assertEqual(notices[0]['patches'], patches)
        self.assertEqual((self.release / notices[0]['texts'][0]).read_text(), 'fixture license bytes\n')
        self.assertEqual(json.loads((self.release / 'licenses/prepared-index.json').read_text()), notices)
        self.assertEqual((self.release / notices[0]['texts'][1]).read_text(), 'retained upstream text\n')
        retained.write_text('changed text')
        with self.assertRaisesRegex(ValueError, 'license digest'):
            copy_prepared_crate_notices(self.source, self.release, {'packages': [package]})
        retained.write_text('retained upstream text\n')
        package['manifest_path'] = str(self.source / 'unselected/Cargo.toml')
        with self.assertRaisesRegex(ValueError, 'selected package graph'):
            copy_prepared_crate_notices(self.source, self.release, {'packages': [package]})

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
        (self.release / 'bin/connect-example').write_bytes(b'nonexecuted installed example fixture binary')
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
                from lib.setup_provider_fixture import CHECKS
                evidence = {'status': 'passed', 'tested_inputs_stable': True,
                    'binary_sha256': digest(self.release / 'bin/connect'),
                    'candidate_schema': 18,
                    'source_revision': 'fixture-revision',
                    'provider_setup': {'status': 'passed',
                        'checks': [{'name': name, 'status': 'passed'} for name in CHECKS],
                        'provider_http_methods': ['GET'], 'provider_http_count': 2,
                        'resource_mutations': 0, 'real_account_requests': 0,
                        'preview_and_readiness_inputs_unchanged': True}}
                (output / 'public-setup.json').write_text(json.dumps(evidence))
            if name == 'installed-native-example-cleanup':
                evidence = self.native_cleanup_fixture(output / 'native-example-cleanup-state')
                if cleanup_change:
                    cleanup_change(evidence)
                (output / 'native-example-cleanup.json').write_text(json.dumps(evidence))
            if name == 'installed-connected-app':
                evidence = self.connected_app_fixture()
                (output / 'connected-app.json').write_text(json.dumps(evidence))
            if name == 'installed-session-controls':
                (output / 'session-controls.json').write_text(json.dumps(self.session_controls_fixture()))
            if name == 'installed-reference-client':
                evidence = {'result': 'passed', 'tested_inputs_stable': True,
                    'source_revision': 'fixture-revision', 'runtime_root': str(self.release),
                    'binary_sha256': digest(self.release / 'bin/connect'),
                    'video': {'status': 'passed', 'continuity': self.reference_continuity_fixture(), 'failures': [
                        {'failure': 'capture-ended', 'cleanupMs': 100},
                        {'failure': 'ice-failed', 'cleanupMs': 100}]}}
                pathlib.Path(env['CONNECT_REFERENCE_OUTPUT']).write_text(json.dumps(evidence))
            if name.startswith('installed-decoded-video'):
                label = name.removeprefix('installed-')
                status = 'passed-mute-recovery' if '--mute-recovery' in command else 'passed-impairment' if '--impairment' in command else 'passed-clean'
                seconds = int(command[command.index('--seconds') + 1])
                evidence = self.decoded_routes_fixture(status, seconds)
                evidence.update(qualification_seconds=8 if smoke else seconds, tested_inputs_stable=True,
                    source_revision='fixture-revision', selected_runtime=str(self.release),
                    selected_assets=str(self.release), input_sha256={
                        str(self.release / 'bin/connect'): digest(self.release / 'bin/connect'),
                        str(self.release / 'bin/connect-video'): digest(self.release / 'bin/connect-video')},
                    unresolved=[])
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
        self.assertEqual(len(result['unresolved']), 5)
        for name, command, timeout, cwd, env in calls:
            self.assertEqual(env['CONNECT_BINARY'], str(self.release / 'bin/connect'))
            self.assertEqual(cwd, self.source if (name.startswith(('native-storage-', 'source-media-', 'external-media-', 'native-example-')) or name == 'installed-media-failures') else self.release)
            self.assertTrue(pathlib.Path(env['PRISM_TEST_STATE_DIR']).is_relative_to(self.base / 'qualification'))
            if '-cold-' in name or name.endswith('-fixture-build'):
                self.assertEqual(timeout, 2400)
            elif name == 'installed-native-example-cleanup':
                self.assertEqual(timeout, 300)
            elif name in ('installed-public-setup', 'installed-connected-app', 'installed-session-controls'):
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

    def decoded_routes_fixture(self, status, seconds):
        import copy
        from video.acceptance_policy import POLICY, audio_video_acceptance
        from video.marker_calibration import marker_frames, ordinal_sync
        scenario = {'passed-clean': 'clean', 'passed-impairment': 'impairment',
                    'passed-mute-recovery': 'mute-recovery'}[status]
        av_scenario = 'mute' if scenario == 'mute-recovery' else scenario
        audio = [frame / 30 for frame in marker_frames() if frame / 30 < seconds - .1]
        sync = ordinal_sync([at + .04 for at in audio], audio, seconds)
        self.assertEqual(sync['status'], 'passed')
        participant = {'av_sync': sync, 'av_acceptance': audio_video_acceptance(sync, av_scenario, seconds)}
        proof = {'status': status, 'qualification_seconds': seconds, 'scenario': scenario,
            'acceptance_policy': POLICY, 'paths': []}
        for route, native_count in [('native-native', 2), ('native-browser', 1), ('browser-browser', 0)]:
            row = {'path': route, 'status': status, 'qualification_seconds': seconds,
                   'scenario': scenario, 'acceptance_policy': POLICY,
                   'native': [copy.deepcopy(participant) for _ in range(native_count)]}
            if native_count != 2:
                row['browser'] = {'acceptance_policy': POLICY,
                    'observations': [copy.deepcopy(participant) for _ in range(2 - native_count)]}
                for browser in row['browser']['observations']:
                    browser['av_sync']['audio_envelope'] = {'status': 'observer-valid',
                        'overflow': False, 'invalid_input': False, 'invalid': []}
            proof['paths'].append(row)
        return proof

    def test_decoded_route_proof_rejects_partial_duplicate_and_wrong_scenarios(self):
        import copy
        from lib.package_gate5 import verify_decoded_routes
        from video.acceptance_policy import POLICY, audio_video_acceptance
        from video.marker_calibration import marker_frames, ordinal_sync
        for status, seconds in [('passed-clean', 60), ('passed-impairment', 60), ('passed-mute-recovery', 20)]:
            proof = self.decoded_routes_fixture(status, seconds)
            scenario = proof['scenario']
            audio = [frame / 30 for frame in marker_frames() if frame / 30 < seconds - .1]
            verify_decoded_routes(proof, status, seconds)
            if scenario == 'impairment':
                degraded = copy.deepcopy(proof)
                for observer in [degraded['paths'][0]['native'][0],
                                 degraded['paths'][2]['browser']['observations'][0]]:
                    envelope = observer['av_sync'].get('audio_envelope')
                    observer['av_sync'] = ordinal_sync([at + .5 for at in audio], audio, seconds)
                    if envelope is not None:
                        observer['av_sync']['audio_envelope'] = envelope
                    self.assertEqual(observer['av_sync']['status'], 'failed')
                    observer['av_acceptance'] = audio_video_acceptance(observer['av_sync'], 'impairment', seconds)
                verify_decoded_routes(degraded, status, seconds)
            changes = {
                'missing route': lambda p: p['paths'].pop(),
                'duplicate route': lambda p: p['paths'].append(copy.deepcopy(p['paths'][0])),
                'failed route': lambda p: p['paths'][0].__setitem__('status', 'failed'),
                'wrong scenario': lambda p: p['paths'][0].__setitem__('status', 'passed-smoke'),
                'short route': lambda p: p['paths'][0].__setitem__('qualification_seconds', seconds - 1),
                'missing duration': lambda p: p['paths'][0].pop('qualification_seconds'),
                'nonfinite route': lambda p: p['paths'][0].__setitem__('qualification_seconds', float('nan')),
                'nonfinite aggregate': lambda p: p.__setitem__('qualification_seconds', float('inf')),
                'boolean duration': lambda p: p['paths'][0].__setitem__('qualification_seconds', True),
                'missing observation': lambda p: p['paths'].__setitem__(0, None),
                'missing aggregate policy': lambda p: p.pop('acceptance_policy'),
                'old route policy': lambda p: p['paths'][0].__setitem__('acceptance_policy', 'legacy'),
                'wrong policy scenario': lambda p: p['paths'][0]['native'][0]['av_acceptance'].__setitem__('scenario', 'unknown'),
                'wrong aggregate scenario': lambda p: p.__setitem__('scenario', 'unknown'),
                'wrong route scenario': lambda p: p['paths'][0].__setitem__('scenario', 'unknown'),
                'missing participant': lambda p: p['paths'][0]['native'].pop(),
                'missing browser observer': lambda p: p['paths'][2]['browser']['observations'].pop(),
                'missing browser policy': lambda p: p['paths'][2]['browser'].pop('acceptance_policy'),
                'missing browser envelope': lambda p: p['paths'][2]['browser']['observations'][0]['av_sync'].pop('audio_envelope'),
                'invalid browser envelope': lambda p: p['paths'][2]['browser']['observations'][0]['av_sync']['audio_envelope'].__setitem__('status', 'observer-invalid'),
                'fabricated acceptance': lambda p: p['paths'][0]['native'][0]['av_acceptance'].__setitem__('reason', 'passed'),
                'changed raw observation': lambda p: p['paths'][0]['native'][0]['av_sync'].__setitem__('paired', 999),
            }
            for label, change in changes.items():
                bad = copy.deepcopy(proof)
                change(bad)
                with self.subTest(status=status, defect=label), self.assertRaisesRegex(ValueError, 'every candidate route/input'):
                    verify_decoded_routes(bad, status, seconds)
            if scenario == 'clean':
                # A clean result cannot borrow impairment's quality exception.
                bad = copy.deepcopy(proof)
                altered = bad['paths'][0]['native'][0]
                altered['av_sync'] = ordinal_sync([at + .5 for at in audio], audio, seconds)
                altered['av_acceptance'] = audio_video_acceptance(altered['av_sync'], 'impairment', seconds)
                self.assertEqual(altered['av_acceptance']['status'], 'passed')
                with self.assertRaisesRegex(ValueError, 'every candidate route/input A/V policy'):
                    verify_decoded_routes(bad, status, seconds)

    def test_provider_setup_proof_requires_every_read_only_case(self):
        import copy
        from lib.package_gate5 import verify_provider_setup
        from lib.setup_provider_fixture import CHECKS
        proof = {'status': 'passed', 'checks': [{'name': name, 'status': 'passed'} for name in CHECKS],
            'provider_http_methods': ['GET'], 'provider_http_count': 2,
            'resource_mutations': 0, 'real_account_requests': 0,
            'preview_and_readiness_inputs_unchanged': True}
        verify_provider_setup(proof)
        changes = {
            'missing case': lambda p: p['checks'].pop(),
            'duplicate case': lambda p: p['checks'].append(copy.deepcopy(p['checks'][0])),
            'failed case': lambda p: p['checks'][0].__setitem__('status', 'failed'),
            'provider mutation': lambda p: p.__setitem__('resource_mutations', 1),
            'provider account': lambda p: p.__setitem__('real_account_requests', 1),
            'mutating HTTP': lambda p: p.__setitem__('provider_http_methods', ['GET', 'POST']),
            'unexpected request': lambda p: p.__setitem__('provider_http_count', 3),
            'changed input': lambda p: p.__setitem__('preview_and_readiness_inputs_unchanged', False),
        }
        for label, change in changes.items():
            bad = copy.deepcopy(proof)
            change(bad)
            with self.subTest(defect=label), self.assertRaisesRegex(ValueError, 'provider setup proof'):
                verify_provider_setup(bad)
        with self.assertRaises(ValueError):
            verify_provider_setup(None)

    def connected_app_fixture(self):
        from lib.package_gate5 import CONNECTED_APP_CHECKS, connected_app_inputs
        for name in ('connect', 'connect-example'):
            path = self.release / 'bin' / name
            if not path.exists():
                path.write_bytes(b'nonexecuted connected app verifier fixture')
        inputs = connected_app_inputs(self.release, self.source)
        return {'status': 'passed', 'profile': 'connected-app-public-cli/1',
            'source_revision': 'fixture-revision',
            'selected_binaries': {name: str((self.release / 'bin' / name).resolve()) for name in ('connect', 'connect-example')},
            'binary_sha256': {name: digest(self.release / 'bin' / name) for name in ('connect', 'connect-example')},
            'driver_sha256': digest(self.source / 'scripts/check-connected-app.py'),
            'input_sha256': inputs, 'input_sha256_after': dict(inputs), 'tested_inputs_stable': True,
            'checks': list(CONNECTED_APP_CHECKS), 'remaining_owned_processes': [],
            'not_run': ['test-support lost-ack crash injection', 'actual-model inference', 'whole-gate qualification'],
            'cleanup': [{'pid': pid, 'exit_code': code} for pid, code in ((1, 0), (2, 0), (3, 1))]}

    def test_connected_app_proof_requires_selected_binaries_source_and_all_behavior(self):
        import copy
        from lib.package_gate5 import verify_connected_app
        proof = self.connected_app_fixture()
        verify_connected_app(proof, self.release, self.source, 'fixture-revision')
        changes = {
            'missing behavior': lambda p: p['checks'].pop(),
            'duplicate behavior': lambda p: p['checks'].append(p['checks'][0]),
            'wrong revision': lambda p: p.__setitem__('source_revision', 'wrong'),
            'wrong binary': lambda p: p['binary_sha256'].__setitem__('connect-example', '0' * 64),
            'changed input': lambda p: p['input_sha256_after'].__setitem__(str((self.source / 'scripts/lib/local_process.py').resolve()), '0' * 64),
            'unstable inputs': lambda p: p.__setitem__('tested_inputs_stable', False),
            'leaked worker': lambda p: p.__setitem__('remaining_owned_processes', [123]),
            'missing exit': lambda p: p['cleanup'].pop(),
            'duplicate exit': lambda p: p['cleanup'].__setitem__(1, p['cleanup'][0]),
            'failed active worker': lambda p: p['cleanup'][0].__setitem__('exit_code', 1),
            'cleanup failure': lambda p: p.__setitem__('cleanup_errors', [{'pid': 1, 'error_class': 'TimeoutError'}]),
            'unaccounted omission': lambda p: p['not_run'].append('restart replay'),
        }
        for label, change in changes.items():
            bad = copy.deepcopy(proof)
            change(bad)
            with self.subTest(defect=label), self.assertRaisesRegex(ValueError, 'connected-app proof'):
                verify_connected_app(bad, self.release, self.source, 'fixture-revision')

    def test_independent_pass_for_another_binary_rejected(self):
        with self.assertRaisesRegex(ValueError, 'candidate inputs'):
            self.installed_hook(independent=True, wrong_binary=True)
        saved = json.loads((self.base / 'qualification/result.json').read_text())
        self.assertEqual(saved['status'], 'failed')
        self.assertFalse(saved['whole_gate_qualified'])


    def session_controls_fixture(self):
        from lib.package_gate5 import session_inputs
        for path in (self.release / 'bin/connect', self.base / 'selected-tool'):
            if not path.exists():
                path.write_bytes(b'nonexecuted proof guard input')
        inputs = session_inputs(self.release, self.source, self.base / 'selected-tool')
        return {'status': 'passed', 'profile': 'installed-session-controls/1', 'source_revision': 'fixture-revision',
            'selected_binary': str((self.release / 'bin/connect').resolve()), 'selected_assets': str(self.release.resolve()),
            'input_sha256': inputs, 'input_sha256_after': dict(inputs), 'tested_inputs_stable': True,
            'interfaces': ['CLI', 'Python native helper', 'Python HTTP', 'TypeScript native helper', 'TypeScript HTTP'],
            'cli_seed_records': 19, 'original_scope_records': 35, 'sdk_origin_records': 16,
            'narrowed_scope_denials': 26, 'current_authorization_enforced_on_replay': True,
            'credential_diagnostics_redacted': True,
            'sdk_phases': [{'name': f'session-{language}-{index}', 'status': 'passed',
                'leftover_group_members': [], 'group_cleanup': 'signalled'} for index, language in
                enumerate(('python', 'typescript', 'python', 'typescript', 'python', 'typescript'), 1)]}

    def test_installed_session_proof_requires_sdk_origins_and_replay_authorization(self):
        import copy
        from lib.package_gate5 import verify_session_controls
        proof = self.session_controls_fixture()
        verify_session_controls(proof, self.release, self.source, self.base / 'selected-tool', 'fixture-revision')
        changes = {
            'wrong binary': lambda p: p.__setitem__('selected_binary', str(self.base / 'foreign')),
            'source SDK fallback': lambda p: p.__setitem__('selected_assets', str(self.source)),
            'missing transport': lambda p: p['interfaces'].pop(),
            'missing SDK origin': lambda p: p.__setitem__('sdk_origin_records', 0),
            'partial denial': lambda p: p.__setitem__('narrowed_scope_denials', 25),
            'replay authorization': lambda p: p.__setitem__('current_authorization_enforced_on_replay', False),
            'changed SDK': lambda p: p['input_sha256_after'].clear(),
            'missing SDK phase': lambda p: p['sdk_phases'].pop(),
            'leaked SDK helper': lambda p: p['sdk_phases'][0].__setitem__('leftover_group_members', [123]),
        }
        for label, change in changes.items():
            bad = copy.deepcopy(proof)
            change(bad)
            with self.subTest(defect=label), self.assertRaisesRegex(ValueError, 'Installed Session controls'):
                verify_session_controls(bad, self.release, self.source, self.base / 'selected-tool', 'fixture-revision')

    def reference_continuity_fixture(self):
        return {'status': 'passed', 'conversation_id': 'conversation', 'session_id': 'session',
            'source_message_id': 'fact', 'decision_message_id': 'decision', 'decision_actor_id': 'actor',
            'followup_message_id': 'followup', 'artifact_id': 'artifact', 'artifact_sha256': 'a' * 64,
            'artifact_size': 100, 'artifact_bytes_reverified_after_restart': True, 'explicit_omitted_sources': 1,
            'exact_decision_replay': True, 'one_followup': True, 'foreign_conversation_sources': 0,
            'transcript_status': 'No transcript or derived summary was produced; AV transport supplies no ASR or model vision.'}

    def test_reference_continuity_proof_requires_attribution_artifact_and_replay(self):
        import copy
        from lib.package_gate5 import verify_reference_continuity
        proof = self.reference_continuity_fixture()
        verify_reference_continuity(proof)
        changes = {
            'missing actor': lambda p: p.pop('decision_actor_id'),
            'same followup': lambda p: p.__setitem__('followup_message_id', p['decision_message_id']),
            'invalid artifact digest': lambda p: p.__setitem__('artifact_sha256', 'wrong'),
            'unverified bytes': lambda p: p.__setitem__('artifact_bytes_reverified_after_restart', False),
            'missing omission': lambda p: p.__setitem__('explicit_omitted_sources', 0),
            'foreign context': lambda p: p.__setitem__('foreign_conversation_sources', 1),
            'lost replay': lambda p: p.__setitem__('exact_decision_replay', False),
            'repeated followup': lambda p: p.__setitem__('one_followup', False),
            'invented transcript': lambda p: p.__setitem__('transcript_status', 'transcript generated'),
        }
        for label, change in changes.items():
            bad = copy.deepcopy(proof)
            change(bad)
            with self.subTest(defect=label), self.assertRaisesRegex(ValueError, 'reference continuity'):
                verify_reference_continuity(bad)
        with self.assertRaises(ValueError):
            verify_reference_continuity(None)


if __name__ == '__main__':
    unittest.main()
