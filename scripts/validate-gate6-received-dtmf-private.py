#!/usr/bin/env python3
"""Validate captured SIP input across an actual native private audience."""
import argparse
import json
from pathlib import Path
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('samples', type=Path)
    parser.add_argument('--source-kind', choices=['initial-sip', 'established-sip'], default='initial-sip')
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
        assert row['calls'] == 1
        if args.source_kind == 'established-sip':
            assert row['private_audio_marked'] == [0, 0]
            assert len(row['outside']) == 1
            outside = row['outside'][0]
            assert outside['source_kind'] == 'established-sip'
            assert outside['record_id'] == row['moved_record'] and outside['generation'] == 2
            assert outside['route_credential_id'] and outside['route_owner_id'] != outside['identity_id']
            assert outside['connection_id'] != outside['runtime_connection_id']
            assert outside['sip_call_id'] and outside['endpoint_id'] and outside['device_id']
            for key in ['endpoint_credential_id', 'key_id', 'key_algorithm', 'public_key_sha256']:
                assert key not in outside

        primary, private, public = row['private_primary'], row['private_candidate'], row['public_sibling']
        assert len({primary, private, public}) == 3
        operations, conversations = set(), set()
        for phase, frames, kind in [('baseline', row['baseline_frames'], 'sip-info'), ('private', [row['private_frame']], 'sip-info'), ('recovered', row['recovered_frames'], 'rfc4733')]:
            record = row[phase]
            status.validate(record)
            assert record['operation_id'] not in operations
            operations.add(record['operation_id'])
            if phase == 'private':
                assert record['state'] == 'failed'
                assert record['problem']['code'] == 'partial-submission'
                result = record['problem']['signal']
            else:
                assert record['state'] == 'completed'
                result = record['result']
            signal.validate(result)
            assert result['signal_source'] == result['source_observation']['kind'] == kind
            assert result['source_observation_id'] == result['source_observation']['observation_id']
            assert result['received_at'] == result['source_observation']['received_at']
            recipients = result['recipients']
            assert len(recipients) == 2
            assert {r['connection_id'] for r in recipients} == {primary, public}
            assert not any(r['connection_id'] == private for r in recipients)
            assert next(r for r in recipients if r['connection_id'] == public)['participant_id'] == row['same_participant']
            if phase == 'private':
                blocked = next(r for r in recipients if r['connection_id'] == primary)
                assert blocked['disposition'] == 'not_started'
                assert blocked['frame_id'] != row['private_frame']['id']
                assert {f['connid'] for f in frames} == {public}
            else:
                assert {f['connid'] for f in frames} == {primary, public}
            for frame in frames:
                wire.validate(frame)
                assert frame['signature']['alg'] == row['algorithm']
                assert frame['type'] == 'dtmf.received' and frame['sid'] == 'native-session'
                conversations.add(frame['cid'])
                assert frame['operation_id'] == record['operation_id']
                assert frame['payload']['source'] == kind
                assert frame['payload']['digits'] == result['source_observation']['digits'] == '5'
                assert frame['payload']['duration_ms'] == result['source_observation']['duration_ms'] == 237
                assert frame['payload']['received_at'] == result['received_at']
                recipient = next(r for r in recipients if r['connection_id'] == frame['connid'])
                assert recipient['disposition'] == 'local_submitted' and recipient['frame_id'] == frame['id']
                assert frame['id'] not in frame_ids
                frame_ids.add(frame['id'])
            for key in ['stream_revision', 'envelope_id', 'signing_key_id', 'signing_algorithm']:
                assert key not in result['source_observation']
        assert len(conversations) == 1 and len(operations) == 3
    assert len(frame_ids) == 10
    print(json.dumps({'passed': True, 'installed_scenes': 2, 'received_operations': 6, 'signed_native_frames': 10, 'source_kind': args.source_kind, 'scope': 'Local selected SIP source outside an actual native private pair. Store excludes privately admitted recipient; current Core audience excludes the public primary while its private slot is active. A public Device of the same consultant Participant receives. Return/replay/fresh-input recovery and owned cleanup are tested by the production fixture. No private SIP slots, WebRTC, crash, capacity or complete Gate6 claim.'}))


if __name__ == '__main__':
    main()
