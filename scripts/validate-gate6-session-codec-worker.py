#!/usr/bin/env python3
"""Validate captured actual signed Session codec reoffers and public status.

The production runtime tests separately verify exact signed peer confirmation,
original replay, real media, storage recovery and no repeated physical effects.
"""
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
    algorithm, mode = row['algorithm'], row['mode']
    assert (algorithm, mode) not in seen
    seen.add((algorithm, mode))
    offers = row['offers']
    expected = 1 if mode == 'accepted-fallback' else 2
    assert len(offers) == expected
    operations = set()
    for frame in offers:
        wire.validate(frame)
        assert frame['type'] == 'connection.offer'
        assert frame['signature']['alg'] == algorithm
        assert frame['sid'] == 'native-session'
        assert frame['operation_id'].startswith('scodec_child_')
        assert frame['operation_id'] not in operations
        operations.add(frame['operation_id'])
        setup = frame['payload']['substrate_setup']
        assert setup['kind'] == 'native-prepared/1'
        assert setup['revision'] == frame['expected_revision'] + 1
        codec = 'pcma' if mode == 'latest' else 'pcmu'
        assert setup['streams'][0]['codec']['name'] == codec
    observed = row['observed']
    decision = row['decision']['data']
    assert row['decision']['kind'] == 'session_control'
    assert observed['operation_id'] == decision['codec_reconciliation']['operation_id']
    assert observed['record_id'] == decision['codec_reconciliation']['record_id']
    if mode == 'bad-confirmation':
        assert observed['state'] == 'unknown'
        assert observed.get('result') is None
    else:
        assert observed['state'] == 'completed'
        assert observed['result']['policy_operation_id'] == decision['operation_id']
        assert observed['result']['policy_revision'] == decision['session']['topology_revision']
        counts = observed['result']['counts']
        assert counts['targets'] == 2 and counts['updated'] == expected
        assert counts['unchanged'] == 2 - expected
        assert all(counts[key] == 0 for key in ['queued', 'preparing', 'refused', 'unavailable', 'cancelled', 'superseded', 'unknown'])
    print(f'{algorithm}/{mode}: signed prepared reoffers and truthful aggregate status verified')
assert seen == {(alg, mode) for alg in ['EdDSA', 'ES256'] for mode in ['preferred', 'latest', 'accepted-fallback', 'recording-recovery', 'bad-confirmation']}
