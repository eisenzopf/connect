"""Actual request-driven encoding and independent decoding; no transport claim."""
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from lib.package_environment import fixture_environment
from video.build_live_vp8_encoder import build


def packets(wire):
    result = []
    while wire:
        if len(wire) < 16:
            raise ValueError("truncated encoder reply")
        magic, index, size, key = struct.unpack("<4sIII", wire[:16])
        if magic != b"V8F1" or key not in (0, 1) or not 1 <= size <= 256 * 1024:
            raise ValueError("invalid encoder reply")
        if len(wire) < 16 + size:
            raise ValueError("truncated VP8 frame")
        frame, wire = wire[16:16 + size], wire[16 + size:]
        if (frame[0] & 1 == 0) != bool(key):
            raise ValueError("encoder key flag disagrees with actual VP8 bytes")
        result.append((index, key, frame))
    return result


def independent_decode(directory, name, frames):
    path = directory / (name + ".ivf")
    header = struct.pack("<4sHH4sHHIIII", b"DKIF", 0, 32, b"VP80", 640, 360,
                         30, 1, len(frames), 0)
    path.write_bytes(header + b"".join(struct.pack("<IQ", len(frame), index) + frame
                                     for index, _, frame in frames))
    path.chmod(0o600)
    decoded = subprocess.run([
        "ffmpeg", "-nostdin", "-hide_banner", "-loglevel", "error", "-i", str(path),
        "-vf", "crop=16:16:0:0", "-pix_fmt", "yuv420p", "-fps_mode", "passthrough",
        "-frames:v", str(len(frames)), "-f", "rawvideo", "pipe:1"],
        capture_output=True, check=True, timeout=10, env=fixture_environment()).stdout
    if len(decoded) != len(frames) * 384:
        raise AssertionError("independent decoder did not produce every original frame")
    actual_luma_sums = []
    for position, (index, _, _) in enumerate(frames):
        luma = decoded[position * 384:position * 384 + 256]
        actual_luma_sums.append(sum(luma))
        if abs(sum(luma) / 256 - (60 + 3 * index)) > 3:
            raise AssertionError("decoded marker belongs to another original frame")
    return actual_luma_sums


class LiveVp8Encoder(unittest.TestCase):
    def test_fresh_requested_frame_original_clock_gop_and_closed_input(self):
        self.assertEqual(sys.platform, "linux", "Qualification runs on local ARM Linux")
        import platform
        self.assertEqual(platform.machine(), "aarch64")
        private = ROOT / ".local/gate-6/live-vp8-source"
        private.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=private) as temporary:
            directory = Path(temporary)
            binary = directory / "encoder"
            metadata = build(binary)
            raw = directory / "original.i420"
            with raw.open("xb") as stream:
                for index in range(40):
                    plane = bytearray([96]) * (640 * 360)
                    for row in range(80):
                        plane[row * 640:row * 640 + 80] = bytes([60 + 3 * index]) * 80
                    stream.write(plane)
                    stream.write(bytes([128]) * (640 * 360 // 2))
            raw.chmod(0o600)

            def run(requests, source=raw):
                # subprocess.run kills and joins a timed-out child. Every helper
                # here has an independent finite lifetime and bounded output.
                return subprocess.run([str(binary), str(source), "40"], input=requests,
                                      capture_output=True, timeout=5,
                                      env=fixture_environment())

            ordinary = run(b"".join(struct.pack("<II", i, 0) for i in range(40)))
            requested = run(b"".join(struct.pack("<II", i, int(i == 7)) for i in range(40)))
            self.assertEqual(ordinary.returncode, 0, ordinary.stderr)
            self.assertEqual(requested.returncode, 0, requested.stderr)
            baseline, recovered = packets(ordinary.stdout), packets(requested.stdout)
            self.assertEqual([p[0] for p in recovered], list(range(40)))
            self.assertEqual(baseline[7][1], 0, "counterfactual unexpectedly already had a keyframe")
            self.assertEqual(recovered[7][1], 1)
            for stream in (baseline, recovered):
                last_key = None
                for index, key, _ in stream:
                    if key:
                        last_key = index
                    self.assertIsNotNone(last_key)
                    self.assertLess(index - last_key, 30)
            # The same original decoder owns all frames across a marked
            # conceptual preparation/commit boundary. No restart at index 7.
            def decode(frames, suffix=b""):
                wire = b"".join(struct.pack("<4sIII", b"D8F1", i, len(frame), key) + frame
                                for i, key, frame in frames) + suffix
                return subprocess.run([str(binary), "--decode-vp8"], input=wire,
                                      capture_output=True, timeout=5,
                                      env=fixture_environment())
            decoded = decode(recovered)
            self.assertEqual(decoded.returncode, 0, decoded.stderr)
            self.assertEqual(len(decoded.stdout), len(recovered) * 32)
            retained_luma_sums = []
            for position, (index, key, _) in enumerate(recovered):
                magic, actual, count, actual_key, width, height, luma, pixels = struct.unpack(
                    "<4sIIIIIII", decoded.stdout[position*32:(position+1)*32])
                self.assertEqual((magic, actual, count, actual_key, width, height, pixels),
                                 (b"V8D1", index, 1, key, 640, 360, 256))
                self.assertLessEqual(abs(luma / pixels - (60 + 3 * index)), 3)
                retained_luma_sums.append(luma)
            # A valid interframe is not startup readiness; malformed, duplicate
            # and falsely labelled keyframes cannot become decoder output.
            self.assertNotEqual(decode(baseline[1:2]).returncode, 0)
            self.assertNotEqual(decode(recovered[:1] * 2).returncode, 0)
            self.assertNotEqual(decode([(0, 1, b"\0" * 16)]).returncode, 0)
            self.assertNotEqual(decode([(0, 0, recovered[0][2])]).returncode, 0)
            self.assertNotEqual(decode(recovered[:1], b"partial").returncode, 0)
            independent_luma_sums = independent_decode(directory, "requested-whole", recovered)
            self.assertEqual(retained_luma_sums, independent_luma_sums,
                             "retained libvpx and independent offline decoder must observe the same original pixels")
            # Start a separate decoder at the requested physical frame. This
            # must decode its own original marker, not a cached earlier image.
            independent_decode(directory, "requested-fresh-history", recovered[7:])
            skipped = run(struct.pack("<II", 7, 1) + struct.pack("<II", 39, 0))
            self.assertEqual(skipped.returncode, 0, skipped.stderr)
            self.assertEqual([(p[0], p[1]) for p in packets(skipped.stdout)], [(7, 1), (39, 1)])
            for bad in (b"x", struct.pack("<II", 40, 0), struct.pack("<II", 1, 2),
                        struct.pack("<II", 1, 0) * 2,
                        struct.pack("<II", 7, 0) + struct.pack("<II", 6, 0)):
                self.assertNotEqual(run(bad).returncode, 0)
            link = directory / "link.i420"
            link.symlink_to(raw)
            self.assertNotEqual(run(struct.pack("<II", 0, 0), link).returncode, 0)
            raw.chmod(0o644)
            self.assertNotEqual(run(struct.pack("<II", 0, 0)).returncode, 0)
            print(json.dumps(dict(build=metadata, independently_decoded_frames=73,
                                  requested_keyframe_index=7, original_indices_preserved=True,
                                  actual_retained_decoder_outputs=40, independent_pixel_comparison=True,
                                  transport_qualified=False, full_media_qualified=False)))


if __name__ == "__main__":
    unittest.main()
