"""Three original workload cycles: joined Session owners and bounded warm residency.

OS residency does not identify live objects. Every original ten-second cleanup
window remains required, and missing evidence is a failure. No producer pass
flag, allocator operation, process replacement or cold-RSS waiver is inferred.
"""
import math
import re

CYCLE_POLICY = dict(profile="connect/matrix-idle-ownership/1", idle_seconds=10,
    sample_seconds=1, session_physical_owners=0, session_owned_tasks=0,
    scope="Original Session owners/tasks return to zero and resource pools to their pre-workload usage within ten seconds; warm process residency is evaluated across three original cycles.")
POLICY = dict(profile="connect/matrix-resource-plateau/1", cycles=3,
    idle_seconds=10, sample_seconds=1, rss_variance_floor_bytes=8*1024*1024,
    rss_variance_fraction=.05, rss_monotonic_step_floor_bytes=1024*1024,
    rss_persistent_step_ratio_min=.75,
    later_extra_processes=0, later_extra_threads=0, later_extra_descriptors=0,
    scope="Three real cycles on the same persistent original owners. Cycle one includes cold initialization; later idle RSS is bounded by max(8MiB,5% of cycle-one idle RSS), and two consecutive rises of at least1MiB fail when the second is at least75% of the first. Later process/thread/descriptor counts cannot exceed cycle-one idle counts. No forced GC, allocator or pool tuning.")
COUNTS = ("live_processes", "sampled_threads", "sampled_open_descriptors")
MEASURES = (*COUNTS, "sampled_rss_bytes")
HEX = re.compile(r"[0-9a-f]{64}")


def _number(value):
    return type(value) in (int, float) and math.isfinite(value)


def _valid_usage(usage):
    return isinstance(usage, dict) and {'physical_owners', 'owned_tasks'} <= usage.keys() and all((type(v) is int and v >= 0 for v in usage.values()))


def _valid_groups(groups):
    return isinstance(groups, dict) and 'server' in groups and (len(groups) >= 2) and all((isinstance(g, dict) and g.get('root_state') == 'available' and (g.get('unavailable_processes') == 0) and (g.get('open_resource_unavailable_processes') == 0) and all((type(g.get(k)) is int and g[k] >= 0 for k in MEASURES)) and (g['live_processes'] > 0) and (g['sampled_rss_bytes'] > 0) for g in groups.values()))


def cycle_restoration(baseline, sample):
    """Session ownership only; OS counter availability is mandatory, not cold equality."""
    try:
        if not isinstance(baseline, dict) or not isinstance(sample, dict):
            return ['incomplete-resource-observation']
        if baseline.get('error') or sample.get('error'):
            return ['unavailable-observation']
        if not _valid_usage(baseline['usage']) or not _valid_usage(sample['usage']):
            return ['invalid-resource-usage']
        if baseline['usage']['physical_owners'] or baseline['usage']['owned_tasks']:
            return ['nonzero-original-session-owner-baseline']
        if not _valid_groups(baseline['groups']) or not _valid_groups(sample['groups']):
            return ['unavailable-original-process-observation']
        errors = []
        if set(sample['groups']) != set(baseline['groups']):
            errors.append('changed-process-role-inventory')
        if set(sample['usage']) != set(baseline['usage']):
            errors.append('changed-resource-pool-inventory')
        errors.extend((k for k, v in sample['usage'].items() if k not in baseline['usage'] or v > baseline['usage'][k]))
        return errors
    except (KeyError, TypeError, ValueError):
        return ['incomplete-resource-observation']


def _idle_window(report, baseline=None):
    if report['format'] not in {'connect/matrix-idle-observation/1', 'connect/native-matrix-idle-observation/2'}:
        raise ValueError('idle-format')
    if report['policy'] != CYCLE_POLICY:
        raise ValueError('idle-policy')
    original = baseline if baseline is not None else report['baseline']
    terminal = report['terminal_monotonic_seconds']
    samples = report['samples']
    if not _number(terminal) or not _number(report['baseline']['monotonic_seconds']):
        raise ValueError('idle-time')
    if report['baseline']['monotonic_seconds'] >= terminal or cycle_restoration(original, report['baseline']):
        raise ValueError('idle-baseline')
    if not 4 <= len(samples) <= 300:
        raise ValueError('idle-sample-count')
    if any((not _number(s['monotonic_seconds']) for s in samples)):
        raise ValueError('idle-sample-time')
    if any((a['monotonic_seconds'] >= b['monotonic_seconds'] for a, b in zip(samples, samples[1:]))):
        raise ValueError('idle-sample-order')
    final = samples[-1]
    if final['stage'] != 'idle-final' or not 10 <= final['monotonic_seconds'] - terminal <= 12:
        raise ValueError('idle-final-window')
    before = [s for s in samples if s['stage'] == 'idle' and 0 <= s['monotonic_seconds'] - terminal <= 10]
    if len(before) < 2:
        raise ValueError('idle-stable-samples')
    stable = before[-2:] + [final]
    if cycle_restoration(original, original) or any((cycle_restoration(original, s) for s in stable)):
        raise ValueError('idle-session-owners-or-unavailable-counters')
    return stable


def cycle_idle_verdict(report, baseline=None):
    try:
        _idle_window(report, baseline)
        return True
    except (KeyError, TypeError, ValueError, IndexError):
        return False


def _valid_owner(owner):

    def birth(value):
        if type(value) is int:
            return value > 0
        if isinstance(value, str):
            return HEX.fullmatch(value) is not None
        return isinstance(value, (tuple, list)) and 1 <= len(value) <= 4 and all((type(v) is int and v >= 0 for v in value)) and any(value)
    return isinstance(owner, dict) and type(owner.get('pid')) is int and (owner['pid'] > 0) and any((k in owner and birth(owner[k]) for k in ('start_ticks', 'start', 'birth', 'start_identity')))


def evaluate(report):
    """Recompute independent resource qualification; never clears media failures."""
    result = dict(format='connect/matrix-resource-plateau-verdict/1', passed=False, failures=[], comparisons=[], policy=POLICY)
    errors = result['failures']
    try:
        if report['format'] != 'connect/matrix-plateau-observation/1':
            raise ValueError('plateau-format')
        if report['policy'] != POLICY:
            raise ValueError('plateau-policy')
        baseline, owners, cycles = (report['baseline'], report['persistent_owners'], report['cycles'])
        if cycle_restoration(baseline, baseline):
            raise ValueError('original-baseline')
        if not isinstance(owners, dict) or set(owners) != set(baseline['groups']) or (not all((_valid_owner(v) for v in owners.values()))):
            raise ValueError('original-persistent-owner-identities')
        if len(cycles) != 3:
            raise ValueError('exactly-three-original-cycles-required')
        workloads = set()
        evidence = set()
        join_evidence = set()
        windows = []
        previous_final = baseline['monotonic_seconds']
        for index, cycle in enumerate(cycles, 1):
            label = 'cycle-' + str(index)
            if type(cycle['index']) is not int or cycle['index'] != index:
                raise ValueError(label + ':index')
            if cycle['persistent_owners'] != owners:
                raise ValueError(label + ':changed-original-owner')
            wid = cycle['workload_id']
            proof = cycle['workload_evidence_sha256']
            if not isinstance(wid, str) or not wid or len(wid) > 256 or (wid in workloads) or (cycle['workload_started'] is not True):
                raise ValueError(label + ':original-workload-missing-or-reused')
            if not isinstance(proof, str) or not HEX.fullmatch(proof) or proof in evidence:
                raise ValueError(label + ':original-workload-evidence-missing-or-reused')
            workloads.add(wid)
            evidence.add(proof)
            if cycle['workload_complete'] is not True:
                raise ValueError(label + ':original-workload-window-incomplete')
            joins = cycle['joins']
            if joins['session_owners_joined'] is not True or joins['helper_work_joined'] is not True or joins['forced'] is not False or (not isinstance(joins['evidence_sha256'], str)) or (not HEX.fullmatch(joins['evidence_sha256'])):
                raise ValueError(label + ':original-session-or-helper-work-unjoined')
            if joins['evidence_sha256'] in join_evidence:
                raise ValueError(label + ':original-join-evidence-reused')
            join_evidence.add(joins['evidence_sha256'])
            idle = cycle['idle_resources']
            fresh_baseline = idle['baseline']['monotonic_seconds']
            if not _number(previous_final) or not _number(fresh_baseline) or fresh_baseline < previous_final or (idle['terminal_monotonic_seconds'] <= previous_final):
                raise ValueError(label + ':overlapping-or-stale-original-cycle')
            window = _idle_window(idle, baseline)
            windows.append(window)
            previous_final = window[-1]['monotonic_seconds']
        # Fix the warm reference after the first real workload. Later growth
        # never resets it. Each window includes all three final idle samples.
        for role in sorted(owners):
            envelope = [{field: max((s['groups'][role][field] for s in w)) for field in MEASURES} for w in windows]
            for field in COUNTS:
                values = [e[field] for e in envelope]
                result['comparisons'].append(dict(role=role, metric=field, values=values, max_later_increase=0))
                if any((v > values[0] for v in values[1:])):
                    errors.append(role + ':' + field + ':later-idle-growth')
            values = [e['sampled_rss_bytes'] for e in envelope]
            allowance = max(POLICY['rss_variance_floor_bytes'], math.ceil(values[0] * POLICY['rss_variance_fraction']))
            steps = [b - a for a, b in zip(values, values[1:])]
            monotonic = all((step >= POLICY['rss_monotonic_step_floor_bytes'] for step in steps)) and steps[1] >= steps[0] * POLICY['rss_persistent_step_ratio_min']
            result['comparisons'].append(dict(role=role, metric='sampled_rss_bytes', values=values, cycle_one_reference_bytes=values[0], variance_allowance_bytes=allowance, idle_to_idle_steps_bytes=steps, monotonic_step_floor_bytes=POLICY['rss_monotonic_step_floor_bytes'], persistent_step_ratio_min=POLICY['rss_persistent_step_ratio_min'], persistent_positive_trend=monotonic))
            if any((v > values[0] + allowance for v in values[1:])):
                errors.append(role + ':sampled_rss_bytes:variance-bound')
            if monotonic:
                errors.append(role + ':sampled_rss_bytes:persistent-positive-trend')
        result['passed'] = not errors
    except (KeyError, TypeError, ValueError, IndexError, OverflowError) as error:
        errors.append(str(error) if isinstance(error, ValueError) else 'incomplete-or-invalid-plateau-evidence:' + type(error).__name__)
    return result


def plateau_verdict(report):
    return evaluate(report)['passed']
