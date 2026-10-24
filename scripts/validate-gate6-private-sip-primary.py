#!/usr/bin/env python3
"""Validate SDK-observed selected SIP-primary captures and original-call correlations."""
import argparse
import hashlib
import json
from pathlib import Path
import jsonschema

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('observations', type=Path)
    parser.add_argument('--source-kind', choices=['initial-sip', 'established-sip'], required=True)
    args = parser.parse_args()
    path = args.observations.resolve()
    assert path.is_relative_to((ROOT / '.local').resolve())
    raw = path.read_bytes()
    assert len(raw) < 1024 * 1024
    rows = [json.loads(line) for line in raw.splitlines() if line.strip()]
    assert len(rows) == 2 and {row['algorithm'] for row in rows} == {'EdDSA', 'ES256'}
    schema_path = ROOT / 'contracts/schemas/uctp-envelope-2.schema.json'
    schema = json.loads(schema_path.read_text())
    validator = jsonschema.Draft202012Validator(schema, format_checker=jsonschema.FormatChecker())
    status = jsonschema.Draft202012Validator({'$defs': schema['$defs'], '$ref': '#/$defs/OperationUpdated'})
    seen = set()
    for row in rows:
        assert row['source_kind'] == args.source_kind
        private = row['selection']['private']
        assert len(private) == 2
        phone = next(p for p in private if p['connection_id'] == row['phone'])
        native = next(p for p in private if p['connection_id'] != row['phone'])
        assert phone['source_kind'] == args.source_kind
        assert phone['generation'] == (1 if args.source_kind == 'initial-sip' else 2)
        assert 'key_id' not in phone and 'credential_id' not in phone
        assert row['selection']['remote']['connection_id'] == row['retained']
        if args.source_kind == 'established-sip':
            assert phone['record_id'] == row['prior_move']['record_id']
            assert all(row['prior_move'][key] is True for key in ('controller_revoked', 'source_credential_refused', 'preparation_expired', 'lifetime_current'))
            assert 'endpoint_credential_id' not in phone
        assert row['calls'] == 1 and row['source_operation_count'] == 4
        assert row['private_sip_to_native_marked'][0] == 0
        assert row['private_sip_to_native_marked'][1] > 10 and row['private_native_to_sip_marked'] > 10
        assert row['native_private_info_count'] == 1
        assert len(row['private_frames']) == len(row['private_statuses']) == 2
        frames = [row['baseline_frame'], *row['private_frames'], row['restored_frame']]
        statuses = [row['baseline'], *row['private_statuses'], row['restored']]
        assert len({s['operation_id'] for s in statuses}) == 4
        for frame, saved in zip(frames, statuses):
            validator.validate(frame)
            status.validate(saved)
            assert saved['state'] == 'completed' and frame['operation_id'] == saved['operation_id']
            assert frame['type'] == 'dtmf.received' and frame['signature']['alg'] == row['algorithm']
            assert all(frame.get(key) for key in ('cid', 'sid', 'connid', 'operation_id'))
            assert frame['sid'] == 'native-session' and frame['payload']['digits'] == '5'
            assert frame['id'] not in seen
            seen.add(frame['id'])
        assert len({f['cid'] for f in frames}) == 1
        assert [f['payload']['source'] for f in row['private_frames']] == ['sip-info', 'rfc4733']
        assert frames[0]['connid'] == frames[-1]['connid'] == row['retained']
        for frame, saved in zip(row['private_frames'], row['private_statuses']):
            assert frame['connid'] == native['connection_id']
            recipients = saved['result']['recipients']
            assert len(recipients) == 1 and recipients[0]['connection_id'] == native['connection_id']
            assert recipients[0]['disposition'] == 'local_submitted'
        status.validate(row['private_native_status'])
        assert row['private_native_status']['state'] == 'completed'
        outgoing = row['private_native_status']['result']['recipients']
        assert len(outgoing) == 1 and outgoing[0]['connection_id'] == row['phone']
        assert outgoing[0]['disposition'] == 'local_submitted'
        returned = row['returned']
        assert returned['state'] == 'completed' and returned['result']['remote_continuing'] is True
        assert set(returned['result']['retained_connection_ids']) == {row['phone'], row['retained']}
        assert returned['result']['retired_connection_ids'] == [native['connection_id']]
        assert len(returned['effects']) == 1 and returned['effects'][0]['attempt'] == 1
    print(json.dumps({'passed': True, 'scenes': 2, 'actual_frames': 8, 'source_kind': args.source_kind,
                      'observations_sha256': hashlib.sha256(raw).hexdigest(),
                      'schema_sha256': hashlib.sha256(schema_path.read_bytes()).hexdigest(),
                      'scope': 'Actual isolated local selected SIP-primary state, shape and correlations; SDK verifies signatures and runtime asserts decoded media/privacy, actual DTMF, original return/replay and cleanup. Other roles/profiles remain separate.'}))


if __name__ == '__main__':
    main()
