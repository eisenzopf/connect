"""One immutable business intent through the actual loopback operation API."""
import json,sys
from connect_sdk import HttpClient,ConnectError
v=json.loads(sys.argv[1]);client=HttpClient(v['base'],v['token'])
assert client.admit_transition(v['request'])['record_id']==v['record_id']
get=dict(action='get',operation_id=v['request']['operation_id'])
assert client.operation_action(get)['state']=='accepted'
for call,code in [(lambda:HttpClient(v['base'],v['foreign_token']).operation_action(get),'forbidden'),(lambda:client.admit_transition(dict(v['request'],expires_at=v['request']['expires_at']+1)),'conflict')]:
    try:call();raise AssertionError('operation unexpectedly succeeded')
    except ConnectError as e:assert e.code==code
print(json.dumps(dict(passed=True)))
