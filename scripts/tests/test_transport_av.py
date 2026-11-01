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
from lib.transport_av import opus_packets, decode_audio, ivf_frames, decode_completed, rtp_timing, source_pixels


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
