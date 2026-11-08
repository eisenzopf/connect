"""Resource qualification needs three original, joined cycles and bounded trend."""
import copy
import hashlib
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from lib.matrix_plateau import CYCLE_POLICY,POLICY,SERVER_RUNTIME_CONFIGURATION,SERVER_SOURCE_INPUTS,runtime_capacity_configuration,cycle_idle_verdict,cycle_restoration,evaluate as _evaluate,plateau_verdict as _plateau_verdict

MIB=1024*1024


def evaluate(value):
    return _evaluate(value, source_inputs=SERVER_SOURCE_INPUTS)


def plateau_verdict(value):
    return _plateau_verdict(value, source_inputs=SERVER_SOURCE_INPUTS)


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
    value = dict(format='connect/matrix-plateau-observation/1', policy=copy.deepcopy(POLICY), baseline=baseline, persistent_owners=owners, cycles=cycles,
        server_runtime_capacity=runtime_capacity_configuration(copy.deepcopy(owners['server']), SERVER_SOURCE_INPUTS))
    for cycle in cycles:
        for s in cycle['idle_resources']['samples']:
            if s['stage'] == 'workload':
                for g in s['groups'].values():
                    g.update(live_processes=2, sampled_threads=300, sampled_open_descriptors=900)
    return value


def group(cycle, role='server'):
    return [s['groups'][role] for s in cycle['idle_resources']['samples'] if s['stage'] in {'idle', 'idle-final'}]


class MatrixPlateauTests(unittest.TestCase):

    def test_actual_cold_warmup_then_bounded_jitter_uses_fixed_cycle_one_reference(self):
        value = fixture()
        result = evaluate(value)
        self.assertTrue(result['passed'], result['failures'])
        count = next(c for c in result['comparisons'] if c.get('role') == 'helper' and c['metric'] == 'sampled_threads')
        self.assertEqual(count['capacity'], 300)
        self.assertEqual(count['capacity_source'], 'original-cycle-one-workload')
        rss = next(c for c in result['comparisons'] if c.get('role') == 'server' and c['metric'] == 'sampled_rss_bytes')
        self.assertTrue(rss['later_rise_unseparated'])
        self.assertEqual(rss['declared_operating_budget_bytes'], 1024*MIB)
        self.assertTrue(plateau_verdict(fixture((160*MIB,160*MIB+512,160*MIB+1024))), 'Owner-approved capacity gate retains linear RSS growth as a separate soak diagnostic')

    def test_bounded_decaying_warmup_can_plateau_but_linear_or_accelerating_growth_fails(self):
        self.assertTrue(plateau_verdict(fixture((160*MIB,190*MIB,200*MIB))))
        for values in [(160*MIB,161*MIB,162*MIB),(160*MIB,162*MIB,165*MIB),(160*MIB,164*MIB,168*MIB)]:
            result = evaluate(fixture(values))
            self.assertTrue(result['passed'], (values,result['failures']))
            self.assertTrue(any('later-rise-not-demonstrably-contracting' in f for f in result['diagnostics']))
            self.assertEqual(result['separate_soak'],dict(task='G6-098',status='pending',qualified=False))
        self.assertTrue(plateau_verdict(fixture((160*MIB,169*MIB,168*MIB))))

    def test_variance_bound_is_disclosed_for_larger_process_footprints(self):
        value = fixture((400*MIB,600*MIB,650*MIB))
        result = evaluate(value)
        self.assertTrue(result['passed'], result['failures'])
        comparison = next(c for c in result['comparisons'] if c.get('role') == 'server' and c['metric'] == 'sampled_rss_bytes')
        self.assertEqual(comparison['declared_operating_budget_bytes'], 1024*MIB)
        self.assertNotIn('variance_allowance_bytes',comparison)
        contract=json.loads((Path(__file__).resolve().parents[2]/'contracts/gate6-transport-matrix.json').read_text())['resource_profile']
        self.assertEqual(contract['profile'],POLICY['profile'])
        self.assertEqual({k:contract['rss_operating_budget_bytes'][k] for k in ('server','browser','helper')},POLICY['role_rss_budget_bytes'])
        self.assertEqual(contract['rss_operating_budget_bytes']['aggregate'],POLICY['aggregate_rss_budget_bytes'])
        self.assertEqual(contract['separate_soak']['task'],'G6-098')
        self.assertEqual(contract['separate_soak']['status'],'pending')
        self.assertFalse(plateau_verdict(fixture((1025*MIB,1025*MIB,1025*MIB))))

    def test_later_thread_fd_or_process_growth_is_never_an_rss_allowance(self):
        for role in ('server','helper'):
            for field in ('live_processes','sampled_threads','sampled_open_descriptors'):
                value=fixture()
                ceiling = 516 if role=='server' and field=='sampled_threads' else dict(live_processes=2,sampled_threads=300,sampled_open_descriptors=900)[field]
                group(value['cycles'][2],role)[-1][field]=ceiling+1
                self.assertFalse(plateau_verdict(value),(role,field))

    def test_bounded_original_thread_pool_growth_must_converge_by_cycle_three(self):
        for counts in [(23,25,25),(23,26,28),(277,290,285)]:
            value=fixture()
            for cycle,count in zip(value['cycles'],counts):
                for observed in group(cycle):
                    observed['sampled_threads']=count
            result=evaluate(value)
            self.assertTrue(result['passed'],result['failures'])
            comparison=next(c for c in result['comparisons'] if c.get('role')=='server' and c['metric']=='sampled_threads')
            self.assertEqual(comparison['values'],list(counts))
            self.assertEqual(comparison['capacity'],516)
            self.assertEqual(comparison['later_idle_steps'],[b-a for a,b in zip(counts,counts[1:])])

    def test_thread_allowance_rejects_continuing_or_unbounded_pool_growth(self):
        value=fixture()
        group(value['cycles'][2])[-1]['sampled_threads']=517
        self.assertFalse(plateau_verdict(value))
        for profile in ('connect/matrix-resource-plateau/1','connect/matrix-resource-plateau/2','connect/matrix-resource-capacity/3'):
            value=fixture()
            value['policy']['profile']=profile
            self.assertFalse(plateau_verdict(value),'Historical policies keep their original verdict')

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
        value['server_runtime_capacity']=runtime_capacity_configuration(value['persistent_owners']['server'],SERVER_SOURCE_INPUTS)
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
        value['policy']['role_rss_budget_bytes']['server'] *= 2
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


    def test_capacity_uses_only_original_first_workload_not_later_peaks_or_summary(self):
        value=fixture()
        for cycle in value['cycles']:
            cycle['idle_resources']['process_summary']={'sampled_threads':99999}
        for s in value['cycles'][1]['idle_resources']['samples']:
            if s['stage']=='workload':
                s['groups']['helper']['sampled_threads']=999
        group(value['cycles'][2],'helper')[-1]['sampled_threads']=301
        self.assertFalse(plateau_verdict(value))
        group(value['cycles'][2],'helper')[-1]['sampled_threads']=300
        self.assertTrue(plateau_verdict(value))

    def test_missing_unavailable_or_out_of_window_first_workload_fails(self):
        for mutation in (
            lambda s:s.update(stage='unknown'),
            lambda s:s.update(monotonic_seconds=40),
            lambda s:s['groups']['helper'].update(unavailable_processes=1),
            lambda s:s['groups']['helper'].update(sampled_threads=None)):
            value=fixture()
            original=next(s for s in value['cycles'][0]['idle_resources']['samples'] if s['stage']=='workload')
            mutation(original)
            self.assertFalse(plateau_verdict(value))
        value=fixture()
        idle=value['cycles'][0]['idle_resources']
        partial=copy.deepcopy(next(s for s in idle['samples'] if s['stage']=='workload'))
        partial['monotonic_seconds']-=1
        partial['groups']['helper']['sampled_open_descriptors']=None
        idle['samples'].insert(1,partial)
        result=evaluate(value)
        self.assertTrue(result['passed'],result['failures'])
        self.assertEqual(result['workload_capacity_samples'][0]['valid'],1)
        self.assertEqual(result['workload_capacity_samples'][0]['skipped'],1)
        partial['groups']['server']['sampled_rss_bytes']=1024*MIB+1
        self.assertFalse(plateau_verdict(value),'Known RSS breach in partial evidence stays failed')
        for malformed in (None, [], {'helper':None}):
            partial['groups']=malformed
            result=evaluate(value)
            self.assertTrue(result['passed'],(malformed,result['failures']))
            self.assertEqual(result['workload_capacity_samples'][0]['skipped'],1)
        late=copy.deepcopy(next(s for s in idle['samples'] if s['stage']=='workload' and s['monotonic_seconds']==39))
        late['monotonic_seconds']=40.5
        late['groups']['helper']['sampled_threads']=99999
        idle['samples'].insert(3,late)
        result=evaluate(value)
        self.assertTrue(result['passed'],result['failures'])
        self.assertEqual(result['workload_capacity_samples'][0]['skipped'],2)
        self.assertEqual(result['workload_capacity_samples'][0]['skipped_reasons']['outside-original-workload-time'],1)
        comparison=next(c for c in result['comparisons'] if c.get('role')=='helper' and c['metric']=='sampled_threads')
        self.assertEqual(comparison['capacity'],300,'Late terminal delivery cannot enlarge original capacity')

    def test_exact_runtime_source_configuration_and_original_server_identity_required(self):
        for mutation in (
            lambda r:r.pop('server_runtime_capacity'),
            lambda r:r['server_runtime_capacity'].update(max_blocking_threads=513),
            lambda r:r['server_runtime_capacity'].update(connect_main_sha256=digest('other-source')),
            lambda r:r['server_runtime_capacity']['original_owner'].update(pid=12)):
            value=fixture()
            mutation(value)
            self.assertFalse(plateau_verdict(value))
        value=fixture()
        independent=dict(SERVER_SOURCE_INPUTS)
        independent['Cargo.lock']=digest('other-lock')
        self.assertFalse(_evaluate(value,source_inputs=independent)['passed'])
        self.assertFalse(_evaluate(value,source_inputs={})['passed'])

    def test_full_observed_intervals_require_demonstrated_contraction_not_max_only(self):
        value=fixture((100*MIB,120*MIB,130*MIB))
        # Point increments20→10 shrink. But a wide second interval does not
        # prove contraction: first increment10..20, second10..20 overlap.
        group(value['cycles'][1])[0]['sampled_rss_bytes']=110*MIB
        result=evaluate(value)
        self.assertTrue(result['passed'],result['failures'])
        self.assertTrue(result['diagnostics'])
        comparison=next(c for c in result['comparisons'] if c.get('role')=='server' and c['metric']=='sampled_rss_bytes')
        self.assertTrue(comparison['diagnostic_only'])
        self.assertEqual(comparison['increments'],[dict(low=10*MIB,high=20*MIB),dict(low=10*MIB,high=20*MIB)])
        self.assertFalse(comparison['demonstrated_contraction'])
        # An overlap means no separable later rise at actual observed resolution.
        group(value['cycles'][2])[0]['sampled_rss_bytes']=120*MIB
        self.assertTrue(plateau_verdict(value))

    def test_declared_rss_budgets_apply_to_workload_and_idle_without_raising_reference(self):
        for cycle_index in (0,1,2):
            value=fixture()
            original=next(s for s in value['cycles'][cycle_index]['idle_resources']['samples'] if s['stage']=='workload')
            original['groups']['server']['sampled_rss_bytes']=1024*MIB+1
            self.assertFalse(plateau_verdict(value))
        value=fixture()
        value['persistent_owners']['browser']=value['persistent_owners'].pop('helper')
        value['baseline']['groups']['browser']=value['baseline']['groups'].pop('helper')
        for cycle in value['cycles']:
            cycle['persistent_owners']['browser']=cycle['persistent_owners'].pop('helper')
            idle=cycle['idle_resources']
            idle['baseline']['groups']['browser']=idle['baseline']['groups'].pop('helper')
            for s in idle['samples']:
                s['groups']['browser']=s['groups'].pop('helper')
                s['groups']['browser']['sampled_rss_bytes']=4*1024*MIB
        self.assertTrue(plateau_verdict(value))
        group(value['cycles'][2],'browser')[-1]['sampled_rss_bytes']+=1
        self.assertFalse(plateau_verdict(value))

    def test_count_capacity_does_not_clear_owned_work_or_changed_owner(self):
        value=fixture()
        value['cycles'][2]['idle_resources']['samples'][-1]['usage']['owned_tasks']=1
        self.assertFalse(plateau_verdict(value))
        value=fixture()
        value['cycles'][2]['persistent_owners']['helper']['start_identity'][0]+=1
        self.assertFalse(plateau_verdict(value))



    def test_known_rss_budgets_cover_cold_cycle_and_original_baseline_samples(self):
        for boundary in ('cold', 'cycle', 'baseline-sample'):
            value=fixture()
            idle=value['cycles'][1]['idle_resources']
            if boundary=='cold':
                observed=value['baseline']
            elif boundary=='cycle':
                observed=idle['baseline']
            else:
                idle['baseline_samples']=[copy.deepcopy(idle['baseline'])]
                observed=idle['baseline_samples'][0]
            observed['groups']['server']['sampled_rss_bytes']=1024*MIB+1
            result=evaluate(value)
            self.assertFalse(result['passed'],boundary)
            self.assertIn('server:sampled_rss_bytes:operating-budget-exceeded',result['failures'])
            comparison=next(c for c in result['comparisons'] if c.get('role')=='server' and c['metric']=='sampled_rss_bytes')
            self.assertEqual(comparison['observed_maximum_bytes'],1024*MIB+1)

    def test_known_partial_early_idle_rss_breach_keeps_role_and_aggregate_budgets(self):
        value=fixture()
        idle=value['cycles'][1]['idle_resources']
        early=sample(idle['terminal_monotonic_seconds']+1,'idle')
        early['groups']['server']['sampled_rss_bytes']=7*1024*MIB
        early['groups']['helper']=None
        idle['samples'].insert(2,early)
        result=evaluate(value)
        self.assertFalse(result['passed'])
        self.assertIn('server:sampled_rss_bytes:operating-budget-exceeded',result['failures'])
        self.assertIn('aggregate:sampled_rss_bytes:operating-budget-exceeded',result['failures'])
        comparison=next(c for c in result['comparisons'] if c['metric']=='aggregate_sampled_rss_bytes')
        self.assertEqual(comparison['observed_maximum_bytes'],7*1024*MIB)
        # Unknown optional baseline samples do not become numeric zero or crash.
        idle['baseline_samples']=[None, [], {'groups':None}]
        self.assertIn('server:sampled_rss_bytes:operating-budget-exceeded',evaluate(value)['failures'])
        early['groups']=None
        self.assertTrue(plateau_verdict(value), 'No known breach remains; mandatory final samples still establish cleanup')

if __name__ == '__main__':
    unittest.main()
