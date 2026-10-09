"""Known G.711 vectors for the optional native microphone sample."""
import importlib.util
from pathlib import Path
import struct
import unittest

source = Path(__file__).resolve().parents[2] / 'examples/conversation-client/pcmu.py'
spec = importlib.util.spec_from_file_location('connect_example_pcmu', source)
codec = importlib.util.module_from_spec(spec)
spec.loader.exec_module(codec)


class MicrophoneCodecTests(unittest.TestCase):
    def test_silence_sign_and_full_scale_vectors(self):
        samples = (0, 1000, -1000, 32124, -32124)
        self.assertEqual(codec.encode(struct.pack('<5h', *samples)), bytes.fromhex('ff ce 4e 80 00'))
        self.assertEqual(struct.unpack('<5h', codec.decode(bytes.fromhex('ff ce 4e 80 00'))),
                         (0, 988, -988, 32124, -32124))

    def test_extreme_samples_and_twenty_millisecond_frame(self):
        self.assertEqual(codec.encode(struct.pack('<2h', -32768, 32767)), bytes.fromhex('00 80'))
        self.assertEqual(codec.encode(bytes(320)), bytes([255]) * 160)
        self.assertEqual(codec.decode(bytes([255]) * 160), bytes(320))
