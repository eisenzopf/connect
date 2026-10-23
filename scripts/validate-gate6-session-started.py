#!/usr/bin/env python3
"""Validate actual signed SDK/native start samples and original HTTP catch-up."""
import datetime
import json
import pathlib
import sys

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
api = json.loads((ROOT / 'contracts/openapi-events.json').read_text())
schema = {'$ref': '#/components/schemas/JournalEvent', 'components': api['components']}
validator = Draft202012Validator(schema)
wire = Draft202012Validator(json.loads((ROOT / 'contracts/schemas/uctp-envelope-2.schema.json').read_text()))
paths = sorted(pathlib.Path(sys.argv[1]).glob('*-session-started.json'))
assert len(paths) == 2
assert {json.loads(p.read_text())['algorithm'] for p in paths} == {'EdDSA', 'ES256'}
for path in paths:
    sample = json.loads(path.read_text())
    hint, event = sample['hint'], sample['journal']
    wire.validate(hint)
    validator.validate(event)
    assert hint['type'] == 'session.started' and hint.get('signature')
    assert hint['signature']['alg'] == sample['algorithm']
    assert not {'connid', 'operation_id', 'in_reply_to'} & hint.keys()
    assert hint['cid'] == event['conversation_id'] and hint['sid'] == event['session_id']
    assert event['kind'] == event['data']['kind'] == 'session_started'
    assert event['connection_id'] is None and event['operation_id'] is None
    fact, payload = event['data']['data'], hint['payload']
    assert fact['session']['state'] == 'active' and sample['terminal_state'] == 'ended'
    assert fact['session']['revision'] == event['revision']
    assert len(payload['participants_present']) == 2
    assert len(payload['active_connections']) == 2
    assert {c['participant_id'] for c in payload['active_connections']} == set(payload['participants_present'])
    assert sample['recipient_connections'] == 0
    assert sample['narrow_scope_barrier']['type'] == 'ack'
    assert datetime.datetime.fromisoformat(payload['started_at'].replace('Z', '+00:00')).timestamp() == fact['started_at']
    assert 0 < fact['started_at'] <= event['observed_at']
    for field in ['participants_present', 'active_connections', 'negotiated_capabilities']:
        assert payload[field] == fact[field]
    assert payload['negotiated_capabilities']['audio_codecs'][0]['name'] == 'g.711-mu'
    assert {c['connid'] for c in payload['active_connections']} == {'started-connection', 'accepted-peer-bootstrap'}
    assert all(c['transport'] == 'quic' for c in payload['active_connections'])
    typed = [e for e in sample['catchup'] if e['type'] == 'session.started']
    assert len(typed) == 1 and typed[0]['payload'] == payload and typed[0]['in_reply_to']
    print(f'{sample["algorithm"]}: original native start, narrow-scope refusal and terminal HTTP recovery verified')
