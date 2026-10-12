"""Reject partial retirement profiles and missing resource observations."""
import math
import re

TEST = 'retired_setups_reuse_capacity_through_the_full_session_limit'
SESSIONS = 10_000
LISTENERS = 32


def validate(data, rust_log):
    summaries = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', rust_log)
    assert summaries == [('1', '0', '0')], 'Required retirement check did not complete'
    assert f'test {TEST} ...' in rust_log and 'FAILED' not in rust_log
    assert data['status'] == 'passed'
    assert (data['target_setups'], data['listener_limit'], data['setup_lifetime_seconds']) == (SESSIONS, LISTENERS, 5)
    expected = [('baseline', 0, 0)]
    for start in range(0, SESSIONS, LISTENERS):
        count = min(LISTENERS, SESSIONS - start)
        expected += [('ready', start + count, count), ('retired', start + count, 0)]
    expected += [('restarted', SESSIONS, 0)]
    observed = data['observations']
    assert len(observed) == len(expected), 'Missing batch or restart observations'
    previous = -1.0
    for row, (stage, completed, live) in zip(observed, expected):
        assert (row['stage'], row['completed_setups'], row['live_listeners']) == (stage, completed, live)
        assert (row['bindings'], row['distinct_nonces'], row['unknown_bindings']) == (completed, completed, 0)
        assert isinstance(row['process_rss_bytes'], int) and row['process_rss_bytes'] > 0
        assert isinstance(row['database_apparent_bytes'], int) and row['database_apparent_bytes'] > 0
        elapsed = row['elapsed_seconds']
        assert math.isfinite(elapsed) and elapsed >= previous
        previous = elapsed
    original, restarted = observed[-2]['binding_digest'], observed[-1]['binding_digest']
    assert isinstance(original, str) and re.fullmatch(r'[0-9a-f]{64}', original)
    assert original == restarted, 'Restart changed retained setup bindings'
    return {
        'retired_setups': SESSIONS, 'batches': math.ceil(SESSIONS / LISTENERS),
        'point_samples': len(observed),
        'whole_process_sampled_peak_rss_bytes': max(row['process_rss_bytes'] for row in observed),
        'initial_process_rss_bytes': observed[0]['process_rss_bytes'],
        'final_retired_process_rss_bytes': observed[-2]['process_rss_bytes'],
        'restarted_process_rss_bytes': observed[-1]['process_rss_bytes'],
        'database_apparent_peak_bytes': max(row['database_apparent_bytes'] for row in observed),
        'retained_binding_digest': original,
        'memory_scope': data['process_rss_scope'],
    }
