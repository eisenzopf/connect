"""Actual public HTTP SDK typed send/replay and response decoding."""
import json,sys
from connect_sdk import HttpClient
value=json.loads(sys.argv[1])
client=HttpClient(value['base'],value['token'])
receipt=client.send_message(value['request_id'],value['cid'],value['message'])
assert receipt['message_id']==value['message']['msg_id']
assert receipt['receipt']['completion_boundary']=='local-mutation'
record=client.operation_action(receipt['receipt']['status_reference'])
assert record['operation_id']==value['request_id'] and record['state']=='completed'
assert record['result']['data']==receipt
history=client.history(value['cid'])
assert len(history['messages'])==1
assert history['messages'][0]['body']==value['message']['body']
assert history['messages'][0]['body_encoding']=='base64'
assert history['messages'][0]['attachments']==[]
print(json.dumps(dict(passed=True)))
