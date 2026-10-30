"""A source-bound browser producer must keep every hot runtime library at opt3."""
from pathlib import Path
import runpy
import unittest

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


if __name__ == '__main__':
    unittest.main()
