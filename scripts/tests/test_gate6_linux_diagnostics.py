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


class InstalledFailureDiagnostics(unittest.TestCase):
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
