"""Actual bounded public event helpers; backend observation handles are absent."""
import json,sys
from connect_sdk import HttpClient
v=json.load(sys.stdin);client=HttpClient(v['base'],v['token'])
snapshot=client.conversation_snapshot(v['cid'],dict(limit=1))
assert snapshot['has_more'];n=len(snapshot['items']);cursor=snapshot['next']
while cursor:
    page=client.conversation_snapshot(v['cid'],dict(page=cursor,limit=1))
    assert page['cursor']==snapshot['cursor'];n+=len(page['items']);cursor=page['next']
assert n>=5
page=client.conversation_events(v['cid'],dict(after=v['cursor'],limit=1))
assert page['events'][0]['data']['data']['id']==v['message_id']
assert page['events'][0]['revision']==1 and page['events'][0]['operation_id']=='message'
own=client.session_observations(v['cid'],v['sid'])
assert own['observations'][0]['observation_id']==v['observation_id']
assert own['observations'][0]['source']['correlation_id']==own['routes'][0]['correlation_id']
assert own['routes'][0]['capabilities']['speech'] is False
peer=HttpClient(v['base'],v['foreign_token']).session_observations(v['cid'],v['sid'])
assert not peer['observations'] and not peer['routes']
assert not hasattr(client,'publish_session_observation')
print(json.dumps(dict(passed=True)))
