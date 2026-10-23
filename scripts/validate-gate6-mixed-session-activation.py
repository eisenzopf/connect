#!/usr/bin/env python3
"""Validate actual signed native/SIP startup and captured recording recovery."""
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
    key = (row['algorithm'], row['sip_payload'], row['sip_last_storage_recovery'])
    assert key not in seen
    seen.add(key)
    frame = row['started']
    wire.validate(frame)
    assert frame['signature']['alg'] == row['algorithm']
    assert frame['type'] == 'session.started' and frame['sid'] == 'native-session'
    assert not {'connid', 'operation_id', 'in_reply_to'} & frame.keys()
    body = frame['payload']
    participants = set(body['participants_present'])
    connections = body['active_connections']
    assert len(participants) == len(connections) == 2
    assert {c['participant_id'] for c in connections} == participants
    assert {c['transport'] for c in connections} == {'quic', 'sip-interop'}
    native = next(c for c in connections if c['transport'] == 'quic')
    assert native['connid'] == 'mixed-native'
    caps = body['negotiated_capabilities']
    if row['sip_payload'] == 0:
        assert [v['name'] for v in caps['audio_codecs']] == ['g.711-mu']
    else:
        assert row['sip_payload'] == 8
        assert caps['audio_codecs'] == []
        exact = caps['connections']
        assert len(exact) == 2
        assert {c['connid'] for c in exact} == {c['connid'] for c in connections}
        assert {c['capabilities']['audio_codecs'][0]['name'] for c in exact} == {'g.711-mu', 'g.711-a'}
        own = next(c for c in exact if c['connid'] == native['connid'])
        assert own['capabilities']['audio_codecs'][0]['name'] == 'g.711-mu'
    assert row['unique_sip_calls'] == 1
    if row['sip_last_storage_recovery']:
        assert row['native_packets_after_sip_recording_released'] == 0
    print(f'{key}: two actual transport sources, per-Participant readiness, truthful negotiated formats and once-only SIP call verified')
assert seen == {(algorithm, payload, recovery) for algorithm in ['EdDSA', 'ES256'] for payload, recovery in [(0, False), (8, False), (0, True)]}
