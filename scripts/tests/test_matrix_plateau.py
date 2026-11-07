"""Resource qualification needs three original, joined cycles and bounded trend."""
import copy
import hashlib
import sys
import unittest
from pathlib import Path

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from lib.matrix_plateau import CYCLE_POLICY,POLICY,cycle_idle_verdict,cycle_restoration,evaluate,plateau_verdict

MIB=1024*1024


def digest(value):
    return hashlib.sha256(str(value).encode()).hexdigest()


def sample(at, stage, rss=100 * MIB):
    group = dict(root_state='available', unavailable_processes=0, open_resource_unavailable_processes=0, live_processes=1, sampled_threads=4, sampled_open_descriptors=7, sampled_rss_bytes=rss)
    return dict(monotonic_seconds=at, stage=stage, usage={'physical_owners': 0, 'owned_tasks': 0, 'pool:quic-ingress': 0, 'pool:http-requests': 1}, groups={'server': dict(group), 'helper': dict(group)})


def fixture(values=(160 * MIB, 161 * MIB, 160 * MIB)):
    baseline = sample(1, 'baseline')
    owners = dict(server=dict(pid=11, start_identity=101), helper=dict(pid=22, start_identity=[202, 303]))
    cycles = []
    for i, rss in enumerate(values, 1):
        t = 40 * i
        idle = dict(format='connect/native-matrix-idle-observation/2', policy=copy.deepcopy(CYCLE_POLICY), baseline=sample(t - 10, 'baseline'), terminal_monotonic_seconds=t, samples=[sample(t - 10, 'baseline'), sample(t - 1, 'workload', rss), sample(t + 8, 'idle', rss), sample(t + 9, 'idle', rss), sample(t + 10.01, 'idle-final', rss)])
        cycles.append(dict(index=i, workload_id='original-cycle-' + str(i), workload_started=True, workload_complete=True, workload_evidence_sha256=digest('work-' + str(i)), persistent_owners=copy.deepcopy(owners), joins=dict(session_owners_joined=True, helper_work_joined=True, forced=False, evidence_sha256=digest('joins-' + str(i))), idle_resources=idle))
    return dict(format='connect/matrix-plateau-observation/1', policy=copy.deepcopy(POLICY), baseline=baseline, persistent_owners=owners, cycles=cycles)


def group(cycle, role='server'):
    return [s['groups'][role] for s in cycle['idle_resources']['samples'] if s['stage'] in {'idle', 'idle-final'}]


class MatrixPlateauTests(unittest.TestCase):

    def test_actual_cold_warmup_then_bounded_jitter_uses_fixed_cycle_one_reference(self):
        value = fixture()
        self.assertTrue(plateau_verdict(value))
        result = evaluate(value)
        rss = next((c for c in result['comparisons'] if c['role'] == 'server' and c['metric'] == 'sampled_rss_bytes'))
        self.assertEqual(rss['cycle_one_reference_bytes'], 160 * MIB)
        self.assertEqual(rss['variance_allowance_bytes'], 8 * MIB)
        self.assertEqual(rss['idle_to_idle_steps_bytes'], [MIB, -MIB])
        self.assertFalse(rss['persistent_positive_trend'])
        self.assertTrue(plateau_verdict(fixture((160 * MIB, 160 * MIB + 512, 160 * MIB + 1024))))

    def test_bounded_decaying_warmup_can_plateau_but_linear_or_accelerating_growth_fails(self):
        self.assertTrue(plateau_verdict(fixture((160 * MIB, 163 * MIB, 164 * MIB))))
        for values in [(160 * MIB, 161 * MIB, 162 * MIB), (160 * MIB, 162 * MIB, 165 * MIB), (160 * MIB, 164 * MIB, 167 * MIB)]:
            result = evaluate(fixture(values))
            self.assertFalse(result['passed'])
            self.assertTrue(any(('persistent-positive-trend' in f for f in result['failures'])))
        self.assertFalse(plateau_verdict(fixture((160 * MIB, 169 * MIB, 168 * MIB))))

    def test_variance_bound_is_disclosed_for_larger_process_footprints(self):
        value = fixture((400 * MIB, 415 * MIB, 414 * MIB))
        self.assertTrue(plateau_verdict(value))
        comparison = next((c for c in evaluate(value)['comparisons'] if c['role'] == 'server' and c['metric'] == 'sampled_rss_bytes'))
        self.assertEqual(comparison['variance_allowance_bytes'], 20 * MIB)
        self.assertFalse(plateau_verdict(fixture((400 * MIB, 421 * MIB, 420 * MIB))))

    def test_later_thread_fd_or_process_growth_is_never_an_rss_allowance(self):
        for role in ('server', 'helper'):
            for field in ('live_processes', 'sampled_threads', 'sampled_open_descriptors'):
                value = fixture()
                for i in (1, 2):
                    for g in group(value['cycles'][i], role):
                        g[field] += i
                self.assertFalse(plateau_verdict(value), (role, field))
        value = fixture()
        for cycle in value['cycles']:
            for g in group(cycle):
                g['sampled_threads'] += 2
        self.assertTrue(plateau_verdict(value))

    def test_each_original_idle_window_requires_zero_session_owners_and_pool_restoration(self):
        for cycle_index in range(3):
            for name in ('physical_owners', 'owned_tasks', 'pool:quic-ingress', 'pool:http-requests'):
                value = fixture()
                value['cycles'][cycle_index]['idle_resources']['samples'][-1]['usage'][name] += 1
                self.assertFalse(plateau_verdict(value), (cycle_index, name))
        before = sample(1, 'baseline')
        after = sample(20, 'idle')
        self.assertEqual(cycle_restoration(before, after), [])
        after['usage']['pool:http-requests'] = 0
        self.assertEqual(cycle_restoration(before, after), [], 'A released process-level baseline health request is not a Session leak')
        before['usage']['owned_tasks'] = 1
        self.assertTrue(cycle_restoration(before, after))

    def test_missing_cycle_unavailable_counters_and_reused_evidence_fail(self):
        for mutation in (lambda r: r['cycles'].pop(), lambda r: r['cycles'][1].pop('idle_resources'), lambda r: r['cycles'][1].update(workload_started=False), lambda r: r['cycles'][1].update(workload_id=r['cycles'][0]['workload_id']), lambda r: r['cycles'][1].update(workload_evidence_sha256=r['cycles'][0]['workload_evidence_sha256'])):
            value = fixture()
            mutation(value)
            self.assertFalse(plateau_verdict(value))
        for change in (None, -1, True):
            value = fixture()
            group(value['cycles'][2])[-1]['sampled_rss_bytes'] = change
            self.assertFalse(plateau_verdict(value))
        value = fixture()
        group(value['cycles'][1])[-1]['root_state'] = 'retired'
        self.assertFalse(plateau_verdict(value))

    def test_exact_original_pid_birth_role_and_all_actual_joins_are_required(self):
        for key in ('pid', 'start_identity'):
            value = fixture()
            value['cycles'][1]['persistent_owners']['server'][key] += 1
            self.assertFalse(plateau_verdict(value))
        value = fixture()
        value['cycles'][1]['persistent_owners'].pop('helper')
        self.assertFalse(plateau_verdict(value))
        for field, new in (('session_owners_joined', False), ('helper_work_joined', False), ('forced', True), ('evidence_sha256', '')):
            value = fixture()
            value['cycles'][1]['joins'][field] = new
            self.assertFalse(plateau_verdict(value))

    def test_each_cycle_requires_its_own_original_join_evidence(self):
        value = fixture()
        value['cycles'][2]['joins']['evidence_sha256'] = value['cycles'][0]['joins']['evidence_sha256']
        result = evaluate(value)
        self.assertFalse(result['passed'])
        self.assertIn('cycle-3:original-join-evidence-reused', result['failures'])

    def test_unknown_birth_placeholders_cannot_stand_in_for_original_owner_identity(self):
        for birth in ('unknown', 'unavailable', '123', ''):
            value = fixture()
            value['persistent_owners']['server']['start_identity'] = birth
            for cycle in value['cycles']:
                cycle['persistent_owners']['server']['start_identity'] = birth
            self.assertFalse(plateau_verdict(value), birth)
        value = fixture()
        birth = digest('original-os-identity')
        value['persistent_owners']['server']['start_identity'] = birth
        for cycle in value['cycles']:
            cycle['persistent_owners']['server']['start_identity'] = birth
        self.assertTrue(plateau_verdict(value), 'An actual producer-observed64hex identity remains representable')

    def test_each_cycle_needs_a_fresh_baseline_after_previous_final_observation(self):
        value = fixture()
        final = value['cycles'][0]['idle_resources']['samples'][-1]['monotonic_seconds']
        idle = value['cycles'][1]['idle_resources']
        idle['baseline']['monotonic_seconds'] = final - 0.001
        self.assertFalse(plateau_verdict(value))
        idle['baseline']['monotonic_seconds'] = final
        self.assertTrue(plateau_verdict(value), 'Boundary equality is sequential, not overlapping')

    def test_three_short_or_missing_original_workload_windows_do_not_qualify(self):
        for value in (False, None, 1):
            report = fixture()
            for cycle in report['cycles']:
                cycle['workload_complete'] = value
            self.assertFalse(plateau_verdict(report), value)
        report = fixture()
        report['cycles'][1].pop('workload_complete')
        self.assertFalse(plateau_verdict(report))
        report = fixture()
        report['functional_passed'] = False
        self.assertTrue(plateau_verdict(report), 'Actual full window can complete despite independent quality failure')

    def test_original_ten_second_budget_ordering_and_policy_are_not_relaxed(self):
        for offset in (9.99, 12.01, float('nan')):
            value = fixture()
            idle = value['cycles'][0]['idle_resources']
            idle['samples'][-1]['monotonic_seconds'] = idle['terminal_monotonic_seconds'] + offset
            self.assertFalse(plateau_verdict(value))
        value = fixture()
        value['cycles'][1]['idle_resources']['terminal_monotonic_seconds'] = 1
        self.assertFalse(plateau_verdict(value))
        value = fixture()
        value['policy']['rss_variance_floor_bytes'] *= 2
        self.assertFalse(plateau_verdict(value))
        value = fixture()
        idle = value['cycles'][0]['idle_resources']
        idle['format'] = 'connect/matrix-idle-observation/1'
        self.assertTrue(cycle_idle_verdict(idle, value['baseline']))
        idle['policy'] = {'profile': 'connect/matrix-idle/1'}
        self.assertFalse(cycle_idle_verdict(idle, value['baseline']))

    def test_producer_green_flags_cannot_clear_missing_observation_or_media_failure(self):
        value = fixture()
        value['passed'] = True
        value['cycles'][0]['idle_resources']['passed'] = True
        value['cycles'][0]['idle_resources']['samples'][-1]['error'] = 'PermissionError'
        self.assertFalse(plateau_verdict(value))
        value = fixture()
        value['functional_passed'] = False
        self.assertTrue(plateau_verdict(value), 'Independent resource verdict does not qualify media or handoff')
        self.assertFalse(value['functional_passed'])


if __name__ == '__main__':
    unittest.main()
