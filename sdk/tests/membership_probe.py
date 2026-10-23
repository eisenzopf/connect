"""Actual membership HTTP receipt/query parity; credentials arrive through stdin."""
import json,sys
from connect_sdk import HttpClient
v=json.load(sys.stdin); client=HttpClient(v['base'],v['token'])
assert client.change_membership(v['request'])==v['expected']
assert isinstance(client.membership_invitations(limit=1)['invitations'],list)
if v.get('snapshot'): assert client.membership_snapshot(v['request']['conversation_id'])==v['snapshot']
print(json.dumps({'passed':True}))
