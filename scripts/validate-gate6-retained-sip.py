#!/usr/bin/env python3
"""Validate actual SDK-observed retained-SIP captures; runtime owns media assertions."""
import argparse
import hashlib
import json
from pathlib import Path
import jsonschema

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('observations', type=Path)
    args = parser.parse_args()
    path = args.observations.resolve()
    assert path.is_relative_to((ROOT / '.local').resolve()), 'Use ignored fixture observations'
    raw = path.read_bytes()
    assert len(raw) < 1024 * 1024
    rows = [json.loads(line) for line in raw.splitlines() if line.strip()]
    assert len(rows) == 2 and {row['algorithm'] for row in rows} == {'EdDSA', 'ES256'}
    schema_path = ROOT / 'contracts/schemas/uctp-envelope-2.schema.json'
    schema = json.loads(schema_path.read_text())
    validator = jsonschema.Draft202012Validator(schema, format_checker=jsonschema.FormatChecker())
    status = jsonschema.Draft202012Validator({'$defs': schema['$defs'], '$ref': '#/$defs/OperationUpdated'})
    frame_ids = set()
    for row in rows:
        algorithm = row['algorithm']
        assert row['source_kind'] == 'sip-initial'
        remote = row['selection']['remote']
        assert remote['source_kind'] == 'initial-sip' and remote['generation'] == 1
        assert remote['connection_id'] == row['remote'] and 'key_id' not in remote
        assert row['calls'] == 1
        assert row['held_submitted'] == 0 and row['source_operation_count'] == 3
        assert row['info_count_private_before'] == row['info_count_private_after']
        for key in ('baseline', 'held', 'recovered'):
            status.validate(row[key])
        assert row['held']['operation_id'] == row['held_operation']
        assert len({row[key]['operation_id'] for key in ('baseline', 'held', 'recovered')}) == 3
        for key in ('baseline_frame', 'private_frame', 'recovered_frame'):
            frame = row[key]
            validator.validate(frame)
            assert frame['type'] == 'dtmf.received' and frame['signature']['alg'] == algorithm
            assert all(frame.get(scope) for scope in ('cid', 'sid', 'connid', 'operation_id'))
            assert frame['sid'] == 'native-session'
            assert frame['id'] not in frame_ids
            frame_ids.add(frame['id'])
        assert len({row[key]['cid'] for key in ('baseline_frame', 'private_frame', 'recovered_frame')}) == 1
        assert row['baseline_frame']['payload']['source'] == 'sip-info'
        assert row['baseline_frame']['payload']['digits'] == row['recovered_frame']['payload']['digits'] == '5'
        assert row['private_frame']['payload']['digits'] == '9'
        assert row['private_frame']['operation_id'] == 'private-native-digit'
        assert row['baseline']['state'] == row['recovered']['state'] == 'completed'
        assert row['baseline_frame']['operation_id'] == row['baseline']['operation_id']
        assert row['recovered_frame']['operation_id'] == row['recovered']['operation_id']
        assert row['recovered_frame']['operation_id'] != row['held_operation']
        assert row['recovered_frame']['payload']['source'] == 'rfc4733'
        returned = row['returned']
        assert returned['state'] == 'completed' and returned['result']['remote_continuing'] is True
        assert len(returned['result']['retired_connection_ids']) == 1
        assert len(returned['effects']) == 1 and returned['effects'][0]['attempt'] == 1
    print(json.dumps({'passed': True, 'scenes': len(rows), 'actual_frames': 6,
                      'observations_sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
                      'schema_sha256': hashlib.sha256(schema_path.read_bytes()).hexdigest(),
                      'scope': 'Actual local retained initial-SIP state, shape and correlations. SDK verifies signatures; runtime checks decoded media, private exclusion, source settlement, return/replay and cleanup. Not independent cryptography or other profiles.'}))


if __name__ == '__main__':
    main()
