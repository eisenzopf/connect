#!/usr/bin/env python3
"""Validate actual signed native invitation-to-record journeys and private views."""
import base64
import hashlib
import json
import pathlib
import sys
from jsonschema import Draft202012Validator
ROOT=pathlib.Path(__file__).resolve().parents[1]
wire=Draft202012Validator(json.loads((ROOT/'contracts/schemas/uctp-envelope-2.schema.json').read_text()))
api=json.loads((ROOT/'contracts/openapi-events.json').read_text())
journal=Draft202012Validator({'$ref':'#/components/schemas/JournalEvent','components':api['components']})
paths=sorted(pathlib.Path(sys.argv[1]).glob('*-journey-*.json'))
assert len(paths)==4, paths
assert {(json.loads(p.read_text())['algorithm'],json.loads(p.read_text())['recovery']) for p in paths}=={(a,r) for a in ['EdDSA','ES256'] for r in [False,True]}
for path in paths:
    s=json.loads(path.read_text());a=s['algorithm']
    for key in ['owner_frames','peer_frames','denied_frames']:
        for frame in s[key]:
            wire.validate(frame)
            assert frame['signature']['alg']==a
    owner=s['owner_frames'];peer=s['peer_frames']
    relay=next(e for e in owner if e['type']=='session.accept')
    invitation=next(e for e in peer if e['type']=='session.invite')
    assert relay['in_reply_to']==s['invite_request']
    assert relay['payload']['by']==invitation['payload']['to'][0]
    assert relay['cid']==invitation['cid'] and relay['sid']==invitation['sid']=='native-session'
    ready=[]
    for frames in [owner,peer]:
        assert any(e['type']=='connection.answer' for e in frames)
        assert any(e['type']=='connection.offer' for e in frames)
        started=next(e for e in frames if e['type']=='session.started')
        ended=next(e for e in frames if e['type']=='session.ended')
        hints=[e for e in frames if e['type']=='recording.vcon-ready'];assert len(hints)==1
        hint=hints[0];ready.append(hint)
        assert frames.index(started)<frames.index(ended)<frames.index(hint)
        for frame in [started,ended,hint]:
            assert not {'connid','operation_id','in_reply_to'}&frame.keys()
            assert frame['cid']==relay['cid'] and frame['sid']=='native-session'
        assert ended['payload']['by']==relay['payload']['by']
        assert ended['payload']['reason_code']==200 and ended['payload']['reason']=='journey-complete'
        assert hint['payload']['encrypted'] is False and hint['payload']['signed_by']==[]
        assert set(hint['payload']['vcon_handle'])=={'uuid','revision'}
        assert 'vcon' not in hint['payload']
    assert ready[0]['payload']==ready[1]['payload']
    assert not any(e['type']=='recording.vcon-ready' for e in s['denied_frames'])
    uid=ready[0]['payload']['vcon_handle']['uuid'];assert ready[0]['payload']['vcon_handle']['revision']==1
    for who,own,hidden in [('owner','journey-owner','journey-peer'),('peer','journey-peer','journey-owner')]:
        raw=s[who+'_record_bytes'];body=json.loads(raw)
        assert body==s[who+'_record'] and body['uuid']==uid and body['vcon']=='0.4.0'
        assert 'signatures' not in body
        outcome=body['analysis'][0]['body']
        assert outcome['state']=='ended' and outcome['capture_complete'] is False and outcome['recorded_audio'] is False
        assert own in raw and hidden not in raw
        digest='sha512-'+base64.urlsafe_b64encode(hashlib.sha512(raw.encode()).digest()).decode().rstrip('=')
        assert digest==s[who+'_hash']
    assert s['owner_hash']!=s['peer_hash'] and s['bidirectional_audio_verified']
    typed=[e for e in s['catchup'] if e['type']=='recording.vcon-ready'];assert len(typed)==1
    assert typed[0]['payload']==ready[0]['payload'] and typed[0]['id']!=ready[0]['id'] and typed[0]['in_reply_to']
    facts=[]
    for e in s['catchup']:
        wire.validate(e)
        if e['type']=='conversation.event':
            journal.validate(e['payload']['data'])
            if e['payload']['data']['data']['kind']=='record':facts.append(e['payload']['data'])
    assert len(facts)==1 and facts[0]['data']['data']['uuid']==uid
    print(f'{a}/{"recovery" if s["recovery"] else "normal"}: full native lifecycle, independent ready, explicit fetch and exact private hashes verified')
