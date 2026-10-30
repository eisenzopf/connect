"""A source-bound browser producer must keep every hot runtime library at opt3."""
from pathlib import Path
import json
import runpy
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

CHECK = runpy.run_path(str(Path(__file__).resolve().parents[1] / 'check-gate6-browser-media.py'))
LIBRARIES = CHECK['RUNTIME_LIBRARIES']
profiles = CHECK['runtime_library_profiles']


def artifact(name, level='3', kind=None):
    return {'reason':'compiler-artifact','target':{'name':name,'kind':['lib'] if kind is None else kind},
            'profile':{'opt_level':level},'package_id':'private source path','filenames':['private executable path']}


class RuntimeBuildProfiles(unittest.TestCase):
    def test_every_required_runtime_library_must_be_present_at_opt3(self):
        result = profiles([artifact(name) for name in LIBRARIES])
        self.assertTrue(result['passed'])
        self.assertEqual(result['missing'], [])
        self.assertEqual(result['completed_artifacts'], len(LIBRARIES))

    def test_local_opt0_exceptions_cannot_qualify_linux_production_libraries(self):
        for name in LIBRARIES:
            result = profiles([artifact(other, '0' if other == name else '3') for other in LIBRARIES])
            self.assertFalse(result['passed'])

    def test_missing_library_or_non_library_target_cannot_count(self):
        self.assertFalse(profiles([])['passed'])
        self.assertFalse(profiles([artifact(name, kind=['custom-build']) for name in LIBRARIES])['passed'])

    def test_compiler_paths_and_unknown_labels_never_enter_progress(self):
        result = profiles([artifact('private-secret'),artifact('connect_service','private-secret')])
        self.assertNotIn('private', repr(result))
        self.assertEqual(result['profiles'], {'connect_service':['invalid']})
        self.assertEqual(set(result), {'completed_artifacts','profiles','missing','passed'})

    def test_conflicting_profiles_cannot_be_hidden_by_a_later_artifact(self):
        result = profiles([artifact(name) for name in LIBRARIES] + [artifact('connect_service','0')])
        self.assertFalse(result['passed'])
        self.assertEqual(result['profiles']['connect_service'], ['0','3'])

    def failed_build(self, changed):
        root = Path(__file__).resolve().parents[2]
        parent = root / '.local/gate-6/browser-build-cache-tests'
        parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=parent) as directory:
            directory = Path(directory)
            manifest = directory / 'cache/source-times.json'
            snapshot = {'source.rs':{'sha256':'a' * 64,'mode':420,'mtime_ns':123}}
            observations = iter([{'source.rs':'a' * 64},{'source.rs':('b' if changed else 'a') * 64}])

            def timeout(command, environment, log, budget):
                self.assertEqual(budget, 3600)
                self.assertEqual(command[-2:], ['-C','opt-level=0'])
                self.assertLess(command.index('--'), len(command) - 2)
                self.assertNotIn('--config', command)
                self.assertIn('--no-default-features', command)
                log.write_text('')
                return {'exit_code':-15,'timed_out':True,'forced_kill':False,'seconds':3600}

            def cache(path):
                self.assertEqual(Path(path).name, 'source_build_cache.py')
                return {'MANIFEST':manifest,'snapshot':lambda actual_root:snapshot}

            build = CHECK['build']
            with patch.dict(build.__globals__, {'ROOT':directory,'BASE':directory / 'run',
                    'INPUTS':lambda:next(observations),'BOUNDED':timeout,
                    'subprocess':SimpleNamespace(run=lambda *args,**kwargs:None),
                    'runpy':SimpleNamespace(run_path=cache)}), patch.dict('os.environ', {'GITHUB_OUTPUT':'','CONNECT_GATE6_FIXTURE_OPT_LEVEL':'0'}):
                with self.assertRaisesRegex(AssertionError, 'producer failed'):
                    build()
            report = json.loads((directory / 'run/evidence/build/summary.json').read_text())
            self.assertFalse(report['passed'])
            self.assertEqual(report['artifacts'], [])
            self.assertEqual(report['sources_unchanged'], not changed)
            self.assertEqual(report['fixture_opt_level'], '0')
            self.assertEqual(manifest.exists(), not changed)
            if not changed:
                self.assertEqual(json.loads(manifest.read_text()), {'format':1,'files':snapshot})

    def test_timed_out_build_preserves_only_verified_unchanged_source_times(self):
        self.failed_build(False)

    def test_changed_source_never_seals_partial_build_cache_times(self):
        self.failed_build(True)


if __name__ == '__main__':
    unittest.main()
