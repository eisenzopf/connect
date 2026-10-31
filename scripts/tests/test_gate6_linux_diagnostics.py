"""Privacy boundary and producer/consumer agreement for installed SDK failures."""
import json
import subprocess
from pathlib import Path
import re
import runpy
import unittest

ROOT = Path(__file__).resolve().parents[2]
check = runpy.run_path(str(ROOT/'scripts/check-gate6-linux-hosting.py'))['node_failure_diagnostic']
progress = runpy.run_path(str(ROOT/'scripts/check-gate6-linux-hosting.py'))['node_progress_diagnostic']
python_movement = runpy.run_path(str(ROOT/'scripts/check-gate6-linux-hosting.py'))['python_movement_failure_diagnostic']
python_phase = runpy.run_path(str(ROOT/'scripts/check-gate6-linux-hosting.py'))['python_movement_phase_diagnostic']
python_native = runpy.run_path(str(ROOT/'scripts/check-gate6-linux-hosting.py'))['python_native_failure_diagnostic']
python_administrative = runpy.run_path(str(ROOT/'scripts/check-gate6-linux-hosting.py'))['python_administrative_failure_diagnostic']
python_consultation = runpy.run_path(str(ROOT/'scripts/check-gate6-linux-hosting.py'))['python_consultation_failure_diagnostic']


def failure(checkpoint='events-bootstrap'):
    return dict(phase='failed',checkpoint=checkpoint,kind='deadline',certainty=None,
                command=None,observedState=None,observedKind=None,observedCertainty=None,
                observedHttpStatus=None,line=None,column=None)


language_av = runpy.run_path(str(ROOT/'scripts/check-gate6-linux-hosting.py'))['language_av_diagnostic']

class InstalledFailureDiagnostics(unittest.TestCase):
    def test_every_actual_rust_node_journey_phase_is_accepted_with_original_bounds(self):
        validate = runpy.run_path(str(ROOT/'scripts/check-gate6-linux-hosting.py'))['node_phase_diagnostic']
        source = (ROOT/'crates/connect-server/tests/support/canonical_stdio/node_sdk.rs').read_text()
        phases = set(re.findall(r'phase\(\s*&mut\s+lines,\s*"([a-z-]+)"', source))
        self.assertEqual(len(phases), 8)
        self.assertIn('media-complete', phases)
        for phase in phases:
            value = dict(algorithm='EdDSA', phase=phase, passed=True, seconds=1.5, deadline_seconds=15)
            self.assertEqual(validate(value), value)
        for invalid in [dict(value, credential='private'), dict(value, phase='private'),
                        dict(value, seconds=True), dict(value, passed=1), dict(value, deadline_seconds=30)]:
            with self.assertRaises(AssertionError):
                validate(invalid)

    def test_video_feedback_component_keeps_only_boolean_route_and_join_observations(self):
        validate = runpy.run_path(str(ROOT/'scripts/check-gate6-linux-hosting.py'))['video_feedback_diagnostic']
        value = dict(active=[[False, False], [False, False]], withdrawn=[False, False], restored=[False, False],
            original_peers_joined=True, original_server_joined=True, reclaimed_slots=2)
        self.assertEqual(validate(value), value)
        for bad in [dict(value, credential='private'), dict(value, active=[[False, 'private'], [False, False]]),
                    dict(value, restored=None), dict(value, original_peers_joined=1), dict(value, reclaimed_slots=True)]:
            with self.assertRaises(AssertionError):
                validate(bad)

    def test_consultation_return_trace_keeps_declared_boundaries_and_bounded_counts(self):
        parse = runpy.run_path(str(ROOT/'scripts/check-gate6-linux-hosting.py'))['consultation_return_observations']
        trace = ('\x1b[31mDEBUG connect_service::canonical_consultation: Native return planning snapshot stage="return-planned" current=true source_count=3 receiver_count=2 connection=private-secret\x1b[0m\n'
                 'DEBUG connect_service::canonical_consultation: Native return preparation refused stage="return-prepare" code=authority-busy reason="private body"\n'
                 'DEBUG connect_service::canonical_consultation: Native return refused stage="private-secret" code=private-secret\n'
                 'DEBUG connect_service::canonical_consultation: Native return refused stage="return-read" code=private-secret source_count=999\n')
        self.assertEqual(parse(trace), [dict(stage='return-planned', code='other', current=True, source_count=3, receiver_count=2),
            dict(stage='return-prepare', code='authority-busy'), dict(stage='return-read', code='other')])
        self.assertNotIn('private', json.dumps(parse(trace)))
        self.assertEqual(len(parse(trace * 20)), 32)
        self.assertEqual(parse('unrelated private error'), [])

    def test_initial_rtc_admission_trace_retains_only_finite_boundaries(self):
        check = runpy.run_path(str(ROOT/'scripts/check-gate6-linux-hosting.py'))['rtc_admission_observations']
        source = '\x1b[31mDEBUG private/path connection=private-secret RTC actual admission boundary refused error=Authority\x1b[0m\nRTC original media admission refused code=forbidden credential=private-secret\nRTC original resource observation refused physical_closed=false bindings_match=true revision=private-secret\nRTC original Session observation refused code=private-secret connection_present=false participant_present=true'
        result = check(source)
        self.assertEqual(result[0], dict(phase='actual-boundary',code='authority',flags={}))
        self.assertEqual(result[1]['code'],'forbidden')
        self.assertEqual(result[2]['flags'],dict(bindings_match=True,physical_closed=False))
        self.assertEqual(result[3],dict(phase='session-observation',code='other',flags=dict(connection_present=False,participant_present=True)))
        self.assertNotIn('private',json.dumps(result))
        self.assertEqual(len(check(source*20)),16)
        self.assertEqual(check('unrelated private diagnostic'),[])
        self.assertEqual(check('RTC initial authority observation refused code=authority-busy private=secret\nRTC initial receipt persistence refused code=storage-unavailable private=secret'), [
            dict(phase='initial-authority',code='authority-busy',flags={}),
            dict(phase='initial-receipt',code='storage-unavailable',flags={})])

    def test_ice_worker_failure_preserves_boundary_without_authority_or_transport_secrets(self):
        validate=runpy.run_path(str(ROOT/'scripts/check-gate6-linux-hosting.py'))['rtc_ice_worker_failure_diagnostic']
        value=dict(phase='current-authority',code='forbidden',expected_generation=2,submitted=True,receipt_captured=False,stop_cancelled=False,physical_cancelled=False,deadline_elapsed=False)
        self.assertEqual(validate(value),value)
        for invalid in [dict(value,credential='private'),dict(value,sdp='private'),dict(value,phase='private operation'),dict(value,code='private reason'),dict(value,expected_generation=True),dict(value,expected_generation=9007199254740992),dict(value,submitted=1)]:
            with self.assertRaises(AssertionError):validate(invalid)

    def test_rtc_operation_failure_excludes_private_record_and_transport_values(self):
        validate=runpy.run_path(str(ROOT/'scripts/check-gate6-linux-hosting.py'))['rtc_operation_failure_diagnostic']
        value=dict(checkpoint='restart-1',state='unknown',problem_code='reconciliation-required',result_present=False,cancel_requested=False,effects=[dict(state='unknown',attempt=1)])
        self.assertEqual(validate(value),value)
        for invalid in [dict(value,problem={'private':'body'}),dict(value,checkpoint='private id'),dict(value,problem_code='private-secret'),dict(value,result_present=1),dict(value,effects=[dict(state='unknown',attempt=True)]),dict(value,effects=[dict(state='private',attempt=1)]),dict(value,effects=value['effects']*5)]:
            with self.assertRaises(AssertionError):validate(invalid)

    def test_ipc_revocation_failure_excludes_command_receipt_and_payload(self):
        validate=runpy.run_path(str(ROOT/'scripts/check-gate6-linux-hosting.py'))['ipc_data_revocation_failure_diagnostic']
        self.assertEqual(validate(dict(kind='cancelled',certainty='read-only',original_operation_matches=True))['kind'],'cancelled')
        value=dict(kind='closed',certainty='not-submitted',original_operation_matches=True)
        self.assertEqual(validate(value),value)
        for invalid in [dict(value,operation={'credential':'private'}),dict(value,kind='private'),dict(value,certainty='private'),dict(value,original_operation_matches=1)]:
            with self.assertRaises(AssertionError):validate(invalid)

    def test_language_av_requires_independent_expected_sources_and_bounded_counts(self):
        first=dict(identified_audio_hz=660,decoded_audio_samples=4800,audio_rms=2800.0,source_energy_ratio=100.0,identified_video_high_channels=[False,False,True],decoded_video_frames=6,decoded_rgb_bytes=4147200,decoder_children_joined=2)
        second=dict(first,identified_audio_hz=440,identified_video_high_channels=[True,False,False])
        self.assertEqual(language_av([first,second],[660,440],[[False,False,True],[True,False,False]]),[first,second])
        for invalid in [dict(first,decoded_audio_samples=True),dict(first,audio_rms=float('nan')),dict(first,source_energy_ratio=float('inf')),dict(first,source_energy_ratio=1),dict(first,identified_audio_hz=440),dict(first,identified_video_high_channels=[True,False,False]),dict(first,private_payload='secret'),dict(first,decoder_children_joined=1)]:
            with self.assertRaises(AssertionError):language_av([invalid,second],[660,440],[[False,False,True],[True,False,False]])

    def test_python_consultation_failure_schema_keeps_recovery_stage_without_saved_intent(self):
        value=dict(phase='failed',checkpoint='return-wait',kind='deadline',certainty='read-only',line=92,error_type='SdkError')
        self.assertEqual(python_consultation(value),value)
        for invalid in [dict(value,reference={'credential':'private'}),dict(value,message='private'),
                        dict(value,line=True),dict(value,line=100000),dict(value,checkpoint='private operation')]:
            with self.assertRaises(AssertionError):python_consultation(invalid)

    def test_python_administrative_diagnostics_exclude_commands_and_private_state(self):
        value=dict(phase='failed',checkpoint='held-cancel',kind='cancelled',certainty='uncertain',
                   line=112,error_type='SdkError')
        self.assertEqual(python_administrative(value),value)
        for invalid in [dict(value,command={'alias':'private'}),dict(value,message='private credential'),
                        dict(value,line=True),dict(value,line=100000),dict(value,checkpoint='/private/state')]:
            with self.assertRaises(AssertionError):python_administrative(invalid)

    def test_python_native_controls_keep_only_bounded_failure_locations(self):
        value=dict(phase='failed',checkpoint='session-decisions',kind='assertion',certainty=None,
                   source_line=184,error_type='AssertionError')
        self.assertEqual(python_native(value),value)
        for invalid in [dict(value,message='private payload'),dict(value,path='/private/state'),
                        dict(value,source_line=True),dict(value,source_line=100000),
                        dict(value,error_type='private account'),dict(value,checkpoint='private intent')]:
            with self.assertRaises(AssertionError):python_native(invalid)

    def test_python_movement_diagnostics_keep_only_bounded_source_locations(self):
        value=dict(phase='failed',checkpoint='begin',kind='consumer-assertion',certainty=None,line=85,error_type='KeyError')
        self.assertEqual(python_movement(value),value)
        for invalid in [dict(value,message='private payload'),dict(value,credential='private'),dict(value,line=True),
                        dict(value,line=100000),dict(value,error_type='private account'),dict(value,checkpoint='/private/config')]:
            with self.assertRaises(AssertionError):python_movement(invalid)

    def test_python_movement_phases_preserve_the_original_bounds(self):
        value=dict(algorithm='ES256',phase='completed',seconds=2.5,deadline_seconds=15)
        self.assertEqual(python_phase(value),value)
        for invalid in [dict(value,seconds=True),dict(value,seconds=float('nan')),dict(value,seconds=91),
                        dict(value,deadline_seconds=30),dict(value,phase='private payload')]:
            with self.assertRaises(AssertionError):python_phase(invalid)

    def test_relative_progress_accepts_fixed_stages_without_payloads_or_private_paths(self):
        for algorithm in ['EdDSA','ES256']:
            for stage in ['session-finish','events-bootstrap','events-original','events-resume']:
                value=dict(algorithm=algorithm,checkpoint=stage,seconds=3.25,deadline_seconds=15)
                self.assertEqual(progress(value),value)

    def test_relative_progress_rejects_unknown_or_unbounded_private_diagnostics(self):
        base=dict(algorithm='EdDSA',checkpoint='events-original',seconds=1.5,deadline_seconds=15)
        for value in [dict(base,credential='private'),dict(base,path='/private/config'),dict(base,message='private'),
                      dict(base,algorithm='private'),dict(base,checkpoint='private checkpoint'),
                      dict(base,seconds=True),dict(base,seconds=float('nan')),dict(base,seconds=float('inf')),
                      dict(base,seconds=-1),dict(base,seconds=91),dict(base,deadline_seconds=60)]:
            with self.assertRaises(AssertionError):progress(value)

    def test_https_assertion_location_uses_the_actual_installed_basename_without_private_stack(self):
        source=(ROOT/'sdk/tests/node_https_server_consumer.mjs').read_text()
        prepare=(ROOT/'sdk/tests/prepare_native_consumer.py').read_text()
        self.assertIn("'https-consume.mjs'",prepare)
        pattern=re.search(r'\.match\((/[^;\n]+/)\)',source)[1]
        code='const pattern='+pattern+';const result=[];for(const name of ["https-consume.mjs","node_https_server_consumer.mjs"]){const at=("Error: private credential and payload\\n at file:///private/state/"+name+":22:7").match(pattern);result.push({line:at?Number(at[1]):null,column:at?Number(at[2]):null});}process.stdout.write(JSON.stringify(result));'
        result=subprocess.run(['node','--input-type=module','-e',code],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,timeout=3,check=True)
        self.assertEqual(json.loads(result.stdout),[{'line':22,'column':7}]*2)
        self.assertNotIn('private',result.stdout)

    def test_original_rust_timeout_producers_match_the_upload_schema(self):
        for name in ['canonical_stdio/node_sdk.rs','canonical_https_hosting/node.rs']:
            source=(ROOT/'crates/connect-server/tests/support'/name).read_text()
            markers=re.findall(r'json!\((\{"phase":"failed"[^\n]+\})\)',source)
            self.assertEqual(len(markers),1)
            value=json.loads(markers[0].replace('"checkpoint":checkpoint','"checkpoint":"events-bootstrap"'))
            self.assertEqual(check(value),failure())

    def test_fixed_native_and_https_stages_remain_uploadable(self):
        for stage in ['events-bootstrap','events-original','events-resume','events-current-authority',
                      'https-snapshot','https-send','https-journal','https-receipts','https-revocation']:
            self.assertEqual(check(failure(stage)),failure(stage))
        value=failure();value.update(kind='assertion',line=37,column=12,observedKind='http-forbidden',observedHttpStatus=403)
        self.assertEqual(check(value),value)

    def test_unbounded_private_fields_and_unrecognized_shapes_are_refused(self):
        for value in [dict(failure(),credential='private'),dict(failure(),message='private'),
                      dict(failure(),checkpoint='private account'),dict(failure(),kind='private body!'),
                      dict(failure(),line=True),dict(failure(),column=100000),
                      dict(failure(),observedState='private'),dict(failure(),observedHttpStatus=700)]:
            with self.assertRaises(AssertionError):check(value)
        value=failure();del value['observedHttpStatus']
        with self.assertRaises(AssertionError):check(value)
