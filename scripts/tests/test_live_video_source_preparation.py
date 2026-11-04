"""Small source-generation check, not the frozen 62-second media workload."""
import json
import os
from pathlib import Path
import platform
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib import transport_av


class LiveSourcePreparation(unittest.TestCase):
    def test_raw_and_reference_share_markers_and_private_finite_manifest(self):
        self.assertEqual(platform.system(), 'Linux')
        self.assertEqual(platform.machine(), 'aarch64')
        parent = Path('.local/gate-6/live-source-preparation')
        parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        with tempfile.TemporaryDirectory(dir=parent) as temporary:
            directory = Path(temporary) / 'source'
            # Exercise the real dual-output FFmpeg command with one second of
            # one colour. Production constants and workload remain unchanged.
            with patch.multiple(transport_av, SECONDS=1, COLORS=('red',), FREQUENCIES=(440,)):
                manifest = transport_av.prepare(directory)
            raw = directory / 'source-0.i420'
            self.assertEqual(raw.stat().st_size, 30 * 640 * 360 * 3 // 2)
            self.assertEqual(raw.stat().st_mode & 0o777, 0o600)
            self.assertEqual(manifest['sources'][0]['raw_video_sha256'], transport_av.checksum(raw))
            self.assertEqual(manifest['live_encoder']['executable_sha256'],
                             transport_av.checksum(directory / 'live-vp8-encoder'))
            self.assertTrue(os.access(directory / 'live-vp8-encoder', os.X_OK))
            reference = directory / 'reference.i420'
            transport_av.command(['ffmpeg', '-nostdin', '-hide_banner', '-loglevel', 'error',
                '-i', str(directory / 'source-0.ivf'), '-pix_fmt', 'yuv420p', '-f', 'rawvideo', str(reference)],
                directory / 'decode-reference.log')
            source_bytes, decoded = raw.read_bytes(), reference.read_bytes()
            self.assertEqual(len(source_bytes), len(decoded))
            for frame in range(30):
                offset = frame * 640 * 360 * 3 // 2
                # Identity and sync marker interiors, across every frame.
                for x in (32, 128):
                    at = offset + 32 * 640 + x
                    self.assertLessEqual(abs(source_bytes[at] - decoded[at]), 3)
            self.assertEqual(len(json.loads((directory / 'source-0.json').read_text())['video']), 30)


if __name__ == '__main__':
    unittest.main()
