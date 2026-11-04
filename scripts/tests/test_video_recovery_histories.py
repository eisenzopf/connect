"""Independent decoder input grouping must not join unrelated video histories."""
import sys
from pathlib import Path
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.transport_moving_av import rows

def row(index, stream='camera', history=None):
    result = dict(elapsed_ms=index*34, timestamp=index*3000, ssrc=83, stream_id=stream, key=True, payload='AA==')
    if history is not None:
        result['decoder_history'] = history
    return result

class VideoRecoveryHistories(unittest.TestCase):
    def test_same_ssrc_epoch_and_loss_histories_are_independent_without_clock_reset(self):
        original = [row(0, history=1), row(1, history=1), row(2, history=2), row(5, history=3)]
        groups = rows(original, 'video', {'camera'})
        self.assertEqual([[r['timestamp'] for r in group] for _, _, group in groups], [[0, 3000], [6000], [15000]])
        self.assertEqual([r for _, _, group in groups for r in group], original)
        for bad in ([row(1, history=2), row(2, history=1)], [row(1, history=1), row(2, history=True)],
                    [row(1, history=1), {**row(2, history=2), 'timestamp': 0}]):
            with self.assertRaises(ValueError): rows(bad, 'video', {'camera'})

    def test_returning_to_a_legacy_camera_cannot_reuse_earlier_decoder_history(self):
        original = [row(0), row(1, 'other'), row(2)]
        self.assertEqual([len(group) for _, _, group in rows(original, 'video', {'camera', 'other'})], [1, 1, 1])

if __name__ == '__main__': unittest.main()
