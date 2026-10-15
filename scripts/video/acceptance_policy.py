"""Decision 012 acceptance; decoded A/V evidence is never rewritten."""
import importlib.util
import math
import pathlib

_SPEC = importlib.util.spec_from_file_location('acceptance_marker_calibration', pathlib.Path(__file__).with_name('marker_calibration.py'))
_MARKERS = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_MARKERS)
POLICY = 'connect-media-acceptance/2'
_DISCRETE = {'video_markers', 'audio_markers', 'paired', 'ordinal', 'ordinals',
             'candidate_peak', 'candidate_bound', 'missing_internal_markers',
             'candidate_assignments', 'unmatched_video_markers', 'unmatched_audio_markers'}


def _number(value):
    return type(value) in (int, float) and math.isfinite(value)


def _matches(actual, expected, field=None):
    # Canonical calculation controls limits. This tolerance only accommodates
    # cross-language floating point roundoff in retained derived metadata.
    if _number(expected):
        if field in _DISCRETE:
            return _number(actual) and actual == expected
        return _number(actual) and abs(actual - expected) <= 1e-7
    if isinstance(expected, dict):
        return isinstance(actual, dict) and all(key in actual and _matches(actual[key], value, key)
                                                for key, value in expected.items())
    if isinstance(expected, list):
        return isinstance(actual, list) and len(actual) == len(expected) and all(
            _matches(a, b, field) for a, b in zip(actual, expected))
    return type(actual) is type(expected) and actual == expected


def audio_video_acceptance(sync, scenario, seconds):
    """Accept valid impaired quality degradation, never malformed evidence.

    Source identity, audio, motion, recovery, lifecycle and resource predicates
    remain mandatory at the calling participant/route boundary.
    """
    result = {'policy': POLICY, 'scenario': scenario, 'status': 'failed',
              'quality_status': 'invalid', 'reason': 'invalid-observation'}
    if scenario not in ('clean', 'mute', 'impairment'):
        return dict(result, reason='invalid-scenario')
    if not _number(seconds) or not 1 <= seconds <= 120:
        return dict(result, reason='invalid-duration')
    if not isinstance(sync, dict):
        return result
    edges = []
    for kind in ('video', 'audio'):
        values = sync.get(kind + '_edge_seconds')
        count = sync.get(kind + '_markers')
        if (not isinstance(values, list) or len(values) > 512 or not _number(count) or count != int(count)
                or count != len(values) or any(not _number(v) for v in values)
                or any(b <= a for a, b in zip(values, values[1:]))):
            return result
        edges.append(values)
    if 'audio_envelope' in sync:
        envelope = sync['audio_envelope']
        if (not isinstance(envelope, dict) or envelope.get('status') != 'observer-valid'
                or envelope.get('invalid') != [] or envelope.get('overflow') is not False
                or envelope.get('invalid_input') is not False):
            return dict(result, reason='invalid-observer')
    if 'observer_failure' in sync:
        return dict(result, reason='invalid-observer')
    canonical = _MARKERS.ordinal_sync(*edges, seconds)
    if not _matches(sync, canonical):
        return dict(result, reason='inconsistent-observation')
    result['quality_status'] = canonical['status']
    if canonical['status'] == 'passed':
        return dict(result, status='passed', reason='quality-met')
    if scenario == 'impairment':
        return dict(result, status='passed', reason='impaired-quality-advisory')
    return dict(result, reason='strict-quality-not-met')
