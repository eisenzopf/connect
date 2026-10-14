"""Independent post-run pixels/audio verification tied to real live observations.

Decoder timestamps bind retained IVF to live journals. This module does not own
servers, capture, transport processes or their operating measurements.
"""
import array
import hashlib
import importlib.util
import math
import pathlib
import struct
import subprocess
import tempfile


_marker_spec = importlib.util.spec_from_file_location('native_marker_calibration',
    pathlib.Path(__file__).with_name('marker_calibration.py'))
markers = importlib.util.module_from_spec(_marker_spec); _marker_spec.loader.exec_module(markers)
_audio_spec = importlib.util.spec_from_file_location('browser_audio_observation',
    pathlib.Path(__file__).with_name('audio_observation.py'))
audio_observer = importlib.util.module_from_spec(_audio_spec); _audio_spec.loader.exec_module(audio_observer)
_motion_spec = importlib.util.spec_from_file_location('decoded_motion_observation',
    pathlib.Path(__file__).with_name('motion_observation.py'))
motion_observer = importlib.util.module_from_spec(_motion_spec); _motion_spec.loader.exec_module(motion_observer)


def observed_rising_edges(levels, times):
    """An initially high cropped sample proves a level, not a rising edge."""
    previous = None; edges = []
    for current, at in zip(levels, times):
        if previous is False and current: edges.append(at)
        previous = current
    return edges


def source_identity(pixels, expected_id, allow_muted_black=False):
    """Sender identity comes from decoded pixels, independently of audio routing."""
    if expected_id not in range(4) or len(pixels) % 64:
        raise ValueError('Generated source identity requires 0..3 and complete 8×8 decoded pixels')
    counts = {str(index): 0 for index in range(4)}
    counts.update(pulse=0, black=0, unclassified=0)
    for at in range(0, len(pixels), 64):
        frame = pixels[at:at + 64]
        if min(frame) > 240: key = 'pulse'
        elif max(frame) < 12: key = 'black'
        else:
            bits = [0 if frame[index] <= 85 else 1 if 155 <= frame[index] <= 230 else None
                for index in [0, 1, 8, 9]]
            key = (str(bits[0] + 2 * bits[1]) if None not in bits
                and bits[0] != bits[2] and bits[1] != bits[3] else 'unclassified')
        counts[key] += 1
    expected = counts[str(expected_id)]; wrong = sum(counts[str(index)] for index in range(4) if index != expected_id)
    unknown = counts['unclassified'] + (0 if allow_muted_black else counts['black'])
    return {'status': 'passed' if expected > 0 and wrong == 0 and unknown == 0 else 'failed',
        'expected_source_id': expected_id, 'recognized_expected_frames': expected,
        'recognized_wrong_source_frames': wrong, 'unclassified_frames': counts['unclassified'],
        'full_white_pulse_frames': counts['pulse'], 'black_frames': counts['black'],
        'muted_black_permitted': allow_muted_black,
        'recognized_source_counts': {str(index): counts[str(index)] for index in range(4)},
        'method': 'independent decoded 8×8 pixels: four large gray cells encode two source bits plus complements; white AV pulses excluded from identity only'}


def retain_decoder_diagnostics(path, stage, result):
    # Qualification callers select private work paths. Fake measurement unit
    # inputs do not create logs in the checkout.
    selected = pathlib.Path(path)
    if selected.is_file():
        target = selected.with_name(selected.name + '-' + stage + '-decoder.log')
        with target.open('xb') as output:
            output.write(result.stderr.encode() if isinstance(result.stderr, str) else result.stderr)
        target.chmod(0o600)


def decode_video(path, seconds, ffmpeg='ffmpeg', report=None, window=None):
    # Actual decoded pixels; packet/assembled-frame receipt alone cannot pass.
    result = subprocess.run([ffmpeg, '-nostdin', '-hide_banner', '-loglevel', 'warning',
        '-copyts', '-i', str(path), '-an', '-enc_time_base', '1:90000',
        '-fps_mode', 'passthrough', '-f', 'framemd5', 'pipe:1'],
        capture_output=True, text=True, timeout=45)
    retain_decoder_diagnostics(path, 'motion', result)
    if result.returncode:
        raise RuntimeError('Independent VP8 decode failed')
    lines = [line for line in result.stdout.splitlines() if line and not line.startswith('#')]
    hashes = [line.split(',')[-1].strip() for line in lines]
    moving = sum(a != b for a, b in zip(hashes, hashes[1:]))
    dimensions = next((line.split(':', 1)[1].strip().split('x') for line in result.stdout.splitlines()
        if line.startswith('#dimensions 0:')), [])
    observation = {'decoder': 'FFmpeg framemd5', 'decoded_frames': len(hashes),
        'width': int(dimensions[0]) if len(dimensions) == 2 else None,
        'height': int(dimensions[1]) if len(dimensions) == 2 else None,
        'changing_frames': moving, 'changing_fps': moving / seconds,
        'decoder_exit_status': result.returncode,
        'decoder_diagnostics_bytes': len(result.stderr.encode()),
        'decoder_diagnostic_lines': len(result.stderr.splitlines()),
        'clean_decoder_diagnostics': not result.stderr.strip()}
    if report is not None and window is not None:
        mapped, mapping = map_decoded_frames(path, report, window, result.stdout)
        when = lambda row: report['unix_start_seconds'] + row.get('emit_seconds', row['arrival_seconds'])
        began = report['unix_start_seconds'] + window.get('start_emit_seconds', window['start_arrival_seconds'])
        observation['decoded_mapping'] = mapping
        keys = [when(row) for row, _, _ in mapped if row['keyframe']]
        observation['decoded_keyframe_observation'] = {'scope': 'exact independently decoded PTS bound to actual received keyframe flags and live emission times; no actual decoder completion timestamp',
            'count': len(keys), 'unix_seconds': keys, 'source_periodic_target_seconds': 1,
            'maximum_observed_inter_keyframe_seconds': max((b - a for a, b in zip(keys, keys[1:])), default=None),
            'window_boundaries_included': False}
        observation['motion_content'] = {'window_start_unix_seconds': began,
            'window_end_unix_seconds': began + seconds, 'changing_times': [when(current[0])
                for previous, current in zip(mapped, mapped[1:]) if current[1] != previous[1]]}
    return observation


def decode_hash_frames(path, ffmpeg='ffmpeg'):
    result = subprocess.run([ffmpeg, '-nostdin', '-hide_banner', '-loglevel', 'warning',
        '-copyts', '-i', str(path), '-an', '-enc_time_base', '1:90000',
        '-fps_mode', 'passthrough', '-f', 'framemd5', 'pipe:1'],
        capture_output=True, text=True, timeout=45)
    retain_decoder_diagnostics(path, 'timestamped-motion', result)
    if result.returncode: raise RuntimeError('Independent timestamped VP8 decode failed')
    return result.stdout, len(result.stderr.encode())


def decode_gray_frames(path, ffmpeg='ffmpeg'):
    # Two outputs share the independent decoder. A private descriptor separates
    # timestamp/hash metadata from diagnostic stderr and raw decoded pixels.
    with tempfile.TemporaryFile(dir=pathlib.Path(path).parent) as metadata:
        options = ['-map', '0:v:0', '-an', '-vf', 'scale=8:8', '-pix_fmt', 'gray',
            '-enc_time_base', '1:90000', '-fps_mode', 'passthrough']
        result = subprocess.run([ffmpeg, '-nostdin', '-hide_banner', '-loglevel', 'warning',
            '-copyts', '-i', str(path), *options, '-f', 'rawvideo', 'pipe:1',
            *options, '-f', 'framemd5', f'pipe:{metadata.fileno()}'],
            pass_fds=(metadata.fileno(),), capture_output=True, timeout=45)
        retain_decoder_diagnostics(path, 'timestamped-markers', result)
        if result.returncode: raise RuntimeError('Independent timestamped marker decode failed')
        metadata.seek(0); hashes = metadata.read().decode('ascii')
    return result.stdout, hashes, len(result.stderr)


def ivf_frame_timestamps(path):
    stamps = []; size = pathlib.Path(path).stat().st_size
    with pathlib.Path(path).open('rb') as source:
        header = source.read(32)
        if len(header) != 32 or header[:4] != b'DKIF' or header[8:12] != b'VP80' \
                or struct.unpack_from('<II', header, 16) != (90000, 1):
            raise RuntimeError('Qualification IVF must retain the explicit 90000 Hz presentation clock')
        while frame := source.read(12):
            if len(frame) != 12: raise RuntimeError('Truncated qualification IVF timestamp')
            encoded_bytes, timestamp = struct.unpack('<IQ', frame)
            if not 0 < encoded_bytes <= 256 * 1024 or len(stamps) >= 10000:
                raise RuntimeError('Qualification IVF frame index exceeds bounded profile')
            source.seek(encoded_bytes, 1)
            if source.tell() > size: raise RuntimeError('Truncated qualification IVF payload')
            if stamps and timestamp <= stamps[-1]: raise RuntimeError('IVF live emission timestamps are not strictly ordered')
            stamps.append(timestamp)
    return stamps


def live_keyframe_present(path):
    """Bounded read of an application's growing IVF, without claiming decoding."""
    selected = pathlib.Path(path)
    if not selected.exists(): return False
    with selected.open('rb') as source:
        header = source.read(32)
        if len(header) < 32: return False
        if header[:4] != b'DKIF' or header[8:12] != b'VP80':
            raise RuntimeError('Native readiness observation requires an actual VP8 IVF')
        for _ in range(10000):
            frame = source.read(12)
            if len(frame) < 12: return False
            size, _ = struct.unpack('<IQ', frame)
            if not 0 < size <= 256 * 1024: raise RuntimeError('Live IVF frame exceeds profile')
            encoded = source.read(size)
            if len(encoded) < size: return False
            if len(encoded) >= 10 and not encoded[0] & 1 and encoded[3:6] == b'\x9d\x01\x2a': return True
        raise RuntimeError('Live IVF readiness index exceeds bounded profile')


def map_decoded_frames(path, report, window, metadata):
    """Bind actual decoder PTS to actual IVF input and live journals, without sorting."""
    start = window.get('start_emit_seconds', window['start_arrival_seconds']); end = start + window['seconds']
    when = lambda row: row.get('emit_seconds', row['arrival_seconds'])
    rows = [row for row in report['video'] if start <= when(row) < end]
    stamps = ivf_frame_timestamps(path)
    if not rows or len(rows) != len(stamps) or len(rows) > 10000:
        raise RuntimeError('IVF timestamp index and live complete-frame journal disagree')
    origin = rows[0].get('timestamp_ivf', rows[0]['timestamp_rtp'])
    expected = [row['timestamp_ivf'] - origin if 'timestamp_ivf' in row
        else (row['timestamp_rtp'] - origin) % 2**32 for row in rows]
    if expected != stamps: raise RuntimeError('IVF timestamps differ from actual live emission journal')
    if '#tb 0: 1/90000' not in metadata.splitlines():
        raise RuntimeError('Independent decoder did not retain the selected IVF presentation clock')
    index = {stamp: position for position, stamp in enumerate(stamps)}
    mapped = []; last = -1
    for line in metadata.splitlines():
        if not line or line.startswith('#'): continue
        fields = [value.strip() for value in line.split(',')]
        if len(fields) != 6 or int(fields[0]) != 0: raise RuntimeError('Invalid independent decoded frame metadata')
        position = index.get(int(fields[2]))
        if position is None or position <= last:
            raise RuntimeError('Decoded PTS has no unique ordered IVF/live-journal mapping')
        mapped.append((rows[position], fields[5], position)); last = position
    if not mapped: raise RuntimeError('Independent decoder produced no mapped pixels')
    return mapped, {'method': 'actual decoded PTS at 1/90000 matched exactly to ordered retained IVF timestamps and live complete-frame journal; no sorting or inferred timestamps',
        'input_frame_index_bound': 10000, 'indexed_complete_inputs': len(rows),
        'mapped_decoded_frames': len(mapped), 'decoder_skipped_complete_inputs': len(rows) - len(mapped),
        'missing_or_duplicate_decoded_mappings': 0}


def decode_audio(path, expected_hz, ffmpeg='ffmpeg'):
    if not isinstance(expected_hz, (int, float)) or not 100 <= expected_hz <= 3000:
        raise ValueError('Expected generated audio requires an identifiable 100–3000 Hz tone; zero frequency is unsupported')
    result = subprocess.run([ffmpeg, '-nostdin', '-hide_banner', '-loglevel', 'warning',
        '-f', 'mulaw', '-ar', '8000', '-ac', '1', '-i', str(path),
        '-f', 'f32le', 'pipe:1'], capture_output=True, timeout=30)
    retain_decoder_diagnostics(path, 'tone', result)
    if result.returncode or not result.stdout:
        raise RuntimeError('Independent PCMU decode failed')
    samples = array.array('f'); samples.frombytes(result.stdout)
    # Analyse ten separated windows; a final tone alone cannot qualify a long stream.
    observations = []
    for index in range(10):
        start = int(index * max(0, len(samples) - 8000) / 9)
        values = samples[start:start + 8000]
        average = sum(values) / len(values)
        rms = math.sqrt(sum((x - average) ** 2 for x in values) / len(values))
        crossings = sum(a <= 0 < b for a, b in zip(values, values[1:]))
        frequency = crossings * 8000 / len(values)
        observations.append({'seconds': start / 8000, 'ac_rms': rms, 'frequency_hz': frequency})
    return {'decoder': 'FFmpeg PCMU to float32', 'samples': len(samples),
        'decoder_exit_status': result.returncode, 'decoder_diagnostics_bytes': len(result.stderr),
        'duration_seconds': len(samples) / 8000, 'expected_tone_hz': expected_hz,
        'windows': observations, 'identifiable_tone': all(
            x['ac_rms'] > .003 and abs(x['frequency_hz'] - expected_hz) <= 20 for x in observations)}


def clock_observation(report):
    """Relative clock drift with first-arrival anchoring, not an absolute lip-sync claim."""
    tracks = {}
    for kind, clock in [('audio', 8000), ('video', 90000)]:
        samples = report[kind]
        if len(samples) < 2:
            tracks[kind] = None; continue
        first = samples[0]
        values = [(s['arrival_seconds'] - first['arrival_seconds']) -
            ((s['timestamp_rtp'] - first['timestamp_rtp']) % 2**32) / clock for s in samples]
        tracks[kind] = {'first_arrival_seconds': first['arrival_seconds'],
            'last_relative_clock_error_ms': values[-1] * 1000,
            'max_relative_clock_error_ms': max(map(abs, values)) * 1000}
    return {'tracks': tracks, 'absolute_av_sync': 'unmeasured',
        'method': 'RTP clock against local arrival, each stream independently anchored at first arrival; random RTP origins provide no absolute audiovisual synchronization'}


def select_window(participant, report, seconds):
    """Retain an exact elapsed receive window starting at an actual keyframe."""
    video = report['video']; audio = report['audio']
    first = next((i for i, row in enumerate(video) if row['keyframe']), None)
    if first is None: raise RuntimeError('No native received keyframe')
    when = lambda row: row.get('emit_seconds', row['arrival_seconds'])
    began = when(video[first]); ended = began + seconds
    if not video or when(video[-1]) < ended - .15:
        raise RuntimeError('Native receive interval shorter than requested clean window')
    selected = [i for i in range(first, len(video)) if when(video[i]) < ended]
    target = participant.work / 'window.ivf'
    with participant.video.open('rb') as source, target.open('xb') as output:
        output.write(source.read(32)); index = 0
        while header := source.read(12):
            if len(header) != 12: raise RuntimeError('Truncated retained IVF header')
            size, timestamp = struct.unpack('<IQ', header)
            encoded = source.read(size)
            if len(encoded) != size: raise RuntimeError('Truncated retained IVF frame')
            if index in selected:
                first_timestamp = video[first].get('timestamp_ivf', video[first]['timestamp_rtp'])
                normalized = timestamp - first_timestamp if 'timestamp_ivf' in video[first] else (timestamp - first_timestamp) % 2**32
                output.write(struct.pack('<IQ', size, normalized)); output.write(encoded)
            index += 1
        if index != len(video): raise RuntimeError('IVF and numeric observation journal disagree')
    audio_target = participant.work / 'window.pcmu'
    with participant.audio.open('rb') as source, audio_target.open('xb') as output:
        for row in audio:
            encoded = source.read(row['bytes'])
            if len(encoded) != row['bytes']: raise RuntimeError('PCMU capture and observation journal disagree')
            if began <= when(row) < ended: output.write(encoded)
        if source.read(1): raise RuntimeError('Unjournaled PCMU samples')
    gaps = [when(video[b]) - when(video[a]) for a, b in zip(selected, selected[1:])]
    return target, audio_target, {'start_arrival_seconds': video[first]['arrival_seconds'], 'start_emit_seconds': began,
        'time_basis': 'actual live codec-input/file emission' if 'emit_seconds' in video[first] else 'arrival (legacy observation)', 'seconds': seconds,
        'complete_encoded_frames': len(selected), 'maximum_complete_frame_gap_seconds': max(gaps, default=None)}


def audiovisual_markers(video_path, audio_path, report, window, ffmpeg='ffmpeg', expected_source_id=None):
    """Independent decoded marker content correlated with recorded receive times.

    Measures receive synchronization, not application renderer/playout latency.
    """
    start = window.get('start_emit_seconds', window['start_arrival_seconds']); end = start + window['seconds']
    when = lambda row: row.get('emit_seconds', row['arrival_seconds'])
    audio_rows = [s for s in report['audio'] if start <= when(s) < end]
    pixels, metadata, diagnostics = decode_gray_frames(video_path, ffmpeg)
    mapped, mapping = map_decoded_frames(video_path, report, window, metadata)
    decoded = subprocess.run([ffmpeg, '-nostdin', '-hide_banner', '-loglevel', 'warning',
        '-f', 'mulaw', '-ar', '8000', '-ac', '1', '-i', str(audio_path),
        '-f', 'f32le', 'pipe:1'], capture_output=True, timeout=30)
    retain_decoder_diagnostics(audio_path, 'markers', decoded)
    if decoded.returncode: raise RuntimeError('Independent audiovisual marker decode failed')
    if len(pixels) != len(mapped) * 64:
        return {'status': 'unmapped', 'reason': 'Independent gray output and timestamped frame hashes disagree'}
    video_levels = []
    for index, (row, pixel_hash, _) in enumerate(mapped):
        values = pixels[index * 64:(index + 1) * 64]
        if hashlib.md5(values).hexdigest() != pixel_hash:
            return {'status': 'unmapped', 'reason': 'Independent gray pixels do not match the exact timestamped frame hash'}
        video_levels.append(min(values) > 240)
    video_edges = observed_rising_edges(video_levels, [when(row) for row, _, _ in mapped])
    samples = array.array('f'); samples.frombytes(decoded.stdout)
    audio_levels = []; offset = 0
    for row in audio_rows:
        values = samples[offset:offset + row['bytes']]; offset += row['bytes']
        audio_levels.append(math.sqrt(sum(x * x for x in values) / len(values)) > .055)
    if offset != len(samples): raise RuntimeError('Decoded audio and arrival journal disagree')
    audio_edges = observed_rising_edges(audio_levels, [when(row) for row in audio_rows])
    result = markers.ordinal_sync(video_edges, audio_edges, window['seconds'])
    result.update(
        time_basis='independent FFmpeg decoded white-video and PCMU gain edges correlated to actual live codec-input/file emission journal; physical speaker/display latency not measured',
        source_identity=source_identity(pixels, expected_source_id) if expected_source_id is not None else {'status': 'unobserved'},
        edge_method='requires independently observed low-to-high transition; initially high cropped levels are not counted as rising edges',
        decoder_mapping=mapping, video_decoder_diagnostics_bytes=diagnostics,
        video_decoder_exit_status=0, audio_decoder_exit_status=decoded.returncode)
    return result



def decode_recovery(video_path, report, window, bursts, ffmpeg='ffmpeg'):
    metadata, diagnostics = decode_hash_frames(video_path, ffmpeg)
    mapped, mapping = map_decoded_frames(video_path, report, window, metadata)
    start = window.get('start_emit_seconds', window['start_arrival_seconds']); end = start + window['seconds']
    when = lambda row: row.get('emit_seconds', row['arrival_seconds'])
    rows = [entry[0] for entry in mapped]
    decoded = [(report['unix_start_seconds'] + when(row), hash_value)
        for row, hash_value, _ in mapped]
    observed = []
    for burst in bursts:
        before = [(at, value) for at, value in decoded if at < burst['start_unix_seconds']]
        last = before[-1][1] if before else None
        key_index = next((index for index, (at, _) in enumerate(decoded)
            if at >= burst['end_unix_seconds'] and rows[index].get('keyframe')), None)
        # An interframe hash change can reflect corrupted prediction. Establish
        # a complete decoded post-burst keyframe and subsequent changing content.
        resumed = next(((at, value, index) for index, (at, value) in enumerate(decoded)
            if key_index is not None and index > key_index and value != decoded[key_index][1]
            and value != decoded[index - 1][1]), None)
        delay = resumed[0] - burst['end_unix_seconds'] if resumed else None
        observed.append({'burst': burst,
            'independently_decoded_keyframe_after_end_seconds': decoded[key_index][0] - burst['end_unix_seconds'] if key_index is not None else None,
            'keyframe_mapping': {'independent_decoder_frame_index': key_index,
                'retained_ivf_input_frame_index': mapped[key_index][2],
                'complete_frame_journal_index': report['video'].index(rows[key_index]),
                'received_keyframe_flag': rows[key_index]['keyframe'],
                'independently_decoded_pixel_hash': decoded[key_index][1]} if key_index is not None else None,
            'fresh_motion_independent_decoder_frame_index': resumed[2] if resumed else None,
            'fresh_decoded_content_after_end_seconds': delay,
            'status': 'passed' if last is not None and delay is not None and delay <= 2 else 'failed'})
    changing = [at for index, (at, value) in enumerate(decoded) if index == 0 or value != decoded[index - 1][1]]
    boundaries = [report['unix_start_seconds'] + start, *changing, report['unix_start_seconds'] + end]
    gaps = [b - a for a, b in zip(boundaries, boundaries[1:])]
    return {'status': 'passed' if observed and all(x['status'] == 'passed' for x in observed) else 'failed',
        'method': 'independently post-run decoded complete post-burst keyframe followed by changing pixels, linked by exact decoded PTS/IVF/live-journal mapping to actual codec emission wall-clock; decoder completion latency and physical render not measured',
        'feedback': 'native periodic keyframes at most one second; browser periodic encoded-worker keyframes plus supported feedback',
        'recovery_limit_seconds': 2, 'bursts': observed,
        'decoder_mapping': mapping,
        'decoder_exit_status': 0,
        'maximum_changing_decoded_frame_emission_gap_seconds': max(gaps, default=None),
        'decoder_diagnostics_bytes': diagnostics}


def decoded_mute_content(video_path, audio_path, report, window, ffmpeg='ffmpeg'):
    """Independent decoded content, with exact mapping to live emission times."""
    began = window.get('start_emit_seconds', window['start_arrival_seconds']); ended = began + window['seconds']
    when = lambda row: row.get('emit_seconds', row['arrival_seconds'])
    audio = [row for row in report['audio'] if began <= when(row) < ended]
    metadata, diagnostics = decode_hash_frames(video_path, ffmpeg)
    mapped, mapping = map_decoded_frames(video_path, report, window, metadata)
    video = [entry[0] for entry in mapped]; hashes = [entry[1] for entry in mapped]
    decoded = subprocess.run([ffmpeg, '-nostdin', '-hide_banner', '-loglevel', 'warning',
        '-f', 'mulaw', '-ar', '8000', '-ac', '1', '-i', str(audio_path), '-f', 'f32le', 'pipe:1'],
        capture_output=True, timeout=30)
    retain_decoder_diagnostics(audio_path, 'mute', decoded)
    samples = array.array('f'); samples.frombytes(decoded.stdout)
    if decoded.returncode or len(samples) != sum(row['bytes'] for row in audio):
        raise RuntimeError('Mute decoder and live observation journals cannot be mapped exactly')
    changing = [report['unix_start_seconds'] + when(video[index]) for index in range(1, len(video))
        if hashes[index] != hashes[index - 1]]
    levels = []; offset = 0
    for row in audio:
        values = samples[offset:offset + row['bytes']]; offset += row['bytes']
        levels.append({'unix_seconds': report['unix_start_seconds'] + when(row),
            'rms': math.sqrt(sum(value**2 for value in values) / len(values)),
            'frequency_hz': sum(a <= 0 < b for a, b in zip(values, values[1:])) * 8000 / len(values)})
    return {'window_start_unix_seconds': report['unix_start_seconds'] + began,
        'window_end_unix_seconds': report['unix_start_seconds'] + ended,
        'changing_times': changing, 'audio_samples': levels,
        'decoded_keyframe_times': [report['unix_start_seconds'] + when(row) for row in video if row['keyframe']],
        'decoder_mapping': mapping, 'video_decoder_diagnostics_bytes': diagnostics}


def mute_observation(content, transitions, expected_hz):
    """Actual sender setters must cause decoded freeze/silence and recovery."""
    began = content['window_start_unix_seconds']; ended = content['window_end_unix_seconds']
    changing = [at for at in content['changing_times'] if began <= at < ended]
    audio = [row for row in content['audio_samples'] if began <= row['unix_seconds'] < ended]
    intervals = []; audio_intervals = []; observations = []
    audible = lambda row: row['rms'] > .003 and abs(row['frequency_hz'] - expected_hz) <= 55
    for kind in ('video', 'audio'):
        events = [event for event in transitions if event['kind'] == kind]
        if len(events) != 2 or not events[0]['muted'] or events[1]['muted']:
            return {'status': 'failed', 'reason': 'Exactly one actual mute/unmute setter pair per kind is required'}
        on = events[0]['unix_seconds']; off = events[1]['unix_seconds']
        if not began <= on < off < ended or off - on < 1:
            return {'status': 'failed', 'reason': 'Actual setter interval is outside decoded observation window or too short'}
        steady_start = on + .5; steady_end = off - .1
        if kind == 'video':
            frozen = sum(steady_start <= at < steady_end for at in changing) == 0
            active = [row for row in audio if steady_start <= row['unix_seconds'] < steady_end and audible(row)]
            coexist = len(active) >= (steady_end - steady_start) * 5
            resumed = next((at for at in changing if at >= off), None)
            intervals.append((max(began, on - .2), min(ended, off + .2)))
            detail = {'changing_frames_while_muted': sum(steady_start <= at < steady_end for at in changing),
                'identifiable_audio_samples_while_camera_muted': len(active)}
            if 'decoded_keyframe_times' in content:
                keyframe = next((at for at in content['decoded_keyframe_times'] if at >= off), None)
                resumed = next((at for at in changing if keyframe is not None and at > keyframe), None)
                detail['independently_decoded_keyframe_after_unmute_seconds'] = keyframe - off if keyframe is not None else None
        else:
            audio_intervals.append((max(began, on - .2), min(ended, off + .2)))
            steady_audio = [row for row in audio if steady_start <= row['unix_seconds'] < steady_end]
            frozen = all(row['rms'] <= .003 for row in steady_audio)
            moving = sum(steady_start <= at < steady_end for at in changing)
            coexist = moving / (steady_end - steady_start) >= 24
            resumed = next((row['unix_seconds'] for row in audio if row['unix_seconds'] >= off and audible(row)), None)
            detail = {'maximum_decoded_rms_while_muted': max((row['rms'] for row in steady_audio), default=0),
                'changing_video_fps_while_microphone_muted': moving / (steady_end - steady_start)}
        recovery = resumed - off if resumed is not None else None
        observations.append(dict(detail, kind=kind, mute_unix_seconds=on, unmute_unix_seconds=off,
            recovery_seconds=recovery, counterpart_continued=coexist,
            status='passed' if frozen and coexist and recovery is not None and recovery <= 2 else 'failed'))
    available = ended - began - sum(stop - start for start, stop in intervals)
    if available <= 0: return {'status': 'failed', 'reason': 'No independently observed unmuted video interval'}
    active_fps = sum(not any(start <= at < stop for start, stop in intervals) for at in changing) / available
    available_audio = ended - began - sum(stop - start for start, stop in audio_intervals)
    unmuted_audio = [row for row in audio if not any(start <= row['unix_seconds'] < stop for start, stop in audio_intervals)]
    valid_audio = sum(audible(row) for row in unmuted_audio)
    audio_identified = valid_audio >= available_audio * 5 and valid_audio >= len(unmuted_audio) * .95
    return {'status': 'passed' if active_fps >= 24 and audio_identified and all(row['status'] == 'passed' for row in observations) else 'failed',
        'method': 'actual public sender setter timestamps compared with independently decoded changing video and audio RMS/frequency at live observation times',
        'unmuted_changing_video_fps': active_fps, 'unmuted_video_seconds': available,
        'decoder_mapping': content.get('decoder_mapping'),
        'identifiable_unmuted_audio': audio_identified,
        'limits': {'changing_video_fps': 24, 'recovery_seconds': 2, 'silence_rms': .003}, 'observations': observations}
