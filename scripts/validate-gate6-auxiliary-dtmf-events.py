#!/usr/bin/env python3
"""Check clause0114's local native wire shape against actual SDK/HTTP samples."""
import argparse
import datetime
import json
import pathlib

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('observations', type=pathlib.Path)
parser.add_argument('--output', type=pathlib.Path)
args = parser.parse_args()
rows = json.loads(args.observations.read_text())
assert len(rows) == 2 and {r['algorithm'] for r in rows} == {'EdDSA', 'ES256'}

def timestamp(value):
    parsed = datetime.datetime.fromisoformat(value.replace('Z', '+00:00'))
    assert parsed.tzinfo is not None
    return parsed

ids = set()
for row in rows:
    frame = row['actual_received']
    status = row['http_operation']
    payload = frame['payload']
    assert frame['v'] == 2 and frame['type'] == 'dtmf.received'
    assert frame['signature']['alg'] == row['algorithm']
    assert isinstance(frame['cid'], str) and frame['cid'].startswith('conv_')
    assert frame['sid'] == 'native-session' and frame['connid'] == 'codec-source'
    assert frame['operation_id'] == status['operation_id'] == 'signal-http-summary'
    assert payload['digits'] == '8#' and payload['duration_ms'] == 100
    assert type(payload['duration_ms']) is int and payload['source'] == 'uctp-native'
    assert timestamp(payload['received_at']) <= timestamp(frame['ts'])
    assert status['state'] == 'completed'
    result = status['result']
    assert result['received_at'] == payload['received_at']
    assert result['signal_source'] == payload['source']
    assert result['local_submission_only'] is True and result['local_work_complete'] is True
    assert result['resources_changed'] is False
    assert len(result['recipients']) == 1
    recipient = result['recipients'][0]
    assert recipient['connection_id'] == frame['connid']
    assert recipient['disposition'] == 'local_submitted' and recipient['frame_id'] == frame['id']
    assert frame['id'] not in ids
    ids.add(frame['id'])

result = {
    'case_id': 'u6-req-0114', 'passed': True, 'observations': len(rows),
    'algorithms': ['EdDSA', 'ES256'], 'source': 'uctp-native',
    'scope': 'Actual local native QUIC SDK events with exact scope, digits/duration, original source receive time, native provenance and matching immutable HTTP local-submission facts.',
    'signature_verification': 'The producing public signed Rust SDK verifies the wire. This validator checks recorded fields and receipt identity; it does not reverify signatures.',
    'limits': 'No SIP/WebRTC source, remote playback, hosting profile, full DTMF group, full G6-046 task or acceptance qualification.'
}
if args.output:
    args.output.write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result))
