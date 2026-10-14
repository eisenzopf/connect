"""Ordinal synchronization from independently decoded nonperiodic marker cadence.

Calibration uses only decoded rising edges and the fixed fixture code. Random
RTP origins, nearest pulse pairing and sender wall clocks do not assign ordinals.
"""
import bisect
import math
import statistics

PROGRAM = 'connect-av-cadence/1'
HORIZON_SECONDS = 300
MAX_MARKERS = 512
MAX_STATES = 1024
CALIBRATION_TOLERANCE_SECONDS = .15


def marker_frames():
    # B(3,6): every six-interval word occurs once before the code repeats.
    digits = [0] * 19; code = []
    def visit(at, period):
        if at > 6:
            if 6 % period == 0: code.extend(digits[1:period + 1])
        else:
            digits[at] = digits[at - period]; visit(at + 1, period)
            for value in range(digits[at - period] + 1, 3):
                digits[at] = value; visit(at + 1, at)
    visit(1, 1)
    at = 9; frames = []
    for value in code:
        if at >= HORIZON_SECONDS * 30: break
        frames.append(at); at += [15, 27, 39][value]
    if len(frames) > MAX_MARKERS: raise RuntimeError('Coded marker generator exceeded its bound')
    return frames


def calibrate(edges, schedule=None):
    schedule = [frame / 30 for frame in marker_frames()] if schedule is None else schedule
    if (not 7 <= len(edges) <= MAX_MARKERS or not schedule or len(schedule) > MAX_MARKERS
            or any(not math.isfinite(at) for at in edges)
            or any(b <= a for a, b in zip(edges, edges[1:]))):
        return {'status': 'missing-proof', 'reason': 'At least seven strictly ordered proven rising edges are required'}
    tolerance = CALIBRATION_TOLERANCE_SECONDS
    states = [((ordinal,), edges[0] - at - tolerance, edges[0] - at + tolerance)
        for ordinal, at in enumerate(schedule)]
    peak = len(states)
    for observed in edges[1:]:
        following = []
        for ordinals, lower, upper in states:
            first = max(ordinals[-1] + 1, bisect.bisect_left(schedule, observed - upper - tolerance))
            stop = bisect.bisect_right(schedule, observed - lower + tolerance)
            for ordinal in range(first, stop):
                offset = observed - schedule[ordinal]
                lo = max(lower, offset - tolerance); hi = min(upper, offset + tolerance)
                if lo <= hi: following.append((ordinals + (ordinal,), lo, hi))
                if len(following) > MAX_STATES:
                    return {'status': 'ambiguous', 'reason': 'Calibration candidate bound exceeded', 'candidate_bound': MAX_STATES}
        states = following; peak = max(peak, len(states))
        if not states: return {'status': 'missing-proof', 'reason': 'Decoded edges do not fit the coded cadence'}
    if len(states) != 1:
        return {'status': 'ambiguous', 'reason': 'Multiple ordinal assignments fit decoded edges',
            'candidate_assignments': len(states), 'candidate_bound': MAX_STATES}
    ordinals, lower, upper = states[0]
    return {'status': 'calibrated', 'ordinals': list(ordinals), 'edges_seconds': list(edges),
        'source_offset_interval_seconds': [lower, upper], 'candidate_peak': peak,
        'candidate_bound': MAX_STATES, 'missing_internal_markers': ordinals[-1] - ordinals[0] + 1 - len(ordinals)}


def ordinal_sync(video_edges, audio_edges, seconds):
    video = calibrate(video_edges); audio = calibrate(audio_edges)
    result = {'status': 'failed', 'program': PROGRAM,
        'method': 'independently decoded low-to-high edges uniquely calibrated against nonperiodic cadence; only equal source ordinals are paired, never nearest periodic edges',
        'video_markers': len(video_edges), 'audio_markers': len(audio_edges),
        'video_edge_seconds': list(video_edges) if len(video_edges) <= MAX_MARKERS and all(math.isfinite(at) for at in video_edges) else [],
        'audio_edge_seconds': list(audio_edges) if len(audio_edges) <= MAX_MARKERS and all(math.isfinite(at) for at in audio_edges) else [],
        'video_calibration': video, 'audio_calibration': audio, 'paired': 0,
        'marker_pairs': [], 'median_abs_skew_ms': None, 'p95_abs_skew_ms': None,
        'calibration_tolerance_seconds': CALIBRATION_TOLERANCE_SECONDS,
        'limits': {'median_abs_skew_ms': 80, 'p95_abs_skew_ms': 120},
        'quantile_method': 'median averages the two middle values; p95 uses nearest rank'}
    if video['status'] != 'calibrated' or audio['status'] != 'calibrated': return result
    by_audio = dict(zip(audio['ordinals'], audio_edges))
    pairs = [{'ordinal': ordinal, 'video_emit_seconds': at,
        'audio_emit_seconds': by_audio[ordinal], 'signed_video_minus_audio_ms': (at - by_audio[ordinal]) * 1000}
        for ordinal, at in zip(video['ordinals'], video_edges) if ordinal in by_audio]
    values = sorted(abs(pair['signed_video_minus_audio_ms']) for pair in pairs)
    result.update(marker_pairs=pairs, paired=len(pairs),
        unmatched_video_markers=len(video_edges) - len(pairs), unmatched_audio_markers=len(audio_edges) - len(pairs))
    if values:
        result.update(median_abs_skew_ms=statistics.median(values), p95_abs_skew_ms=values[math.ceil(len(values) * .95) - 1])
        result['status'] = 'passed' if len(values) >= seconds * .7 and result['median_abs_skew_ms'] <= 80 and result['p95_abs_skew_ms'] <= 120 else 'failed'
    return result
