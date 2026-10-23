#!/usr/bin/env python3
"""Validate actual signed initial Session activation after participant coverage."""
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
    algorithm = row['algorithm']
    assert algorithm not in seen
    seen.add(algorithm)
    frame = row['started']
    wire.validate(frame)
    assert frame['type'] == 'session.started' and frame['sid'] == 'native-session'
    assert frame['signature']['alg'] == algorithm
    assert all(key not in frame for key in ['connid', 'operation_id', 'in_reply_to'])
    body = frame['payload']
    assert len(body['participants_present']) == 2
    active = body['active_connections']
    assert len(active) == 3 and {c['connid'] for c in active} == {'activation-first', 'activation-second', 'activation-peer'}
    assert {c['participant_id'] for c in active} == set(body['participants_present'])
    assert all(c['transport'] == 'quic' for c in active)
    handles = row['same_peer_handles']
    assert len(handles) == len(set(handles)) == 2
    assert all(1 <= handle <= 65535 for handle in handles + [row['peer_handle']])
    assert row['accepted_without_all_ready'] and row['negotiation_without_flow_is_not_active']
    print(f'{algorithm}: all accepted Participants covered by actual ready Connections; signed original snapshot valid')
assert seen == {'EdDSA', 'ES256'}
