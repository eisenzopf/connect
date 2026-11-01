"""Independent codec calibration and corrupt-container refusals, not transport proof."""
import base64
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from lib.package_environment import fixture_environment
from lib.transport_av import (opus_packets, decode_audio, ivf_frames, decode_completed,
    rtp_timing, source_pixels, clock_observation, submission_observation)


@unittest.skipUnless(shutil.which('ffmpeg'), 'Independent FFmpeg calibration requires the named prerequisite')
class NativeCodecCalibration(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.directory = Path(self.temp.name)
        self.path = self.directory / 'known.ogg'
        subprocess.run(['ffmpeg', '-nostdin', '-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i',
            'sine=frequency=440:sample_rate=48000:duration=1', '-ac', '1', '-c:a', 'libopus', '-b:a', '32k',
            '-vbr', 'off', '-frame_duration', '20', str(self.path)], check=True, timeout=10, env=fixture_environment())
        self.body = self.path.read_bytes()

    def tearDown(self):
        self.temp.cleanup()

    def test_real_ffmpeg_packets_decode_and_wrong_speaker_is_rejected(self):
        packets = opus_packets(self.body)
        self.assertEqual(len(packets), 51)
        rows = [{'payload': base64.b64encode(p).decode()} for p in packets]
        expected = decode_audio(self.directory, 0, rows, 440)
        wrong = decode_audio(self.directory, 1, rows, 600)
        self.assertTrue(expected['all_blocks_identified'])
        self.assertFalse(wrong['all_blocks_identified'])
        self.assertGreater(wrong['bad_identity_blocks'], 0)
        self.assertGreaterEqual(expected['decoded_seconds'], 1)

    def test_truncation_crc_change_and_chained_stream_are_refused(self):
        changed = bytearray(self.body); changed[-3] ^= 1
        for body in [self.body[:-1], bytes(changed), self.body + self.body]:
            with self.subTest(size=len(body)):
                with self.assertRaises(ValueError): opus_packets(body)

    def test_other_video_size_is_refused_before_frame_collection(self):
        import struct
        for width, height in [(320, 180), (640, 480), (1280, 720)]:
            body = struct.pack('<4sHH4sHHIIII', b'DKIF', 0, 32, b'VP80', width, height, 30, 1, 0, 0)
            with self.assertRaises(ValueError): ivf_frames(body)


class CompletedObservationRetention(unittest.TestCase):
    def test_late_control_failure_keeps_completed_decode_and_failed_control_result(self):
        control = {'passed': False, 'failure': {'phase': 'original-receipt-recovery'},
            'native_av': {'original_receive_tasks_joined': True}}
        decoded = {'passed': False, 'scope': 'A distinct decoded media failure'}
        with patch('lib.transport_av.verify', return_value=decoded) as decoder:
            self.assertIs(decode_completed(control, Path('private-observation'), {'sources': 6}), decoded)
            decoder.assert_called_once_with(Path('private-observation'), {'sources': 6})
        self.assertFalse(control['passed'])
        self.assertEqual(control['failure']['phase'], 'original-receipt-recovery')

    def test_unfinished_media_does_not_become_a_decode_or_runtime_pass(self):
        for control in [{'passed': False}, {'passed': False, 'native_av': {'original_receive_tasks_joined': False}}]:
            with patch('lib.transport_av.verify') as decoder:
                self.assertIsNone(decode_completed(control, Path('private-observation'), {}))
                decoder.assert_not_called()
            self.assertFalse(control['passed'])


class NativeObservationDiagnostics(unittest.TestCase):
    def test_exact_u64_clock_fraction_and_rtp_wrap_map_original_arrival_age(self):
        # An NTP fraction just below one second must not round into the next
        # second before subtraction. RTP crosses wrap on the second packet.
        origin = (2208988800 + 1700000000) << 32
        report = dict(elapsed_ms=1000, ssrc=7, ntp=str(origin + 2 ** 32 - 1),
            rtp=2 ** 32 - 960, packets=1, octets=80)
        rows = [dict(elapsed_ms=1025, ssrc=7, timestamp=2 ** 32 - 960),
            dict(elapsed_ms=1045, ssrc=7, timestamp=0)]
        observed = clock_observation(rows, [report], 48000, 1700000000000)
        ages = observed['packet_arrival_minus_reported_source_time']
        self.assertEqual(ages['count'], 2)
        self.assertAlmostEqual(ages['minimum_ms'], 25 + 1000 / 2 ** 32, places=8)
        self.assertEqual(ages['minimum_ms'], ages['maximum_ms'])

    def test_future_foreign_and_stale_clock_reports_do_not_invent_age(self):
        origin = 2208988800 << 32
        reports = [dict(elapsed_ms=100, ssrc=7, ntp=str(origin), rtp=0, packets=0, octets=0),
            dict(elapsed_ms=200, ssrc=8, ntp=str(origin), rtp=0, packets=0, octets=0)]
        rows = [dict(elapsed_ms=50, ssrc=7, timestamp=0),
            dict(elapsed_ms=150, ssrc=8, timestamp=0),
            dict(elapsed_ms=250, ssrc=9, timestamp=0),
            dict(elapsed_ms=300, ssrc=7, timestamp=3 * 48000),
            dict(elapsed_ms=2201, ssrc=8, timestamp=0)]
        result = clock_observation(rows, reports, 48000, 0)
        self.assertEqual(result['no_preceding_same_ssrc_anchor'], 3)
        self.assertEqual(result['stale_anchor_or_rtp_distance'], 2)
        self.assertEqual(result['packet_arrival_minus_reported_source_time']['count'], 0)
        self.assertIsNone(result['packet_arrival_minus_reported_source_time']['median_ms'])
        # A previous-source report cannot replace a usable same-source anchor.
        mapped = clock_observation([dict(elapsed_ms=250, ssrc=7, timestamp=960)], reports, 48000, 0)
        self.assertEqual(mapped['packet_arrival_minus_reported_source_time']['median_ms'], 230)

    def test_malformed_clock_identity_and_original_report_reordering_are_refused(self):
        good = dict(elapsed_ms=100, ssrc=7, ntp=str(2208988800 << 32), rtp=0, packets=0, octets=0)
        for changed in [dict(ntp=2208988800 << 32), dict(ntp='01'), dict(ntp=str(2 ** 64)),
                dict(ntp='-1'), dict(ssrc=2 ** 32), dict(rtp=-1), dict(elapsed_ms=float('nan'))]:
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                clock_observation([], [dict(good, **changed)], 48000, 0)
        with self.assertRaises(ValueError):
            clock_observation([], [dict(good, elapsed_ms=101), good], 48000, 0)

    def test_source_receipt_observation_keeps_queue_duration_separate_from_source_age(self):
        result = submission_observation([dict(source_offset_ms=20, submit_started_ms=21, submit_completed_ms=24),
            dict(source_offset_ms=40, submit_started_ms=60, submit_completed_ms=62)])
        self.assertEqual(result['original_queue_submission_duration']['maximum_ms'], 3)
        self.assertEqual(result['original_source_age_at_queue_receipt']['maximum_ms'], 22)
        with self.assertRaises(ValueError):
            submission_observation([dict(source_offset_ms=0, submit_started_ms=2, submit_completed_ms=1)])
        with self.assertRaises(ValueError):
            submission_observation([dict(source_offset_ms=0, submit_started_ms=0, submit_completed_ms=float('nan'))])

    def test_original_rtp_wrap_loss_and_slow_clock_remain_distinct(self):
        rows = [dict(elapsed_ms=0, timestamp=2 ** 32 - 960, sequence=65535),
            dict(elapsed_ms=20, timestamp=0, sequence=0),
            dict(elapsed_ms=60, timestamp=1920, sequence=2)]
        missing = rtp_timing(rows, 48000, 960)
        self.assertEqual(missing['missing_timestamp_steps'], 1)
        self.assertEqual(missing['missing_rtp_sequence_packets'], 1)
        self.assertEqual(missing['rtp_minus_arrival_span_ms'], 0)
        self.assertEqual(missing['backwards_timestamps'], 0)
        slow = rtp_timing([dict(elapsed_ms=i * 40, timestamp=i * 960, sequence=i) for i in range(4)], 48000, 960)
        self.assertEqual(slow['missing_timestamp_steps'], 0)
        self.assertEqual(slow['missing_rtp_sequence_packets'], 0)
        self.assertEqual(slow['rtp_minus_arrival_span_ms'], -60)
        self.assertEqual(slow['minimum_progress_offset_ms'], -60)

    def test_other_source_marker_is_not_conflated_with_unknown_corruption(self):
        result = source_pixels(bytes([255, 0, 0, 0, 255, 0, 80, 80, 80]), 0)
        self.assertEqual(result['expected_source_frames'], 1)
        self.assertEqual(result['other_identified_source_frames'], 1)
        self.assertEqual(result['unclassified_pixel_frames'], 1)
        self.assertEqual(result['wrong_source_pixel_frames'], 2)
        with self.assertRaises(ValueError): source_pixels(bytes([255]), 0)


if __name__ == '__main__':
    unittest.main()
