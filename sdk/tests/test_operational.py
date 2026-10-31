import copy,json,unittest
from pathlib import Path
from connect_sdk.operational import valid_operational
from connect_sdk.protocol import decode_response
from connect_sdk.errors import ConnectError
FIXTURE=json.loads((Path(__file__).resolve().parents[2]/'contracts/fixtures/operational-health-v1.json').read_text())
def decode(v):
 return decode_response(json.dumps({'profile':'connect-control/1','result':{'Ok':{'kind':'health','data':{'installation_id':'fixture','schema':1,'profile':'connect-control/1','operational':v}}}}),'health')
class Operational(unittest.TestCase):
 def test_available_busy_retired_and_over_limit_remain_distinct(self):
  self.assertTrue(valid_operational(FIXTURE));self.assertEqual(decode(FIXTURE)['data']['operational'],FIXTURE)
  v=copy.deepcopy(FIXTURE);v['resource_pools'][0]['usage']['used']=5;self.assertTrue(valid_operational(v))
  v['runtime']=None;v['resource_pools']=None;self.assertTrue(valid_operational(v))
 def test_invalid_numbers_private_fields_and_inconsistent_lifecycle_refuse(self):
  edits=[
   lambda v:v['operations'].update(accepted=True),lambda v:v['operations'].update(retained=1),
   lambda v:v['limits'].update(connections=9007199254740992),lambda v:v.update(observed_at=-1),
   lambda v:v.update(credential='private-diagnostic-secret'),lambda v:v['operations'].update(body='private-diagnostic-secret'),
   lambda v:v['resource_pools'][0]['usage'].update(remote_address='private-diagnostic-secret'),
   lambda v:v['resource_pools'].append(v['resource_pools'][0]),lambda v:v.update(resource_pools=[v['resource_pools'][0]]*33),
   lambda v:v['resource_pools'][0].update(usage=None),lambda v:v['resource_pools'][1].update(usage=v['resource_pools'][0]['usage']),
   lambda v:v['resource_pools'][0].update(kind='private-provider-id'),lambda v:v['runtime'].update(owned_tasks=1.5)]
  for edit in edits:
   v=copy.deepcopy(FIXTURE);edit(v)
   with self.subTest(value=v):
    self.assertFalse(valid_operational(v))
    with self.assertRaises(ConnectError) as caught:decode(v)
    self.assertEqual(caught.exception.code,'protocol');self.assertNotIn('private-diagnostic-secret',str(caught.exception))
