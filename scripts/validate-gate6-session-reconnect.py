#!/usr/bin/env python3
"""Validate signed production reconnect/end samples against current references."""
import datetime
import json
import pathlib
import sys
from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
api = json.loads((ROOT / 'contracts/openapi-events.json').read_text())
journal = Draft202012Validator({'$ref': '#/components/schemas/JournalEvent', 'components': api['components']})
wire = Draft202012Validator(json.loads((ROOT / 'contracts/schemas/uctp-envelope-2.schema.json').read_text()))
directory = pathlib.Path(sys.argv[1])
for algorithm in ['EdDSA', 'ES256']:
    sample = json.loads((directory / f'{algorithm}-session-reconnect.json').read_text())
    hint = sample['ended']
    wire.validate(hint)
    assert hint['signature']['alg'] == algorithm and sample['algorithm'] == algorithm
    assert hint['type'] == 'session.ended' and hint['sid'] == sample['same_session'] == 'native-session'
    assert not {'connid', 'operation_id', 'in_reply_to'} & hint.keys()
    assert len(sample['journal']) == 1
    event = sample['journal'][0]
    journal.validate(event)
    fact = event['data']['data']
    assert event['data']['kind'] == 'session_ended' and fact['session']['state'] == 'ended'
    assert hint['cid'] == event['conversation_id'] and hint['sid'] == event['session_id']
    assert event['connection_id'] is None
    payload = hint['payload']
    assert payload['by'] == fact['by_participant'] and fact['by_membership_epoch'] > 0
    assert payload['reason_code'] == fact['reason_code'] == 486
    assert payload['reason'] == fact['reason'] == 'explicit-completion'
    assert datetime.datetime.fromisoformat(payload['ended_at'].replace('Z', '+00:00')).timestamp() == fact['ended_at']
    recovered = [e for e in sample['catchup'] if e['type'] == 'session.ended']
    assert len(recovered) == 1 and recovered[0]['payload'] == payload
    assert recovered[0]['id'] != hint['id'] and recovered[0]['in_reply_to']
    for e in sample['catchup']:
        wire.validate(e)
    default = json.loads((directory / f'{algorithm}-session-default-grace.json').read_text())
    hint = default['ended']
    wire.validate(hint)
    assert hint['signature']['alg'] == algorithm and hint['type'] == 'session.ended'
    assert default['deadline_millis'] - default['lost_at_millis'] == 30_000
    sent = datetime.datetime.fromisoformat(hint['ts'].replace('Z', '+00:00')).timestamp() * 1000
    assert default['deadline_millis'] <= sent < default['deadline_millis'] + 5000
    assert hint['payload']['reason_code'] == 408 and hint['payload']['reason'] == 'reconnect-grace-expired'
    assert hint['payload'].get('by') is None and default['late_offer_refused']
    assert not {'connid', 'operation_id', 'in_reply_to'} & hint.keys()
    pending = json.loads((directory / f'{algorithm}-session-pending-grace.json').read_text())
    wire.validate(pending['joined'])
    assert pending['joined']['signature']['alg'] == algorithm
    assert pending['joined']['payload']['via_connection'] == 'pending-reconnect'
    assert pending['delay_millis'] > pending['configured_grace_seconds'] * 1000
    assert pending['session_state'] == 'active'
    restart = json.loads((directory / f'{algorithm}-session-restart-grace.json').read_text())
    assert restart['deadline_millis'] - restart['lost_at_millis'] == restart['frozen_grace_seconds'] * 1000 == 5000
    assert restart['restart_default_seconds'] == 30 and restart['runtime_changed']
    assert len(restart['journal']) == 1
    journal.validate(restart['journal'][0])
    fact = restart['journal'][0]['data']['data']
    assert fact['reason_code'] == 408 and fact['reason'] == 'reconnect-grace-expired'
    assert fact['by_participant'] is None
    print(f'{algorithm}: signed reconnect, explicit end/replay, actual 30-second expiry, pending negotiation and owned restart verified')
