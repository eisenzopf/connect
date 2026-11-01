import math
import unittest
from lib.transport_replacement_audio import classify_pcm, replacement_arrival_observation


class ReplacementAudioTests(unittest.TestCase):
    def test_original_packet_identity_preserves_timestamps_and_arrival_order(self):
        frequencies = (440, 660, 880)
        samples = [0.25 * math.sin(2 * math.pi * hz * index / 48000)
            for hz in frequencies for index in range(960)]
        rows = [dict(elapsed_ms=100 + index * 20, timestamp=0xffffffff - index) for index in range(3)]
        labels = classify_pcm(samples, rows)
        self.assertEqual([row['label_hz'] for row in labels], list(frequencies))
        self.assertEqual([row['timestamp'] for row in labels], [row['timestamp'] for row in rows])
        with self.assertRaises(ValueError):
            classify_pcm(samples[:-1], rows)
        with self.assertRaises(ValueError):
            classify_pcm([float('nan')] + samples[1:], rows)
        rows[1]['elapsed_ms'] = 1
        with self.assertRaises(ValueError):
            classify_pcm(samples, rows)

    def fixture(self):
        row = lambda hz, at: dict(label_hz=hz, elapsed_ms=at, timestamp=at * 48)
        return dict(original=[row(880, i * 20) for i in range(25)],
            retained=[row(440, i * 20) for i in range(25)] + [row(660, 560 + i * 20) for i in range(350)],
            target=[row(880, i * 20) for i in range(350)])

    def test_one_original_arrival_observation_does_not_claim_device_or_full_profile_qualification(self):
        result = replacement_arrival_observation(self.fixture())
        self.assertEqual(result['decoded_packet_arrival_gap_ms'], 60)
        self.assertFalse(result['device_presentation_measured'])
        self.assertFalse(result['full_media_quality_qualified'])

    def test_old_speaker_replay_self_audio_and_missing_destination_decode_refuse(self):
        for change in [lambda value: value['retained'].append(dict(label_hz=440, elapsed_ms=8000)),
                lambda value: value['target'].append(dict(label_hz=660, elapsed_ms=8000)),
                lambda value: value.update(target=value['target'][:200]),
                lambda value: value['retained'].insert(25, dict(label_hz=880, elapsed_ms=500))]:
            value = self.fixture()
            change(value)
            with self.assertRaises(ValueError):
                replacement_arrival_observation(value)

    def test_a_gap_over_the_original_one_second_bound_refuses(self):
        value = self.fixture()
        for row in value['retained']:
            if row['label_hz'] == 660:
                row['elapsed_ms'] += 1000
        with self.assertRaises(ValueError):
            replacement_arrival_observation(value)
