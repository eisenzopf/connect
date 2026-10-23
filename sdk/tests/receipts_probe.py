"""Native result decoding and current recipient receipt policy via public HTTP SDK."""
import json,sys
from connect_sdk import HttpClient,ConnectError
v=json.loads(sys.argv[1]);owner=HttpClient(v['base'],v['token']);peer=HttpClient(v['base'],v['peer_token']);foreign=HttpClient(v['base'],v['foreign_token'])
assert peer.receipt_preferences(v['cid'])['read'] is True
assert peer.set_receipt_preferences('py-pref',v['cid'],True,True)['read'] is True
assert peer.message_receipt('py-read',v['cid'],v['message'],'read')['recorded']=='read'
p=owner.message_receipts(v['cid'],v['message']);assert len(p['receipts'])==1 and p['receipts'][0]['state']=='read'
assert peer.message_receipts(v['cid'],v['message'])['receipts'][0]['state']=='read'
assert any(e.get('receipt',{}).get('state')=='read' for e in owner.inbox()['events'])
for call in [lambda:foreign.message_receipts(v['cid'],v['message']),lambda:owner.message_receipt('py-forge',v['cid'],v['message'],'read')]:
    try:call();raise AssertionError('unauthorized receipt succeeded')
    except ConnectError as e:assert e.code=='forbidden'
print(json.dumps(dict(passed=True)))
