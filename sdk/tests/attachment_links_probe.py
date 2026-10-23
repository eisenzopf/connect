"""Public SDK issuance, stable chunk replay and wrong-recipient denials."""
import json,sys
from connect_sdk import HttpClient,ConnectError
v=json.loads(sys.argv[1]);owner=HttpClient(v['base'],v['token'])
assert owner.artifact_inspect(v['cid'],v['artifact'])['received']==6
link=owner.artifact_upload_link(v['cid'],v['artifact'],45)
assert link['purpose']=='upload'
assert owner.artifact_link_append('chunk',link,0,'c2VjcmV0')['received']==6
peer=HttpClient(v['base'],v['peer_token']);foreign=HttpClient(v['base'],v['foreign_token'])
for call in [lambda:peer.artifact_upload_link(v['cid'],v['artifact']),lambda:peer.artifact_link_append('stolen',link,0,'c2VjcmV0'),lambda:foreign.artifact_inspect(v['cid'],v['artifact'])]:
    try:call();raise AssertionError('unauthorized operation succeeded')
    except ConnectError as e:assert e.code=='forbidden'
print(json.dumps(dict(passed=True)))
