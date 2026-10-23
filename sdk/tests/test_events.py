"""Shared vectors reject forged authority, cross-scope pages and source drift."""
import copy,json,pathlib,unittest
from connect_sdk.protocol import decode_response
from connect_sdk.errors import ConnectError
from connect_sdk.task_operations import exchange_timeout
MATRIX=json.loads((pathlib.Path(__file__).parent/'events_matrix.json').read_text())
def raw(outcome):return json.dumps(dict(profile='connect-control/1',result=dict(Ok=outcome)))
class Events(unittest.TestCase):
    def test_valid_results(self):
        for v in MATRIX['valid']:
            with self.subTest(v['name']):self.assertEqual(decode_response(raw(v['outcome']),v['outcome']['kind'])['kind'],v['outcome']['kind'])
    def test_invalid_results(self):
        for v in MATRIX['invalid']:
            with self.subTest(v['name']),self.assertRaises(ConnectError) as error:
                decode_response(raw(v['outcome']),v['outcome']['kind'])
            self.assertEqual(error.exception.code,'protocol')
    def test_bounded_wait(self):
        self.assertEqual(exchange_timeout(1,dict(op='conversation_events',query=dict(wait_ms=999999))),31)
        self.assertEqual(exchange_timeout(1,dict(op='session_observations',query=dict(wait_ms=True))),1)
    def test_nonfinite_or_invalid_unicode(self):
        for value in [float('inf'),'\ud800']:
            outcome=copy.deepcopy(MATRIX['valid'][5]['outcome']);outcome['data']['observations'][0]['data']['arguments']['value']=value
            with self.assertRaises(ConnectError) as error:decode_response(raw(outcome),outcome['kind'])
            self.assertEqual(error.exception.code,'protocol')
