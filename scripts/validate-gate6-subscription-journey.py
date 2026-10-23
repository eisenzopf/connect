#!/usr/bin/env python3
"""Check captured production subscription replies and stable removal outcomes."""
import argparse
import json
from pathlib import Path

from jsonschema import Draft202012Validator

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('observations', type=Path)
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
wire = Draft202012Validator(json.loads(
    (root / 'contracts/schemas/uctp-envelope-2.schema.json').read_text()))
groups = {}
for line in args.observations.read_text().splitlines():
    row = json.loads(line)
    frame = row['frame']
    wire.validate(frame)
    algorithm = frame['signature']['alg']
    if frame['type'] == 'operation.updated':
        assert frame['operation_id'] == row['operation_id']
        groups.setdefault((algorithm, frame['cid']), []).append(row)

qualified = []
for (algorithm, cid), rows in groups.items():
    def outcomes(operation):
        return [r for r in rows if r['operation_id'] == operation]

    selection = outcomes('future-select-participant-video')
    if not selection or 'kinds' in selection[0]['payload']['subscriptions'][0]:
        continue
    assert len(selection) == 1
    selected = selection[0]
    assert selected['command'] == 'stream.subscribe'
    assert set(selected['payload']['subscriptions'][0]) == {'from_participant'}
    assert selected['expected_revision'] == 1
    assert selected['frame']['payload']['state'] == 'completed'
    assert selected['frame']['payload']['result']['stream_revision'] == 2
    for name in ['select-no-source', 'select-no-participant', 'select-unsupported-screen']:
        refused, = outcomes(name)
        assert refused['frame']['payload']['state'] == 'failed'
        assert refused['frame']['payload']['problem']['code']
        assert not refused['frame']['payload'].get('result')
    removed = outcomes('future-remove-camera')
    assert len(removed) == 2
    assert removed[0]['frame']['id'] != removed[1]['frame']['id']
    assert removed[0]['frame']['payload'] == removed[1]['frame']['payload']
    result = removed[0]['frame']['payload']['result']
    for row in removed:
        assert row['command'] == 'stream.unsubscribe'
        assert row['payload'] == {'strm_ids': ['later-camera']}
        assert result['stream_revision'] == row['expected_revision'] + 1
    again, = outcomes('future-remove-camera-again')
    assert again['frame']['payload']['state'] == 'completed'
    assert again['expected_revision'] == result['stream_revision']
    assert again['frame']['payload']['result']['stream_revision'] == result['stream_revision']
    assert again['frame']['payload']['result']['resources_changed'] is False
    restored, = outcomes('future-restore-camera')
    assert restored['frame']['payload']['state'] == 'completed'
    assert restored['payload'] == {'subscriptions': [{'strm_id': 'later-camera'}]}
    assert restored['frame']['payload']['result']['stream_revision'] == result['stream_revision'] + 1
    qualified.append(algorithm)
    print(f'{algorithm}: unfiltered selector, atomic refusals, exact replay, no-op removal and explicit restore verified')
assert sorted(qualified) == ['ES256', 'EdDSA'], qualified
