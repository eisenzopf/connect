#!/usr/bin/env python3
"""Check captured signed subscription refusals and heterogeneous wildcard outcomes.

The production SDK verifies signatures during the run. This checks the retained
wire shapes, request correlation and durable replay observations independently.
"""
import argparse
import json
from pathlib import Path

from jsonschema import Draft202012Validator, FormatChecker

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('observations', type=Path)
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
wire = Draft202012Validator(json.loads(
    (root / 'contracts/schemas/uctp-envelope-2.schema.json').read_text()),
    format_checker=FormatChecker())
http = json.loads((root / 'contracts/openapi-canonical-http.json').read_text())
assert '488' in http['paths']['/v2/commands']['post']['responses']
problem = Draft202012Validator(http['components']['schemas']['Problem'])
refusals = {}
completed = set()
for line in args.observations.read_text().splitlines():
    row = json.loads(line)
    if 'error' not in row:
        frame = row['frame']
        wire.validate(frame)
        if row['operation_id'] == 'hetero-wildcard' and frame['type'] == 'operation.updated':
            assert frame['payload']['state'] == 'completed'
            assert frame['payload']['result']['stream_revision'] == 2
            completed.add((frame['signature']['alg'], frame['cid']))
        continue
    changed, error = row['operation'], row['error']
    for frame in [changed, error]:
        wire.validate(frame)
        assert frame['in_reply_to'] == row['request_id']
        assert frame['operation_id'] == row['operation_id']
        assert frame['sid'] == 'native-session' and frame['connid'] == 'hetero-target'
    assert changed['type'] == 'operation.updated' and changed['payload']['state'] == 'failed'
    assert error['type'] == 'error' and error['payload']['code'] in [404, 488]
    problem.validate(error['payload'])
    assert changed['id'] != error['id']
    assert changed['signature']['alg'] == error['signature']['alg']
    code = error['payload']['code']
    assert changed['payload']['problem']['code'] == {
        404: 'not-found', 488: 'incompatible-capabilities'}[code]
    refusals.setdefault((changed['signature']['alg'], changed['cid'], row['operation_id']), []).append(row)
for algorithm in ['EdDSA', 'ES256']:
    scenes = {cid for alg, cid in completed if alg == algorithm}
    assert len(scenes) == 1, (algorithm, scenes)
    cid, = scenes
    for operation, code, minimum in [
        ('hetero-missing-stream', 404, 2),
        ('hetero-missing-participant', 404, 2),
        ('hetero-incompatible-video', 488, 3),
    ]:
        rows = refusals[(algorithm, cid, operation)]
        assert len(rows) >= minimum
        assert all(r['error']['payload']['code'] == code for r in rows)
        assert all(r['operation']['payload'] == rows[0]['operation']['payload'] for r in rows)
        assert len({r['request_id'] for r in rows}) == len(rows)
        assert len({r['operation']['id'] for r in rows}) == len(rows)
        assert len({r['error']['id'] for r in rows}) == len(rows)
    print(f'{algorithm}: correlated 404/488, failed replay across later success and heterogeneous wildcard completion verified')
codec = [rows for (alg, _, op), rows in refusals.items()
         if alg == 'EdDSA' and op == 'hetero-no-common-format']
assert len(codec) == 1 and codec[0][0]['error']['payload']['code'] == 488
print('EdDSA: disjoint current data formats return 488 with a durable failed operation')
