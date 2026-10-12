"""Full retirement evidence cannot be replaced by partial load or missing RSS."""
import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.gate4_phases import phases
from lib.qualification_report import IGNORED
from lib.voice_retirement_report import TEST, validate


def complete():
    stages = [('baseline', 0, 0)]
    for start in range(0, 10_000, 32):
        count = min(32, 10_000-start)
        stages += [('ready', start+count, count), ('retired', start+count, 0)]
    stages += [('restarted', 10_000, 0)]
    return {'status': 'passed', 'target_setups': 10_000, 'listener_limit': 32,
            'setup_lifetime_seconds': 5, 'process_rss_scope': 'whole fixture process',
            'observations': [{'stage': stage, 'completed_setups': count, 'live_listeners': live,
                              'bindings': count, 'distinct_nonces': count, 'unknown_bindings': 0,
                              'process_rss_bytes': 1024, 'database_apparent_bytes': 4096,
                              'elapsed_seconds': index, 'binding_digest': 'a'*64}
                             for index, (stage, count, live) in enumerate(stages)]}


LOG = f'test {TEST} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored;\n'


class RetirementEvidenceTests(unittest.TestCase):
    def test_full_profile_has_every_batch_and_restart(self):
        summary = validate(complete(), LOG)
        self.assertEqual((summary['retired_setups'], summary['batches'], summary['point_samples']), (10_000, 313, 628))

    def test_partial_or_skipped_rust_execution_cannot_pass(self):
        for bad in ['', LOG.replace('1 passed', '0 passed'), LOG.replace('0 ignored', '1 ignored')]:
            with self.assertRaises(AssertionError):
                validate(complete(), bad)
        data = complete()
        data['observations'].pop(42)
        with self.assertRaisesRegex(AssertionError, 'Missing batch'):
            validate(data, LOG)

    def test_changed_bindings_or_missing_resource_samples_are_not_passes(self):
        for field, value in [('binding_digest', 'b'*64), ('distinct_nonces', 9999),
                             ('unknown_bindings', 1), ('process_rss_bytes', 0),
                             ('database_apparent_bytes', 0), ('elapsed_seconds', float('nan'))]:
            data = copy.deepcopy(complete())
            data['observations'][-1][field] = value
            with self.subTest(field=field), self.assertRaises(AssertionError):
                validate(data, LOG)

    def test_both_runner_modes_have_one_explicit_ignored_test_followup(self):
        self.assertIn(TEST, IGNORED)
        for source_only in (False, True):
            names = [p[0] for p in phases(source_only)]
            self.assertEqual(names.count('voice-setup-retirement'), 1)
            self.assertLess(names.index('workspace'), names.index('voice-setup-retirement'))


if __name__ == '__main__':
    unittest.main()
