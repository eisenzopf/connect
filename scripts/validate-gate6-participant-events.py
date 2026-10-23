#!/usr/bin/env python3
"""Check actual signed native appearance facts and current-authorized HTTP recovery."""
import datetime
import json
import pathlib
import sys
from jsonschema import Draft202012Validator
ROOT=pathlib.Path(__file__).resolve().parents[1]
wire=Draft202012Validator(json.loads((ROOT/'contracts/schemas/uctp-envelope-2.schema.json').read_text()))
api=json.loads((ROOT/'contracts/openapi-events.json').read_text())
event=Draft202012Validator({'$ref':'#/components/schemas/JournalEvent','components':api['components']})
def timestamp(value):
    parsed=datetime.datetime.fromisoformat(value.replace('Z','+00:00'))
    assert parsed.tzinfo is not None
    return int(parsed.timestamp())
paths=sorted(pathlib.Path(sys.argv[1]).glob('*-participants.json'))
assert len(paths)==2
assert {json.loads(p.read_text())['algorithm'] for p in paths}=={'EdDSA','ES256'}
for path in paths:
    row=json.loads(path.read_text());frames=[row[k] for k in ['joined','left','rejoined','explicit_left']]
    assert len({f['id'] for f in frames})==4
    assert len(row['journal'])==4
    pid=frames[0]['payload']['participant']['participant_id']
    for frame,fact in zip(frames,row['journal']):
        wire.validate(frame);event.validate(fact)
        assert frame['signature']['alg']==row['algorithm']
        assert frame['cid']==fact['conversation_id'] and frame['sid']==fact['session_id']=='native-session'
        assert not {'connid','operation_id','in_reply_to'}&frame.keys()
        assert fact['connection_id'] is None and fact['participant_id']==pid
        assert fact['operation_id']==('presence-explicit-leave' if frame['payload'].get('reason')=='explicit-leave' else None)
        data=fact['data']['data']
        assert fact['kind']==fact['data']['kind']
        assert data['membership_epoch']==1 and data['session']['revision']==fact['revision']
        if frame['type']=='session.participant.joined':
            assert fact['kind']=='session_participant_joined' and data['session']['state']=='active'
            body=frame['payload']['participant'];expected=dict(data['participant'])
            assert timestamp(body['joined_at'])==expected.pop('joined_at')<=fact['observed_at']
            actual=dict(body);actual.pop('joined_at');assert actual==expected
            assert body['participant_id']==pid and frame['payload']['via_connection']==data['via_connection']
        else:
            assert frame['type']=='session.participant.left' and fact['kind']=='session_participant_left'
            assert frame['payload']['participant_id']==data['participant_id']==pid
            assert timestamp(frame['payload']['left_at'])==data['left_at']<=fact['observed_at']
            assert frame['payload']['reason']==data['reason']
    assert frames[0]['payload']['via_connection']=='presence-first'
    assert frames[2]['payload']['via_connection']=='presence-third'
    assert frames[1]['payload']['reason']=='all-connections-ended'
    assert frames[3]['payload']['reason']=='explicit-leave'
    print(row['algorithm']+': joined/left originals, exact source attribution, repeated appearance and HTTP recovery verified')
