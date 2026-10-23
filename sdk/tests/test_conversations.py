"""Exact conversation receipt and typed current model at the public HTTP boundary."""
import copy,json,threading,unittest
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from connect_sdk import HttpClient,ConnectError
class Conversations(unittest.TestCase):
 def test_create_requires_exact_receipt_and_keeps_input_without_retry(self):
  valid=dict(id='room',tenant_id='tenant',policy='persistent',idle_close_secs=None,metadata={'private':'value'},participants=[dict(participant_id='part',identity_id='identity',kind='human',role='participant',display_name=None)],opened_at=1,revision=1,closed=False,closed_at=None,reason_code=None,reason=None,receipt=dict(operation_id='create',conversation_id='room',state='completed',completion_boundary='local-mutation',status_reference=dict(action='get',operation_id='create')))
  values=[copy.deepcopy(valid) for _ in range(5)];del values[1]['receipt'];values[2]['receipt']['operation_id']='other';values[2]['receipt']['status_reference']['operation_id']='other';values[3]['receipt']['completion_boundary']='media-ready';values[4]['participants']=['identity'];requests=[]
  class Handler(BaseHTTPRequestHandler):
   def log_message(self,*args):pass
   def do_POST(self):
    requests.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))));raw=json.dumps(dict(profile='connect-control/1',result={'Ok':dict(kind='conversation',data=values[len(requests)-1])})).encode();self.send_response(200);self.send_header('Content-Length',str(len(raw)));self.end_headers();self.wfile.write(raw)
  server=ThreadingHTTPServer(('127.0.0.1',0),Handler);thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
  try:
   client=HttpClient(f'http://127.0.0.1:{server.server_port}','fixture');input=dict(tenant_id=None,policy='persistent',idle_close_secs=9,metadata={'private':'value'},initial_participants=[dict(identity_id='peer',role='guest')]);self.assertEqual(client.create_conversation('create',input),valid)
   for _ in range(4):
    with self.assertRaises(ConnectError) as e:client.create_conversation('create',input)
    self.assertEqual(e.exception.code,'protocol')
   self.assertEqual(len(requests),5);self.assertTrue(all(r['request']==input for r in requests))
  finally:server.shutdown();server.server_close();thread.join()

 def test_untrusted_large_integer_and_null_receipt_are_protocol_rejections(self):
  from connect_sdk.protocol import valid_data
  base=dict(id='room',tenant_id='tenant',policy='persistent',idle_close_secs=None,metadata=None,participants=[],opened_at=1,revision=1,closed=False,closed_at=None,reason_code=None,reason=None)
  self.assertTrue(valid_data('conversation',base))
  self.assertFalse(valid_data('conversation',dict(base,metadata=10**1000)))
  self.assertFalse(valid_data('conversation',dict(base,receipt=dict(operation_id=None,conversation_id='room',state='completed',completion_boundary='local-mutation',status_reference=dict(action='get',operation_id=None)))))
