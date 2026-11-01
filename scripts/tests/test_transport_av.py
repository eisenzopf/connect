"""Independent codec calibration and corrupt-container refusals, not transport proof."""
import base64
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from lib.package_environment import fixture_environment
from lib.transport_av import opus_packets, decode_audio, ivf_frames


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


if __name__ == '__main__':
    unittest.main()
