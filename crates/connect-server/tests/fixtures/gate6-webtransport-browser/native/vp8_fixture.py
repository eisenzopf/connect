"""Bounded native VP8 fixture primitives; no network or interoperability claim."""
from pathlib import Path
import hashlib
import json
import struct
import subprocess

WIDTH, HEIGHT, FPS, COUNT = 640, 360, 30, 120
RTP_PAYLOAD = 1180 - 12 - 1


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def prepare(directory, ffmpeg='/opt/homebrew/bin/ffmpeg'):
    directory = Path(directory)
    directory.mkdir(exist_ok=False)
    output = directory / 'native-blue-vp8.ivf'
    frames = []
    for n in range(COUNT):
        frame = bytearray(bytes((20, 20, 220)) * (WIDTH * HEIGHT))
        x = (n * 11) % 608
        for y in range(80, 280):
            start = (y * WIDTH + x) * 3
            frame[start:start + 32 * 3] = bytes((10, 230, 10)) * 32
        frames.append(frame)
    command = [ffmpeg, '-hide_banner', '-loglevel', 'error', '-nostdin', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-s', '640x360', '-r', '30', '-i', 'pipe:0', '-an', '-c:v', 'libvpx', '-threads', '1', '-deadline', 'realtime', '-cpu-used', '8', '-b:v', '500k', '-maxrate', '1000k', '-bufsize', '1000k', '-g', '30', '-keyint_min', '30', '-pix_fmt', 'yuv420p', '-frames:v', str(COUNT), '-f', 'ivf', str(output)]
    with (directory / 'encode.log').open('wb') as log:
        subprocess.run(command, input=b''.join(frames), stderr=log, check=True, timeout=30)
    del frames
    blob = output.read_bytes()
    assert blob[:12] == b'DKIF\x00\x00\x20\x00VP80'
    assert struct.unpack_from('<HH', blob, 12) == (WIDTH, HEIGHT)
    rate, scale = struct.unpack_from('<II', blob, 16)
    assert rate / scale == FPS
    packets, keyframes, timestamps, offset = [], [], [], 32
    while offset < len(blob):
        size, timestamp = struct.unpack_from('<IQ', blob, offset)
        offset += 12
        assert 0 < size <= 256 * 1024
        frame = blob[offset:offset + size]
        assert len(frame) == size
        if frame[0] & 1 == 0:
            keyframes.append(len(packets))
        packets.append((size + RTP_PAYLOAD - 1) // RTP_PAYLOAD)
        timestamps.append(timestamp)
        offset += size
    assert offset == len(blob) and len(packets) == COUNT
    assert timestamps == list(range(COUNT))
    assert keyframes[0] == 0 and all(b - a <= FPS for a, b in zip(keyframes, keyframes[1:]))
    assert COUNT - keyframes[-1] <= FPS and max(packets) <= 64
    decoded = directory / 'independent-rgb.raw'
    decode = [ffmpeg, '-hide_banner', '-loglevel', 'error', '-nostdin', '-i', str(output), '-threads', '1', '-f', 'rawvideo', '-pix_fmt', 'rgb24', str(decoded)]
    with (directory / 'decode.log').open('wb') as log:
        subprocess.run(decode, stderr=log, check=True, timeout=30)
    size = WIDTH * HEIGHT * 3
    assert decoded.stat().st_size == COUNT * size
    identified, motion = 0, set()
    with decoded.open('rb') as source:
        for _ in range(COUNT):
            frame = source.read(size)
            pixel = frame[(10 * WIDTH + 10) * 3: (10 * WIDTH + 10) * 3 + 3]
            identified += pixel[2] > 150 and pixel[0] < 80
            row = frame[180 * WIDTH * 3:181 * WIDTH * 3]
            xs = [x for x in range(WIDTH) if row[x * 3 + 1] > 120 and row[x * 3 + 1] > row[x * 3] * 1.5 and row[x * 3 + 1] > row[x * 3 + 2] * 1.5]
            assert len(xs) > 8
            motion.add(round(sum(xs) / len(xs)))
    assert identified == COUNT and len(motion) >= 40
    result = dict(passed=True, scope='Native software VP8 source and independent decoded pixel fixture only. No RTP transport, public client, browser/native interoperability, playback timing or G6-050 completion claim.', frames=COUNT, fps=FPS, identified=identified, distinct_motion=len(motion), keyframe_indices=keyframes, max_fragments=max(packets), ivf_sha256=digest(output), decoded_sha256=digest(decoded), command=command, decoder_command=decode, ffmpeg_version=subprocess.check_output([ffmpeg,'-version'],text=True).splitlines()[0], helper_sha256=digest(Path(__file__)))
    (directory / 'summary.json').write_text(json.dumps(result,indent=2)+'\n')
    decoded.unlink()
    return result

if __name__ == '__main__':
    import sys
    print(json.dumps(prepare(sys.argv[1])), flush=True)
