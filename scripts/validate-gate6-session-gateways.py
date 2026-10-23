#!/usr/bin/env python3
"""Check real gateway observations; signed replies were verified by the SDK."""
import argparse
import json
from pathlib import Path
from jsonschema import Draft202012Validator, FormatChecker

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('observations', type=Path)
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
wire = Draft202012Validator(json.loads((root / 'contracts/schemas/uctp-envelope-2.schema.json').read_text()), format_checker=FormatChecker())
seen = set()
for line in args.observations.read_text().splitlines():
    row = json.loads(line)
    request = row['logical_request']
    reference = row['requested_reference']
    assert request['action']['kind'] == 'invite'
    assert request['action']['participants'] == [reference]
    assert row['sip_calls'] == 1
    assert row['destination'] in row['actual_sip_to']
    if row['reference_kind'] == 'uri':
        assert row['stored_reference'] == reference
        scheme = reference.split(':', 1)[0]
        assert scheme in ('tel', 'sip')
        if scheme == 'tel':
            assert row['destination'].startswith('sip:' + reference[4:] + '@')
        else:
            assert row['destination'] == reference
    else:
        assert row['reference_kind'] == 'identity'
        assert row['stored_reference'] is None
        assert ':' not in reference
        assert not row['http_first']
        scheme = 'identity'
    key = (row['algorithm'], scheme, row['http_first'])
    assert key not in seen
    seen.add(key)
    reply, started = row['reply'], row['started']
    for frame in [reply, started]:
        wire.validate(frame)
        assert frame['signature']['alg'] == row['algorithm']
        assert frame['cid'] == request['conversation_id']
        assert frame['sid'] == request['session_id']
    assert reply['type'] == 'operation.updated'
    assert reply['payload']['state'] == 'completed'
    assert reply['payload']['result'] is not None
    assert started['type'] == 'session.started'
    assert not {'operation_id', 'connid', 'in_reply_to'} & started.keys()
    connections = started['payload']['active_connections']
    assert len(connections) == 2
    assert {c['transport'] for c in connections} == {'quic', 'sip-interop'}
    assert len({c['participant_id'] for c in connections}) == 2
    print(f'{key}: original selector, actual SIP destination, one call, verified SDK reply and mixed startup checked')
assert seen == {(alg, scheme, http) for alg in ['EdDSA', 'ES256'] for scheme, http in [('tel', False), ('sip', False), ('tel', True), ('sip', True), ('identity', False)]}
