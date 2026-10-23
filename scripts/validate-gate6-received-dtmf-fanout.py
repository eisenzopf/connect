#!/usr/bin/env python3
"""Validate actual sibling-connection delivery and credential-local revocation."""
import argparse
import json
from pathlib import Path
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('samples', type=Path)
    args = parser.parse_args()
    raw = args.samples.read_bytes()
    assert len(raw) < 1024 * 1024
    rows = [json.loads(line) for line in raw.splitlines() if line.strip()]
    assert len(rows) == 2
    assert {row['algorithm'] for row in rows} == {'EdDSA', 'ES256'}
    contract = json.loads((ROOT / 'contracts/openapi-operations.json').read_text())
    signal = Draft202012Validator({**contract, '$ref': '#/components/schemas/ReceivedDtmfSubmission'})
    wire_contract = json.loads((ROOT / 'contracts/schemas/uctp-envelope-2.schema.json').read_text())
    wire = Draft202012Validator(wire_contract)
    status = Draft202012Validator({'$defs': wire_contract['$defs'], '$ref': '#/$defs/OperationUpdated'})
    frame_ids = set()
    for row in rows:
        assert row['same_participant'] and row['calls'] == 1
        first, after = row['first'], row['after_revocation']
        assert first['operation_id'] != after['operation_id']
        for record, frames, kind in [(first, row['first_frames'], 'sip-info'), (after, [row['after_frame']], 'rfc4733')]:
            status.validate(record)
            assert record['state'] == 'completed'
            result = record['result']
            signal.validate(result)
            assert result['signal_source'] == result['source_observation']['kind'] == kind
            recipients = result['recipients']
            assert len(recipients) == len(frames)
            assert {r['connection_id'] for r in recipients} == {f['connid'] for f in frames}
            assert len({r['frame_id'] for r in recipients}) == len(frames)
            for frame in frames:
                wire.validate(frame)
                assert frame['signature']['alg'] == row['algorithm']
                assert frame['type'] == 'dtmf.received' and frame['sid'] == 'native-session'
                assert frame['operation_id'] == record['operation_id']
                assert frame['payload']['source'] == kind
                assert frame['payload']['digits'] == result['source_observation']['digits'] == '5'
                assert frame['payload']['duration_ms'] == result['source_observation']['duration_ms'] == 237
                assert frame['payload']['received_at'] == result['received_at']
                assert frame['id'] not in frame_ids
                frame_ids.add(frame['id'])
                recipient = next(r for r in recipients if r['connection_id'] == frame['connid'])
                assert recipient['frame_id'] == frame['id'] and recipient['disposition'] == 'local_submitted'
        recipients = first['result']['recipients']
        assert {r['connection_id'] for r in recipients} == {'initial-native', 'received-sibling'}
        assert recipients[0]['participant_id'] == recipients[1]['participant_id']
        assert recipients[0]['membership_epoch'] == recipients[1]['membership_epoch']
        assert after['result']['recipients'][0]['connection_id'] == 'initial-native'
    print(json.dumps({'passed': True, 'installed_scenes': 2, 'received_operations': 4, 'signed_native_frames': len(frame_ids), 'scope': 'Local initial SIP input to two native Connections of one Participant; revoking only one credential retires only that recipient, replay does not redeliver, fresh input reaches the survivor. SDK verifies signatures. No broader transport, privacy, crash, capacity or Gate6 completion claim.'}))


if __name__ == '__main__':
    main()
