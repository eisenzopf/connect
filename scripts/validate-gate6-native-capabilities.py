#!/usr/bin/env python3
"""Validate actual SDK-observed unsolicited native descriptor envelopes."""
import argparse
import collections
import hashlib
import json
from pathlib import Path

import jsonschema

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('observations', type=Path)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    path = args.observations.resolve()
    assert path.is_relative_to((ROOT / '.local').resolve()), 'Use ignored observations'
    schema_path = ROOT / 'contracts/schemas/uctp-envelope-2.schema.json'
    validator = jsonschema.Draft202012Validator(
        json.loads(schema_path.read_text()), format_checker=jsonschema.FormatChecker())
    counts = collections.Counter()
    rows = [json.loads(line) for line in path.read_text().splitlines() if line]
    assert rows, 'No actual observations'
    for row in rows:
        algorithm, frame = row['algorithm'], row['frame']
        assert algorithm in ('EdDSA', 'ES256')
        assert frame['type'] == 'capability.advertise'
        assert frame['signature']['alg'] == algorithm
        assert all(frame.get(scope) for scope in ('cid', 'sid', 'connid'))
        assert 'in_reply_to' not in frame and 'operation_id' not in frame
        assert frame['payload']['by_participant']
        assert isinstance(frame['payload']['capabilities'], dict)
        validator.validate(frame)
        counts[algorithm] += 1
    assert counts['EdDSA'] and counts['ES256'], 'Both actual signature profiles required'
    report = {
        'status': 'passed',
        'scope': 'Contract shape of actual isolated SDK-verified Connect native listener facts. This checker does not verify signatures, privacy, actual delivery custody, or unexecuted vectors.',
        'observations': str(args.observations),
        'observations_sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
        'schema_sha256': hashlib.sha256(schema_path.read_bytes()).hexdigest(),
        'actual_frames': len(rows),
        'samples': dict(sorted(counts.items())),
    }
    if args.output:
        args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))


if __name__ == '__main__':
    main()
