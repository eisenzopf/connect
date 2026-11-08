"""Account only for measured gaps across original public ownership cuts."""
import math

TRANSITION_PROFILE = 'connect/media-transition-coverage/1'


def transition_coverage(times, cuts, *, start_ms, end_ms, frame_ms, minimum_gap_ms):
    number = lambda value: type(value) in (int, float) and math.isfinite(value)
    if (not isinstance(times, list) or len(times) > 16000
            or not isinstance(cuts, list) or len(cuts) > 2
            or not all(number(value) for value in (start_ms, end_ms, frame_ms, minimum_gap_ms))
            or start_ms >= end_ms or frame_ms <= 0 or minimum_gap_ms <= frame_ms):
        raise ValueError('Invalid transition coverage bounds')
    if any(not number(at) or not start_ms <= at < end_ms
            or i and at < times[i-1] for i, at in enumerate(times)):
        raise ValueError('Original media arrivals must be ordered and bounded')
    if (any(not isinstance(cut.get('id'), str) or not cut['id']
            or not number(cut.get('atMs')) or not start_ms < cut['atMs'] < end_ms
            or i and cut['atMs'] <= cuts[i-1]['atMs'] for i, cut in enumerate(cuts))
            or len({cut['id'] for cut in cuts}) != len(cuts)):
        raise ValueError('Original public ownership cuts must be unique and ordered')
    boundaries = [start_ms, *times, end_ms]
    used, transitions = set(), []
    maximum_clean_gap = 0
    for i in range(1, len(boundaries)):
        left, right = boundaries[i-1:i+1]
        span = right-left
        matching = [cut for cut in cuts if left <= cut['atMs'] < right]
        if (i > 1 and i < len(boundaries)-1 and span > minimum_gap_ms
                and len(matching) == 1 and matching[0]['id'] not in used and span <= 1000):
            cut = matching[0]
            used.add(cut['id'])
            transitions.append(dict(id=cut['id'], cut_ms=cut['atMs'], last_sample_ms=left,
                resumed_sample_ms=right, interruption_ms=span,
                missing_start_ms=left+frame_ms, missing_end_ms=right))
        else:
            maximum_clean_gap = max(maximum_clean_gap, span)
    return dict(profile=TRANSITION_PROFILE, maximum_interruption_ms=1000,
        original_public_cuts=[dict(cut) for cut in cuts], transitions=transitions,
        transition_missing_ms=sum(gap['missing_end_ms']-gap['missing_start_ms'] for gap in transitions),
        maximum_clean_gap_ms=maximum_clean_gap)


def clean_bin_durations(coverage, start_ms, seconds):
    return [1000-sum(max(0, min(start_ms+(i+1)*1000, gap['missing_end_ms'])
        - max(start_ms+i*1000, gap['missing_start_ms'])) for gap in coverage['transitions'])
        for i in range(seconds)]
