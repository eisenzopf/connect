"""Three original workload cycles: joined Session owners and bounded warm residency.

OS residency does not identify live objects. Every original ten-second cleanup
window remains required, and missing evidence is a failure. No producer pass
flag, allocator operation, process replacement or cold-RSS waiver is inferred.
"""
import math
import hashlib
from pathlib import Path
import re

CYCLE_POLICY = dict(profile="connect/matrix-idle-ownership/1", idle_seconds=10,
    sample_seconds=1, session_physical_owners=0, session_owned_tasks=0,
    scope="Original Session owners/tasks return to zero and resource pools to their pre-workload usage within ten seconds; warm process residency is evaluated across three original cycles.")
# These are NEW engineering operating budgets, not measured allocator limits or
# OS-enforced quotas. Count capacity is frozen from the first original workload.
GIB = 1024 * 1024 * 1024
SERVER_SOURCE_INPUTS = {
    "crates/connect-server/src/main.rs": "acd9b8a3eb7e80342e38d08055e2dd3cb6018b6cc9ed2b6223cae5d3e6dd0a58",
    "Cargo.lock": "e9e8e6143b6fa2d40efa95532ecabeb41b1712ad54f98a3d930f7e093a5a3c0f"}
SERVER_RUNTIME_CONFIGURATION = dict(
    format="connect/matrix-server-runtime-capacity/1",
    runtime="tokio", version="1.53.2", builder="tokio::main-default-multi-thread",
    max_blocking_threads=512,
    connect_main_sha256="acd9b8a3eb7e80342e38d08055e2dd3cb6018b6cc9ed2b6223cae5d3e6dd0a58",
    tokio_package_checksum="e95f91fcc7a621e8b030f6aa23c71fe9838ae2fb4d8118b75602a328f5144044",
    tokio_builder_sha256="352db30f01ecd4c5b80ad03780124c6d7241fa1538e3f83497f024f253d957d5")
POLICY = dict(profile="connect/matrix-resource-capacity/4", cycles=3,
    idle_seconds=10, sample_seconds=1,
    role_rss_budget_bytes=dict(server=GIB, browser=4*GIB, helper=GIB),
    aggregate_rss_budget_bytes=6*GIB,
    count_capacity="First original workload samples, frozen before later cycles. Server threads use original baseline total plus the exact source-bound default blocking-pool capacity.",
    rss_trend="Diagnostic only. Last-three-sample idle intervals. A later rise must be unseparated at observed resolution or the second positive increment must be strictly smaller than the first across the complete observed intervals.",
    scope="Owner-approved capacity and strict ownership profile. Three-cycle RSS trends are diagnostic, not a qualification gate. Sustained growth assessment remains pending under G6-098. No proof of object liveness or absence of a slowly decaying leak. Original Session ownership, task and pool restoration, all actual joins, fixed identities, complete workloads and ten-second cleanup remain required. No forced GC, allocator or pool tuning.")
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


def _workload_samples(idle, roles):
    baseline = idle['baseline']['monotonic_seconds']
    terminal = idle['terminal_monotonic_seconds']
    original = [s for s in idle['samples'] if s['stage'] == 'workload']
    valid, skipped = [], {}
    for sample in original:
        if not baseline <= sample['monotonic_seconds'] < terminal:
            # The original terminal may arrive after the numeric sample was
            # labelled. Never admit that late sample to workload capacity.
            reason = 'outside-original-workload-time'
            skipped[reason] = skipped.get(reason, 0) + 1
            continue
        if (sample.get('error') or not _valid_groups(sample.get('groups')) or
                set(sample['groups']) != roles):
            reason = 'unavailable-or-incomplete-original-quantitative-sample'
            skipped[reason] = skipped.get(reason, 0) + 1
        else:
            valid.append(sample)
    if not valid:
        raise ValueError('workload-capacity-valid-samples-missing')
    return dict(valid=valid, original=original, skipped=skipped)


def _known_rss(sample, role):
    groups = sample.get('groups') if isinstance(sample, dict) else None
    group = groups.get(role) if isinstance(groups, dict) else None
    value = group.get('sampled_rss_bytes') if isinstance(group, dict) else None
    return value if type(value) is int and value > 0 else None


def _rss_intervals(windows, role):
    intervals = [dict(low=min(s['groups'][role]['sampled_rss_bytes'] for s in w),
                      high=max(s['groups'][role]['sampled_rss_bytes'] for s in w)) for w in windows]
    first, second, third = intervals
    increments = [dict(low=b['low']-a['high'], high=b['high']-a['low'])
                  for a,b in zip(intervals,intervals[1:])]
    # Overlap is only a statement about the observed resolution, not a
    # statistical confidence interval. Strict contraction has no fitted ratio.
    later_rise_unseparated = third['low'] <= second['high']
    demonstrated_contraction = (increments[0]['low'] > 0 and
        0 < increments[1]['high'] < increments[0]['low'])
    return dict(intervals=intervals, increments=increments,
        later_rise_unseparated=later_rise_unseparated,
        demonstrated_contraction=demonstrated_contraction,
        passed=later_rise_unseparated or demonstrated_contraction)


def runtime_capacity_configuration(owner, source_inputs=None):
    """Source-bound capacity, never a claim about actual blocking-worker idleness.

    Producers use their current source files. Independent verifiers may pass
    their already-recomputed source_inputs for the exact archived candidate.
    A missing/different source binding remains unavailable, not a default guess.
    """
    try:
        if source_inputs is None:
            root = Path(__file__).resolve().parents[2]
            source_inputs = {name: hashlib.sha256((root / name).read_bytes()).hexdigest()
                             for name in SERVER_SOURCE_INPUTS}
        observed = {name: source_inputs[name] for name in SERVER_SOURCE_INPUTS}
        if observed != SERVER_SOURCE_INPUTS:
            raise ValueError('source-configuration-changed')
        return dict(SERVER_RUNTIME_CONFIGURATION, original_owner=owner, source_inputs=observed)
    except (OSError, KeyError, TypeError, ValueError) as error:
        return dict(format=SERVER_RUNTIME_CONFIGURATION['format'], available=False,
                    error=type(error).__name__)


def _server_thread_ceiling(report, owners, source_inputs):
    # The source binding is required, rather than silently assuming a 512 cap.
    # This bounds capacity ONLY. Zero owned tasks/actual joins independently
    # prove Session cleanup. This does not claim all work on every OS thread is idle.
    configuration = report['server_runtime_capacity']
    expected = runtime_capacity_configuration(owners['server'], source_inputs)
    if expected.get('available') is False or configuration != expected:
        raise ValueError('server-runtime-capacity-configuration')
    return (report['baseline']['groups']['server']['sampled_threads'] +
            configuration['max_blocking_threads'])


def evaluate(report, source_inputs=None):
    """Recompute independent resource qualification; never clears media failures."""
    result = dict(format='connect/matrix-resource-plateau-verdict/1', passed=False, failures=[], diagnostics=[], comparisons=[], policy=POLICY, separate_soak=dict(task='G6-098', status='pending', qualified=False))
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
        roles = set(owners)
        if roles not in ({'server', 'browser'}, {'server', 'helper'}):
            raise ValueError('unsupported-operating-role-inventory')
        observations = [_workload_samples(c['idle_resources'], roles) for c in cycles]
        workload = [o['valid'] for o in observations]
        result['workload_capacity_samples'] = [dict(index=i, valid=len(o['valid']),
            skipped=sum(o['skipped'].values()), skipped_reasons=o['skipped'])
            for i,o in enumerate(observations,1)]
        first_workload = workload[0]
        server_thread_ceiling = _server_thread_ceiling(report, owners, source_inputs)
        # RSS capacity applies to every original numeric observation, including
        # cold/cycle baselines and earlier idle samples. These observations do
        # not raise the frozen workload count capacity or change trend windows.
        rss_samples = [baseline, *report.get('baseline_samples', [])]
        for cycle in cycles:
            idle = cycle['idle_resources']
            rss_samples.extend([idle['baseline'], *idle.get('baseline_samples', []),
                                *idle['samples']])
        # Do not read process_summary: its peak can include later cycles.
        for role in sorted(owners):
            envelope = [{field: max(s['groups'][role][field] for s in w)
                         for field in MEASURES} for w in windows]
            for field in COUNTS:
                values = [e[field] for e in envelope]
                capacity = max(s['groups'][role][field] for s in first_workload)
                capacity_source = 'original-cycle-one-workload'
                if role == 'server' and field == 'sampled_threads':
                    capacity = server_thread_ceiling
                    capacity_source = 'original-baseline-total-plus-source-bound-blocking-capacity'
                result['comparisons'].append(dict(role=role, metric=field, values=values,
                    capacity=capacity, capacity_source=capacity_source,
                    later_idle_steps=[b-a for a,b in zip(values, values[1:])]))
                if any(v > capacity for v in values):
                    errors.append(role + ':' + field + ':operating-capacity-exceeded')
            rss = _rss_intervals(windows, role)
            budget = POLICY['role_rss_budget_bytes'][role]
            # A partial process observation cannot establish count capacity.
            # Its known RSS is still a lower bound: never discard a breach.
            known_rss = [_known_rss(s, role) for s in rss_samples]
            maximum = max(n for n in known_rss if type(n) is int and n > 0)
            result['comparisons'].append(dict(role=role, metric='sampled_rss_bytes',
                declared_operating_budget_bytes=budget, observed_maximum_bytes=maximum, diagnostic_only=True, **rss))
            if maximum > budget:
                errors.append(role + ':sampled_rss_bytes:operating-budget-exceeded')
            if not rss['passed']:
                result['diagnostics'].append(role + ':sampled_rss_bytes:later-rise-not-demonstrably-contracting')
        # Aggregated RSS can double count shared pages. This is a conservative
        # observed process-tree budget, not a claim about unique physical RAM.
        aggregate_samples = rss_samples
        aggregate = max(sum(n for role in owners
                            if (n := _known_rss(s, role)) is not None)
                        for s in aggregate_samples)
        result['comparisons'].append(dict(metric='aggregate_sampled_rss_bytes',
            observed_maximum_bytes=aggregate,
            declared_operating_budget_bytes=POLICY['aggregate_rss_budget_bytes']))
        if aggregate > POLICY['aggregate_rss_budget_bytes']:
            errors.append('aggregate:sampled_rss_bytes:operating-budget-exceeded')
        result['passed'] = not errors
    except (KeyError, TypeError, ValueError, IndexError, OverflowError) as error:
        errors.append(str(error) if isinstance(error, ValueError) else 'incomplete-or-invalid-plateau-evidence:' + type(error).__name__)
    return result


def plateau_verdict(report, source_inputs=None):
    return evaluate(report, source_inputs)['passed']
