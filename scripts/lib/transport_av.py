"""Synthetic Opus/VP8 inputs and independent decoded native observations.

This owns no Connect authority or media transport. Source files, encoded packets
and decoder output remain private; a decoded component never qualifies a cell.
"""
import array
import base64
import hashlib
import json
import math
from pathlib import Path
import struct
import subprocess

from .package_environment import fixture_environment

SECONDS = 62  # two seconds to establish clocks, then the frozen 60-second window
COLORS = ('red', 'lime', 'blue', 'yellow', 'cyan', 'magenta')
FREQUENCIES = (440, 600, 760, 920, 1080, 1240)
MAX_FILE = 32 * 1024 * 1024


def checksum(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def ogg_crc(body):
    value = 0
    for byte in body:
        value ^= byte << 24
        for _ in range(8):
            value = ((value << 1) ^ (0x04c11db7 if value & 0x80000000 else 0)) & 0xffffffff
    return value


def opus_packets(body):
    """Validate the complete encoder container, including CRC and lacing."""
    if len(body) > MAX_FILE:
        raise ValueError('Opus source size')
    offset = sequence = 0
    serial = None
    pending = bytearray()
    packets = []
    ended = False
    while offset < len(body):
        header = body[offset:offset + 27]
        if len(header) != 27 or header[:5] != b'OggS\0' or ended:
            raise ValueError('Opus page header')
        count = header[26]
        lacing = body[offset + 27:offset + 27 + count]
        size = 27 + count + sum(lacing)
        page = bytearray(body[offset:offset + size])
        current_serial, current_sequence, expected_crc = struct.unpack_from('<III', header, 14)
        if (len(lacing) != count or len(page) != size or current_sequence != sequence
                or (serial is not None and current_serial != serial)
                or bool(header[5] & 1) != bool(pending) or header[5] & ~7
                or bool(header[5] & 2) != (sequence == 0)):
            raise ValueError('Opus page sequence or continuation')
        page[22:26] = b'\0' * 4
        if ogg_crc(page) != expected_crc:
            raise ValueError('Opus page checksum')
        serial = current_serial
        sequence += 1
        at = 27 + count
        for length in lacing:
            pending.extend(page[at:at + length]); at += length
            if len(pending) > 65536:
                raise ValueError('Opus packet size')
            if length < 255:
                packets.append(bytes(pending)); pending.clear()
        ended = bool(header[5] & 4)
        offset += size
    if pending or not ended or len(packets) < 3 or len(packets) > SECONDS * 50 + 3:
        raise ValueError('Opus incomplete source')
    head = packets[0]
    if len(head) != 19 or head[:10] != b'OpusHead\x01\x01' or struct.unpack_from('<I', head, 12)[0] != 48000 or head[18] != 0:
        raise ValueError('Opus mono 48k profile')
    if not packets[1].startswith(b'OpusTags'):
        raise ValueError('Opus tags')
    return packets[2:]


def ivf_frames(body):
    if len(body) < 32 or len(body) > MAX_FILE:
        raise ValueError('VP8 source size')
    magic, version, size, codec, width, height, rate, scale, count, reserved = struct.unpack('<4sHH4sHHIIII', body[:32])
    if (magic, version, size, codec, width, height, rate, scale, reserved) != (b'DKIF', 0, 32, b'VP80', 640, 360, 30, 1, 0):
        raise ValueError('VP8 360p30 profile')
    frames = []; offset = 32; last_key = -30
    while offset < len(body):
        if offset + 12 > len(body) or len(frames) >= SECONDS * 30:
            raise ValueError('VP8 frame header')
        length, pts = struct.unpack_from('<IQ', body, offset); offset += 12
        encoded = body[offset:offset + length]; offset += length
        if not 0 < length <= 256 * 1024 or len(encoded) != length or pts != len(frames):
            raise ValueError('VP8 frame bound or timestamp')
        if encoded[0] & 1 == 0:
            if len(encoded) < 10 or encoded[3:6] != b'\x9d\x01\x2a' or struct.unpack_from('<HH', encoded, 6) != (640, 360):
                raise ValueError('VP8 keyframe dimensions')
            last_key = pts
        if pts - last_key >= 30:
            raise ValueError('VP8 keyframe interval')
        frames.append(dict(timestamp=pts * 3000, payload=base64.b64encode(encoded).decode()))
    if count not in (0, len(frames)) or len(frames) != SECONDS * 30:
        raise ValueError('VP8 finite frame count')
    return frames


def command(args, log):
    with log.open('xb') as output:
        result = subprocess.run(args, stdin=subprocess.DEVNULL, stdout=output, stderr=subprocess.STDOUT, timeout=120, env=fixture_environment())
    if result.returncode:
        raise ValueError('Independent codec command failed; inspect private log')


def prepare(directory):
    directory.mkdir(mode=0o700)
    ffmpeg = subprocess.check_output(['ffmpeg', '-version'], text=True, timeout=5, env=fixture_environment()).splitlines()[0]
    records = []
    for index, (hz, color) in enumerate(zip(FREQUENCIES, COLORS)):
        audio = directory / f'source-{index}.ogg'; video = directory / f'source-{index}.ivf'
        common = ['ffmpeg', '-nostdin', '-hide_banner', '-loglevel', 'error', '-y']
        command(common + ['-f', 'lavfi', '-i', f'sine=frequency={hz}:sample_rate=48000:duration={SECONDS}',
            '-ac', '1', '-c:a', 'libopus', '-b:a', '32k', '-vbr', 'off', '-frame_duration', '20', '-application', 'audio', str(audio)], directory / f'encode-audio-{index}.log')
        command(common + ['-f', 'lavfi', '-i', f'testsrc2=size=640x360:rate=30:duration={SECONDS}',
            '-vf', f'drawbox=x=0:y=0:w=80:h=80:color={color}:t=fill', '-an', '-c:v', 'libvpx', '-threads', '1',
            '-deadline', 'realtime', '-cpu-used', '8', '-b:v', '500k', '-maxrate', '1M', '-bufsize', '1M', '-g', '30', str(video)], directory / f'encode-video-{index}.log')
        audio_packets = opus_packets(audio.read_bytes())[:SECONDS * 50]
        if len(audio_packets) != SECONDS * 50:
            raise ValueError('Opus frame count')
        value = dict(format='connect/native-av-source/1', seconds=SECONDS, audio_hz=hz,
            audio=[base64.b64encode(packet).decode() for packet in audio_packets], video=ivf_frames(video.read_bytes()))
        path = directory / f'source-{index}.json'; path.write_text(json.dumps(value) + '\n')
        records.append(dict(index=index, audio_hz=hz, video_color=color, input_sha256=checksum(path),
            ogg_sha256=checksum(audio), ivf_sha256=checksum(video)))
    manifest = dict(format='connect/native-av-sources/1', ffmpeg=ffmpeg, seconds=SECONDS,
        warmup_seconds=2, observation_seconds=60, audio=dict(codec='opus', sample_rate=48000, channels=1, frame_ms=20, target_bps=32000),
        video=dict(codec='vp8', width=640, height=360, rate=30, target_bps=500000, max_bps=1000000, keyframe_frames=30), sources=records)
    (directory / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    return manifest


def ogg_page(packet, sequence, granule, flags=0):
    if len(packet) > 16384:
        raise ValueError('Received Opus packet bound')
    sizes = bytes([255] * (len(packet) // 255) + [len(packet) % 255])
    body = bytearray(struct.pack('<4sBBQIIIB', b'OggS', 0, flags, granule, 0x47363035, sequence, 0, len(sizes)) + sizes + packet)
    struct.pack_into('<I', body, 22, ogg_crc(body))
    return body


def decode_audio(directory, index, rows, expected_hz):
    path = directory / f'received-{index}.ogg'
    head = struct.pack('<8sBBHIhB', b'OpusHead', 1, 1, 0, 48000, 0, 0)
    tags = b'OpusTags' + struct.pack('<II', 0, 0)
    with path.open('xb') as stream:
        stream.write(ogg_page(head, 0, 0, 2)); stream.write(ogg_page(tags, 1, 0))
        for offset, row in enumerate(rows):
            stream.write(ogg_page(base64.b64decode(row['payload'], validate=True), offset + 2, (offset + 1) * 960, 4 if offset == len(rows) - 1 else 0))
    raw = directory / f'received-{index}.f32'
    command(['ffmpeg', '-nostdin', '-hide_banner', '-loglevel', 'warning', '-i', str(path), '-ac', '1', '-ar', '48000', '-f', 'f32le', str(raw)], directory / f'decode-audio-{index}.log')
    values = array.array('f'); values.frombytes(raw.read_bytes())
    # Independent spectral identity in every complete 100ms decoded block.
    desired = FREQUENCIES.index(expected_hz); powers = []; bad = 0
    for offset in range(0, len(values) - 4799, 4800):
        block = values[offset:offset + 4800:8]
        energy = sum(v * v for v in block) / len(block)
        tone = []
        for hz in FREQUENCIES:
            coefficient = 2 * math.cos(2 * math.pi * hz / 6000); a = b = 0
            for sample in block:
                c = sample + coefficient * a - b; b = a; a = c
            tone.append((a * a + b * b - coefficient * a * b) / len(block) ** 2)
        wrong = max(v for i, v in enumerate(tone) if i != desired)
        bad += int(energy <= 1e-6 or tone[desired] <= 8 * max(wrong, 1e-12))
        powers.append(energy)
    return dict(decoder='independent FFmpeg Opus/f32le', samples=len(values), decoded_seconds=len(values) / 48000,
        expected_hz=expected_hz, complete_100ms_blocks=len(powers), bad_identity_blocks=bad,
        all_blocks_identified=bool(powers) and bad == 0, pcm_sha256=checksum(raw), ogg_sha256=checksum(path))


def rtp_timing(rows, rate, step):
    """Describe original RTP progression without filling gaps or retiming media."""
    if not rows or len(rows) > 3200 or (rate, step) not in ((48000, 960), (90000, 3000)):
        raise ValueError('RTP timing observation bound')
    progress = 0; offsets = []; elapsed = 0
    missing = repeats = backwards = irregular = sequence_missing = sequence_backwards = 0
    for previous, current in zip(rows, rows[1:]):
        arrival = current['elapsed_ms'] - previous['elapsed_ms']
        if arrival < 0 or not math.isfinite(arrival):
            raise ValueError('RTP observation arrival order')
        delta = ((current['timestamp'] - previous['timestamp'] + 2 ** 31) % 2 ** 32) - 2 ** 31
        progress += delta; elapsed += arrival
        if delta == 0: repeats += 1
        elif delta < 0: backwards += 1
        else:
            missing += max(0, delta // step - 1)
            irregular += int(delta % step != 0)
        offsets.append(progress * 1000 / rate - elapsed)
        if 'sequence' in previous and 'sequence' in current:
            distance = (current['sequence'] - previous['sequence']) % 65536
            if distance > 32768: sequence_backwards += 1
            elif distance: sequence_missing += distance - 1
    return dict(packets_or_frames=len(rows), clock_rate=rate, nominal_step=step,
        rtp_span_ms=(progress + step) * 1000 / rate,
        arrival_span_ms=rows[-1]['elapsed_ms'] - rows[0]['elapsed_ms'] + step * 1000 / rate,
        rtp_minus_arrival_span_ms=progress * 1000 / rate - elapsed,
        minimum_progress_offset_ms=min(offsets, default=0), maximum_progress_offset_ms=max(offsets, default=0),
        missing_timestamp_steps=missing, repeated_timestamps=repeats, backwards_timestamps=backwards,
        non_nominal_timestamp_deltas=irregular, missing_rtp_sequence_packets=sequence_missing,
        backwards_rtp_sequences=sequence_backwards,
        scope='Original received header/arrival diagnostics only; no invented samples, PLC, source-age or clock synchronization claim.')


def source_pixels(values, expected_index):
    """Distinguish an identified other source marker from an unclassified pixel."""
    colors = ((255, 0, 0), (0, 255, 0), (0, 0, 255), (255, 255, 0), (0, 255, 255), (255, 0, 255))
    if len(values) % 3 or expected_index not in range(6):
        raise ValueError('Decoded source pixel observation')
    counts = [0] * 6; unknown = 0
    for at in range(0, len(values), 3):
        found = next((i for i, color in enumerate(colors)
            if all(abs(component - target) <= 40 for component, target in zip(values[at:at + 3], color))), None)
        if found is None: unknown += 1
        else: counts[found] += 1
    return dict(expected_source_index=expected_index, source_marker_counts=counts,
        expected_source_frames=counts[expected_index],
        other_identified_source_frames=sum(counts) - counts[expected_index],
        unclassified_pixel_frames=unknown, wrong_source_pixel_frames=len(values) // 3 - counts[expected_index])


def verify(directory, manifest):
    """Decode bytes independently; retain explicit measurement limits."""
    results = []
    for index in range(6):
        path = directory / f'received-{index}.json'
        if path.stat().st_size > MAX_FILE:
            raise ValueError('Native media observation size')
        captured = json.loads(path.read_text())
        if captured['format'] != 'connect/native-av-received/1' or captured['index'] != index:
            raise ValueError('Native media observation identity')
        audio = [r for r in captured['audio'] if 2000 <= r['elapsed_ms'] < 62000]
        video = [r for r in captured['video'] if 2000 <= r['elapsed_ms'] < 62000]
        if len(audio) > 3200 or len(video) > 2000 or not audio or not video:
            raise ValueError('Native media observation count')
        audio_result = decode_audio(directory, index, audio, manifest['sources'][captured['peer_index']]['audio_hz'])
        # Decode from the first received keyframe, retaining the original RTP
        # timestamps. Warmup frames initialize the codec but do not count toward
        # the 60-second measurement window.
        all_video = captured['video']
        first_key = next((at for at, row in enumerate(all_video) if row['key']), None)
        if first_key is None:
            raise ValueError('Native media missing keyframe')
        ivf = directory / f'received-{index}.ivf'
        with ivf.open('xb') as stream:
            stream.write(struct.pack('<4sHH4sHHIIII', b'DKIF', 0, 32, b'VP80', 640, 360, 90000, 1, 0, 0))
            origin = all_video[first_key]['timestamp']
            for row in all_video[first_key:]:
                body = base64.b64decode(row['payload'], validate=True)
                if not 0 < len(body) <= 256 * 1024:
                    raise ValueError('Received VP8 frame bound')
                stream.write(struct.pack('<IQ', len(body), (row['timestamp'] - origin) & 0xffffffff) + body)
        hashes = directory / f'received-{index}.framemd5'
        command(['ffmpeg', '-nostdin', '-hide_banner', '-loglevel', 'error', '-copyts', '-i', str(ivf), '-an',
            '-enc_time_base', '1:90000', '-fps_mode', 'passthrough', '-f', 'framemd5', str(hashes)], directory / f'decode-video-{index}.log')
        lines = hashes.read_text().splitlines()
        if '#dimensions 0: 640x360' not in lines:
            raise ValueError('Independently decoded VP8 dimensions')
        decoded = [line for line in lines if line and not line.startswith('#')]
        if len(decoded) != len(all_video) - first_key:
            raise ValueError('Received/decoded VP8 frame identity differs')
        timestamps = [row['timestamp'] for row in all_video[first_key:]]
        if len(set(timestamps)) != len(timestamps) or any(b <= a for a, b in zip(timestamps, timestamps[1:])):
            raise ValueError('Received VP8 timestamp order')
        if any(int(line.split(',')[2]) != row['timestamp'] - origin for row, line in zip(all_video[first_key:], decoded)):
            raise ValueError('Independent decoder PTS differs from received frame')
        mapping = {row['timestamp']: line.split(',')[-1].strip() for row, line in zip(all_video[first_key:], decoded)}
        window_hashes = [mapping[row['timestamp']] for row in video if row['timestamp'] in mapping]
        changing = sum(a != b for a, b in zip(window_hashes, window_hashes[1:]))
        # Record independently decoded source pixels as well as motion. Decoder
        # completion times and paired A/V pulses require separate observations.
        pixels = directory / f'received-{index}.rgb'
        command(['ffmpeg', '-nostdin', '-hide_banner', '-loglevel', 'error', '-i', str(ivf), '-an',
            '-vf', 'crop=16:16:8:8,scale=1:1', '-pix_fmt', 'rgb24', '-fps_mode', 'passthrough', '-f', 'rawvideo', str(pixels)], directory / f'decode-marker-{index}.log')
        values = pixels.read_bytes(); markers = source_pixels(values, captured['peer_index'])
        wrong = markers['wrong_source_pixel_frames']
        audio_result['rtp_timing'] = rtp_timing(audio, 48000, 960)
        bins = [sum(second * 1000 <= row['elapsed_ms'] < (second + 1) * 1000 for row in video) for second in range(2, 62)]
        gaps = [b['elapsed_ms'] - a['elapsed_ms'] for a, b in zip(audio, audio[1:])]
        passed = (audio_result['all_blocks_identified'] and audio_result['decoded_seconds'] >= 59.9 and audio_result['samples'] == len(audio) * 960
            and max(gaps, default=math.inf) <= 200 and len(window_hashes) >= 1440 and changing >= 1440
            and min(bins) >= 24 and len(values) == 3 * len(decoded) and wrong == 0
            and captured['received_clocks']['audio'] > 0 and captured['received_clocks']['video'] > 0)
        results.append(dict(index=index, peer_index=captured['peer_index'], passed=passed, audio=audio_result,
            received_window_audio_packets=len(audio), maximum_audio_arrival_gap_ms=max(gaps, default=None),
            video=dict(independent_decoder='FFmpeg VP8/framemd5/rgb24', width=640, height=360,
                decoded_window_frames=len(window_hashes), changing_window_frames=changing, changing_fps=changing / 60,
                frames_by_observation_second=bins, wrong_source_pixel_frames=wrong, source_marker_classification=markers,
                rtp_timing=rtp_timing(video, 90000, 3000), ivf_sha256=checksum(ivf), framemd5_sha256=checksum(hashes)),
            received_clocks=captured['received_clocks'], gap_packets=captured['gap_packets'], late_send_frames=captured['late_send_frames'],
            observation_sha256=checksum(path)))
    report = dict(format='connect/native-av-decoded-component/1', passed=all(row['passed'] for row in results),
        source_manifest=manifest, connections=6, participants=4, sessions=2, observation_seconds=60, warmup_seconds=2,
        routes=results, scope='Independent decoded native Opus/VP8 source identity and moving-frame component in the named six-Connection workload. RTP arrival times bind observation-window membership. Decoder completion times, source-age/paired AV-sync, live upgrade/replacement, impairment and complete transport cell qualification remain open.',
        task_complete=False, cell_qualified=False, matrix_qualified=False, whole_gate_qualified=False)
    (directory / 'decoded.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def decode_completed(observed, directory, manifest):
    """A late control failure does not erase a completed media observation."""
    if observed.get('native_av', {}).get('original_receive_tasks_joined') is not True:
        return None
    return verify(directory, manifest)
