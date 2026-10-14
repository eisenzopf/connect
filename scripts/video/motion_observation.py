"""Fresh independently decoded content across the exact measured receive window."""
import math

LIMIT_SECONDS = 2
MAX_CHANGES = 10000


def motion_observation(content, mode='clean', transitions=(), bursts=()):
    began = content['window_start_unix_seconds']; ended = content['window_end_unix_seconds']
    times = content['changing_times']
    valid = (mode in ('clean', 'mute', 'impairment') and math.isfinite(began) and math.isfinite(ended)
        and ended > began and 0 < len(times) <= MAX_CHANGES
        and all(math.isfinite(at) for at in times)
        and all(b > a for a, b in zip(times, times[1:])))
    observed = [at for at in times if began <= at <= ended]
    result = {'status': 'failed', 'method': 'successive independently decoded pixel changes on actual receiver observation/emission times; no physical display or native decoder-completion claim',
        'freshness_limit_seconds': LIMIT_SECONDS, 'changing_time_bound': MAX_CHANGES,
        'changing_observations_in_window': len(observed), 'observed_exclusions': [],
        'active_segments': [], 'maximum_unexcluded_freshness_gap_seconds': None}
    if not valid or not observed: return result
    exclusions = []
    if mode == 'mute':
        events = [row for row in transitions if row['kind'] == 'video']
        if len(events) != 2 or not events[0]['muted'] or events[1]['muted']: return result
        exclusions = [(events[0]['unix_seconds'], events[1]['unix_seconds'], 'actual-camera-mute')]
    elif mode == 'impairment':
        if not bursts: return result
        exclusions = [(row['start_unix_seconds'], row['end_unix_seconds'], 'observed-network-burst') for row in bursts]
    if any(not math.isfinite(start) or not math.isfinite(stop) or not began < start < stop < ended
        for start, stop, _ in exclusions) or any(b[0] <= a[1] for a, b in zip(exclusions, exclusions[1:])):
        return result
    cursor = began; segments = []
    for start, stop, kind in exclusions:
        segments.append((cursor, start)); cursor = stop
        before = next((at for at in reversed(observed) if at < start), None)
        resumed = next((at for at in observed if at >= stop), None)
        passed = before is not None and start - before <= LIMIT_SECONDS and resumed is not None and resumed - stop <= LIMIT_SECONDS
        result['observed_exclusions'].append({'kind': kind, 'start_unix_seconds': start,
            'end_unix_seconds': stop, 'last_fresh_content_before_unix_seconds': before,
            'first_fresh_content_after_unix_seconds': resumed,
            'status': 'passed' if passed else 'failed'})
    segments.append((cursor, ended))
    for start, stop in segments:
        boundaries = [start] + [at for at in observed if start <= at <= stop] + [stop]
        gap = max(b - a for a, b in zip(boundaries, boundaries[1:]))
        result['active_segments'].append({'start_unix_seconds': start, 'end_unix_seconds': stop,
            'maximum_freshness_gap_seconds': gap, 'status': 'passed' if gap <= LIMIT_SECONDS else 'failed'})
    result['maximum_unexcluded_freshness_gap_seconds'] = max(row['maximum_freshness_gap_seconds'] for row in result['active_segments'])
    result['status'] = 'passed' if all(row['status'] == 'passed' for row in
        result['active_segments'] + result['observed_exclusions']) else 'failed'
    return result
