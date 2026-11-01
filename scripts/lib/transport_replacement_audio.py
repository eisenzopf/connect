"""Independent Opus identity and original packet-arrival observations for a move.

Inputs are private original RTP payloads, after public SDK/native RTCP demux.
No PLC, gap filling, retiming, transport or mutation is performed here.
"""
import array
import hashlib
import math
import re
import struct
import subprocess

from .package_environment import fixture_environment
from .transport_av import ogg_page


def classify_pcm(samples, rows, frequencies=(440, 660, 880)):
    if (len(frequencies) != 3 or len(set(frequencies)) != 3
            or any(type(hz) is not int or not 100 <= hz <= 2000 for hz in frequencies)
            or not 1 <= len(rows) <= 4000 or len(samples) != len(rows) * 960
            or any(not math.isfinite(value) for value in samples)):
        raise ValueError('original Opus PCM/packet observation bound')
    labels = []
    previous = -1
    for index, row in enumerate(rows):
        elapsed = row['elapsed_ms']
        if (not isinstance(elapsed, (int, float)) or not math.isfinite(elapsed)
                or elapsed < previous or elapsed < 0):
            raise ValueError('original packet arrival order')
        previous = elapsed
        block = samples[index * 960:(index + 1) * 960:8]
        powers = []
        for hz in frequencies:
            coefficient = 2 * math.cos(2 * math.pi * hz / 6000)
            a = b = 0
            for sample in block:
                value = sample + coefficient * a - b
                b, a = a, value
            powers.append((a * a + b * b - coefficient * a * b) / len(block) ** 2)
        best = max(range(3), key=lambda k: powers[k])
        other = max(value for k, value in enumerate(powers) if k != best)
        energy = sum(value * value for value in block) / len(block)
        label = frequencies[best] if energy > 1e-6 and powers[best] > 8 * max(other, 1e-12) else None
        labels.append(dict(label_hz=label, elapsed_ms=elapsed, timestamp=row['timestamp']))
    return labels


def decode_original_packets(directory, name, rows, frequencies=(440, 660, 880)):
    if not re.fullmatch(r'[a-z-]{1,32}', name) or not 1 <= len(rows) <= 4000:
        raise ValueError('private decoder name or packet bound')
    # Validate every original packet before invoking the independent decoder.
    packets = []
    for row in rows:
        payload = row['payload']
        if (not isinstance(payload, list) or not 1 <= len(payload) <= 1168
                or any(type(value) is not int or not 0 <= value <= 255 for value in payload)
                or type(row['timestamp']) is not int or not 0 <= row['timestamp'] <= 0xffffffff):
            raise ValueError('original RTP payload/clock bound')
        packets.append(bytes(payload))
    path = directory / (name + '.ogg')
    with path.open('xb') as output:
        output.write(ogg_page(struct.pack('<8sBBHIhB', b'OpusHead', 1, 1, 0, 48000, 0, 0), 0, 0, 2))
        output.write(ogg_page(b'OpusTags' + struct.pack('<II', 0, 0), 1, 0))
        for index, payload in enumerate(packets):
            output.write(ogg_page(payload, index + 2, (index + 1) * 960, 4 if index == len(packets) - 1 else 0))
    raw = directory / (name + '.f32')
    with (directory / (name + '-decode.log')).open('xb') as log:
        result = subprocess.run(['ffmpeg', '-nostdin', '-hide_banner', '-loglevel', 'warning',
            '-i', str(path), '-ac', '1', '-ar', '48000', '-f', 'f32le', str(raw)],
            stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
            timeout=30, env=fixture_environment())
    if result.returncode != 0 or raw.stat().st_size != len(rows) * 960 * 4:
        raise ValueError('independent decode changed original 960-sample packet correspondence')
    samples = array.array('f')
    samples.frombytes(raw.read_bytes())
    labels = classify_pcm(samples, rows, frequencies)
    return dict(packets=len(rows), samples=len(samples),
        identified_counts={str(hz): sum(value['label_hz'] == hz for value in labels) for hz in frequencies},
        unclassified=sum(value['label_hz'] is None for value in labels),
        pcm_sha256=hashlib.sha256(raw.read_bytes()).hexdigest(),
        ogg_sha256=hashlib.sha256(path.read_bytes()).hexdigest()), labels


def replacement_arrival_observation(labels, frequencies=(440, 660, 880)):
    """One frozen ten-second source component; arrival is not presentation."""
    old_hz, new_hz, retained_hz = frequencies
    original, retained, target = (labels[key] for key in ('original', 'retained', 'target'))
    old = [row for row in retained if row['label_hz'] == old_hz]
    new = [row for row in retained if row['label_hz'] == new_hz]
    if (sum(row['label_hz'] == retained_hz for row in original) < 20
            or sum(row['label_hz'] == retained_hz for row in target) < 300
            or len(old) < 20 or len(new) < 300
            or any(row['label_hz'] in (old_hz, new_hz) for row in original + target)
            or any(row['label_hz'] == retained_hz for row in retained)):
        raise ValueError('independent original/retained/destination speaker identity failed')
    last_old, first_new = old[-1]['elapsed_ms'], new[0]['elapsed_ms']
    gap = first_new - last_old - 20
    if not -20 <= gap <= 1000 or any(row['label_hz'] == old_hz and row['elapsed_ms'] >= first_new for row in retained):
        raise ValueError('original speaker replay or decoded packet-arrival gap')
    return dict(original_speaker_after_identified_destination_frames=0,
        retained_receiver_last_original_speaker_ms=last_old,
        retained_receiver_first_destination_speaker_ms=first_new,
        decoded_packet_arrival_gap_ms=gap, one_observation_within_1000ms=True,
        decoder='independent FFmpeg Opus/f32le; per-original-packet spectral identity',
        device_presentation_measured=False, paired_av_sync_qualified=False,
        full_media_quality_qualified=False, full_transport_cell_qualified=False,
        scope='Decoded original packets retain their receive order and each 960-sample output maps to its original arrival. No PLC, invented samples or source-clock correction. Packet-arrival timing is not decoder completion or device presentation.')
