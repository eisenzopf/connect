"""Actual HTTP conversation receipt/list parity; credentials arrive through stdin."""
import json,sys
from connect_sdk import HttpClient
v=json.load(sys.stdin);client=HttpClient(v['base'],v['token'])
c=client.create_conversation('shared-conversation',v['request']);assert c['id']==v['cid'];assert c['metadata']==v['request']['metadata'];assert c['receipt']['operation_id']=='shared-conversation'
page=client.list_conversations({'filter':{'state':'all','identity_id':v['peer']},'limit':1});assert page['conversations'][0]['id']==c['id']
print(json.dumps({'passed':True}))
