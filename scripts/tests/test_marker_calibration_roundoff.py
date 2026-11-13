import importlib.util
import json
from pathlib import Path
import unittest

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location('actual_marker_roundoff', HERE.parent / 'video/marker_calibration.py')
MARKER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MARKER)
ACTUAL = json.loads((HERE / 'av_skew_roundoff_observed.json').read_text())
SCHEDULE = [frame / 30 for frame in MARKER.marker_frames() if 1 <= frame / 30 < 60]

class MarkerCalibrationRoundoffTests(unittest.TestCase):
    def test_actual_observed_boundary_keeps_original_pairs_and_edges(self):
        before = json.dumps(ACTUAL, sort_keys=True)
        result = MARKER.ordinal_sync(ACTUAL['video'], ACTUAL['audio'], 60)
        self.assertEqual(result['status'], 'passed')
        self.assertEqual(result['p95_abs_skew_ms'], 120)
        self.assertEqual(result['quantile_roundoff']['raw_p95_abs_skew_ms'], 120.000000000001)
        self.assertTrue(result['quantile_roundoff']['p95_normalized'])
        self.assertEqual(result['marker_pairs'], ACTUAL['raw_pairs'])
        self.assertEqual(result['video_edge_seconds'], ACTUAL['video'])
        self.assertEqual(result['audio_edge_seconds'], ACTUAL['audio'])
        self.assertEqual(json.dumps(ACTUAL, sort_keys=True), before)

    def test_p95_meaningful_excesses_remain_failed(self):
        def observed(offset):
            return MARKER.ordinal_sync(SCHEDULE, [at + (offset if i % 10 == 0 else .03)
                for i, at in enumerate(SCHEDULE)], 60)
        self.assertEqual(observed(.12)['status'], 'passed')
        for offset in [.12000001, .123, .1241]:
            result = observed(offset)
            self.assertEqual(result['status'], 'failed')
            self.assertGreater(result['p95_abs_skew_ms'], 120)
            self.assertFalse(result['quantile_roundoff']['p95_normalized'])

    def test_median_meaningful_excesses_remain_failed(self):
        self.assertEqual(MARKER.ordinal_sync(SCHEDULE, [at + .08 for at in SCHEDULE], 60)['status'], 'passed')
        for offset in [.08000000001, .08000001]:
            result = MARKER.ordinal_sync(SCHEDULE, [at + offset for at in SCHEDULE], 60)
            self.assertEqual(result['status'], 'failed')
            self.assertGreater(result['median_abs_skew_ms'], 80)
            self.assertFalse(result['quantile_roundoff']['median_normalized'])

    def test_no_new_proof_or_absolute_clock_allowance(self):
        self.assertEqual(MARKER.ordinal_sync([1, 2], [1, 2], 60)['status'], 'failed')
        self.assertEqual(MARKER.ordinal_sync(SCHEDULE, [float('nan')] * len(SCHEDULE), 60)['status'], 'failed')
        result = MARKER.ordinal_sync([at + 1000 for at in SCHEDULE], [at + 1000 + .12 for at in SCHEDULE], 60)
        self.assertEqual(result['quantile_roundoff']['p95_error_bound_ms'], 0)
        self.assertFalse(result['quantile_roundoff']['p95_normalized'])
