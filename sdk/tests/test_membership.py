"""Untrusted membership receipts/current projections at the public HTTP boundary."""
import copy,json,threading,unittest
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from connect_sdk import HttpClient,ConnectError

def member():return dict(participant_id='part',conversation_id='room',identity_id='peer',kind='human',role='guest',membership_epoch=1,active=True,removal_reason=None)
def result():return dict(kind='participant',participant=member(),revision=2,receipt=dict(operation_id='add~one',conversation_id='room',state='completed',completion_boundary='local-mutation',status_reference=dict(action='get',operation_id='add~one')))
class Membership(unittest.TestCase):
 def fixture(self,values,run):
  requests=[]
  class Handler(BaseHTTPRequestHandler):
   def log_message(self,*args):pass
   def do_POST(self):
    requests.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))));raw=json.dumps(dict(profile='connect-control/1',result={'Ok':values[len(requests)-1]})).encode();self.send_response(200);self.send_header('Content-Length',str(len(raw)));self.end_headers();self.wfile.write(raw)
  server=ThreadingHTTPServer(('127.0.0.1',0),Handler);thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
  try:run(HttpClient(f'http://127.0.0.1:{server.server_port}','fixture'),requests)
  finally:server.shutdown();server.server_close();thread.join()
 def rejection(self,fn):
  with self.assertRaises(ConnectError) as error:fn()
  self.assertEqual(error.exception.code,'protocol')
 def test_exact_receipt_closed_current_dto_and_no_mutation_retry(self):
  values=[result() for _ in range(12)];del values[1]['receipt'];values[2]['receipt']['operation_id']='other';values[2]['receipt']['status_reference']['operation_id']='other';values[3]['receipt']['conversation_id']='other';values[4]['participant']['conversation_id']='other';values[5]['receipt']['completion_boundary']='media-ready';values[6]['participant']['membership_epoch']=0;values[7]['participant']['role']='é'*33;values[8]['participant']['removal_reason']='bad\nreason';values[9]['participant']['identity_id']='bad:id';values[10]['actor']='injected';values[11]['receipt']['status_reference']['extra']=True
  def run(client,requests):
   request=dict(operation_id='add~one',conversation_id='room',expected_revision=1,change=dict(action='add',identity_id='peer',kind='human',role='guest'));self.assertEqual(client.change_membership(request),values[0])
   for _ in values[1:]:self.rejection(lambda:client.change_membership(request))
   self.assertEqual(len(requests),len(values));self.assertTrue(all(r['request']==request for r in requests))
  self.fixture([dict(kind='membership',data=v) for v in values],run)
 def test_query_scope_cursor_and_page_bounds(self):
  snapshot=dict(conversation_id='room',revision=2,participants=[member()]);other=copy.deepcopy(snapshot);other['participants'][0]['conversation_id']='other';page=dict(invitations=[dict(invitation_id='invite',conversation_id='room',identity_id='peer',expires_at=100,state='pending',conversation_revision=2)],next='opaque')
  values=[dict(kind='membership_snapshot',data=v) for v in [snapshot,dict(snapshot,conversation_id='other'),other]]+[dict(kind='membership_invitations',data=v) for v in [page,dict(page,next='x'*2049),dict(page,invitations=page['invitations']*101)]]
  def run(client,requests):
   self.assertEqual(client.membership_snapshot('room'),snapshot)
   for _ in range(2):self.rejection(lambda:client.membership_snapshot('room'))
   self.assertEqual(client.membership_invitations('opaque',1),page)
   for _ in range(2):self.rejection(lambda:client.membership_invitations('opaque',1))
   self.assertEqual(len(requests),6);self.assertEqual(requests[3]['after'],'opaque')
  self.fixture(values,run)
