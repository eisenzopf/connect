"""Bounded browser decoded-audio identity and explicitly observed fault windows."""
import math

SAMPLE_INTERVAL = .1
MAX_GAP = .2
MIN_SAMPLE_FRACTION = .9


def audio_observation(content, expected_hz, mode='clean', transitions=(), bursts=()):
    began = content['window_start_unix_seconds']; ended = content['window_end_unix_seconds']
    samples = [row for row in content['samples'] if began <= row['unix_seconds'] <= ended]
    identified = lambda row: (math.isfinite(row['rms']) and math.isfinite(row['frequency_hz'])
        and row['rms'] > .001 and abs(row['frequency_hz'] - expected_hz) < 35)
    ordered = all(b['unix_seconds'] > a['unix_seconds'] for a, b in zip(samples, samples[1:]))
    boundaries = [began] + [row['unix_seconds'] for row in samples] + [ended]
    gaps = [b - a for a, b in zip(boundaries, boundaries[1:])]
    covered = (ended > began and ordered and len(samples) <= 2000
        and len(samples) >= math.floor((ended - began) / SAMPLE_INTERVAL) * MIN_SAMPLE_FRACTION
        and max(gaps, default=math.inf) <= MAX_GAP)
    result = {'status': 'failed', 'method': 'throughout-window independent received AudioContext RMS/frequency samples; no physical speaker claim',
        'expected_frequency_hz': expected_hz, 'samples': len(samples), 'coverage_complete': covered,
        'maximum_sampling_gap_seconds': max(gaps, default=None), 'configured_interval_seconds': SAMPLE_INTERVAL,
        'coverage_bounds': {'maximum_samples': 2000, 'minimum_expected_sample_fraction': MIN_SAMPLE_FRACTION,
            'maximum_gap_seconds': MAX_GAP}, 'observed_exceptions': [], 'bad_unexcepted_samples': 0,
        'limits': {'identified_rms_above': .001, 'frequency_error_below_hz': 35,
            'recovery_seconds': 2, 'mute_silence_rms_at_most': .003, 'mute_observation_seconds': .5}}
    if mode not in ('clean', 'mute', 'impairment') or not covered: return result
    exceptions = []; fault_checks = []
    if mode == 'mute':
        events = [event for event in transitions if event['kind'] == 'audio']
        if len(events) != 2 or not events[0]['muted'] or events[1]['muted']: return result
        on, off = [event['unix_seconds'] for event in events]
        if not began <= on < off < ended: return result
        silence = next((row['unix_seconds'] for row in samples if row['unix_seconds'] >= on and row['rms'] <= .003), None)
        resumed = next((row['unix_seconds'] for row in samples if row['unix_seconds'] >= off and identified(row)), None)
        valid = (silence is not None and resumed is not None and silence <= on + .5
            and silence < off and resumed - off <= 2)
        if valid:
            muted = [row for row in samples if silence <= row['unix_seconds'] < off]
            valid = bool(muted) and all(row['rms'] <= .003 for row in muted)
            exceptions.append((silence, resumed))
        fault_checks.append({'kind': 'actual-microphone-mute', 'mute_unix_seconds': on, 'unmute_unix_seconds': off,
            'observed_silence_start_unix_seconds': silence, 'observed_identified_recovery_unix_seconds': resumed,
            'status': 'passed' if valid else 'failed'})
    elif mode == 'impairment':
        # A residual buffered tone immediately after a network burst is not a
        # stable return: decoded attenuation can arrive later. Inspect the
        # existing two-second aftermath, exempting only finite near-silence.
        attenuated = lambda row: (math.isfinite(row['rms']) and 0 <= row['rms'] <= .001
            and math.isfinite(row['frequency_hz']))
        previous_deadline = -math.inf
        for burst in bursts:
            start, stop = burst['start_unix_seconds'], burst['end_unix_seconds']
            deadline = stop + 2
            before = next((row for row in reversed(samples) if row['unix_seconds'] < start), None)
            aftermath = [row for row in samples if start <= row['unix_seconds'] <= deadline]
            lows = [row for row in aftermath if attenuated(row)]
            last_low = lows[-1]['unix_seconds'] if lows else None
            first = next((row['unix_seconds'] for row in aftermath if row['unix_seconds'] >= stop and identified(row)), None)
            resumed = next((row['unix_seconds'] for row in aftermath if row['unix_seconds'] >= stop
                and (last_low is None or row['unix_seconds'] > last_low) and identified(row)), None)
            witness = next((row['unix_seconds'] for row in samples if resumed is not None
                and row['unix_seconds'] >= resumed + MAX_GAP and identified(row)), None)
            valid = (began <= start < stop and deadline <= ended and start > previous_deadline
                and before is not None and identified(before) and start - before['unix_seconds'] <= MAX_GAP
                and resumed is not None and resumed <= deadline and witness is not None
                and all(identified(row) or attenuated(row) for row in aftermath)
                and all(identified(row) for row in samples if resumed <= row['unix_seconds'] <= witness))
            previous_deadline = deadline
            if valid: exceptions.append((start, resumed))
            fault_checks.append({'kind': 'observed-network-burst', 'start_unix_seconds': start, 'end_unix_seconds': stop,
                'recovery_deadline_unix_seconds': deadline, 'first_identified_after_burst_unix_seconds': first,
                'last_attenuation_unix_seconds': last_low, 'attenuated_samples': lows,
                'observed_identified_recovery_unix_seconds': resumed,
                'identified_continuation_witness_unix_seconds': witness, 'status': 'passed' if valid else 'failed'})
    def excepted(row):
        return (any(start <= row['unix_seconds'] < stop for start, stop in exceptions)
            and (mode != 'impairment' or attenuated(row)))
    bad = [row for row in samples if not excepted(row) and not identified(row)]
    if mode == 'impairment':
        result.update(observed_bad_samples=sum(not identified(row) for row in samples),
            excepted_attenuation_samples=sum(excepted(row) for row in samples),
            recovery_method='last finite near-silence in the observed two-second aftermath, then identified return and covered continuation; wrong tone is never excepted')
    result.update(observed_exceptions=fault_checks, bad_unexcepted_samples=len(bad),
        status='passed' if not bad and all(row['status'] == 'passed' for row in fault_checks) else 'failed')
    return result
