"""Privacy boundary and producer/consumer agreement for installed SDK failures."""
import json
from pathlib import Path
import re
import runpy
import unittest

ROOT = Path(__file__).resolve().parents[2]
check = runpy.run_path(str(ROOT/'scripts/check-gate6-linux-hosting.py'))['node_failure_diagnostic']


def failure(checkpoint='events-bootstrap'):
    return dict(phase='failed',checkpoint=checkpoint,kind='deadline',certainty=None,
                command=None,observedState=None,observedKind=None,observedCertainty=None,
                observedHttpStatus=None,line=None,column=None)


class InstalledFailureDiagnostics(unittest.TestCase):
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
