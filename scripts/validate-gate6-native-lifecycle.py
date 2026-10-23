#!/usr/bin/env python3
"""Validate actual SDK-observed native lifecycle envelopes, not synthetic vectors."""
import argparse
import collections
import hashlib
import json
from pathlib import Path

try:
    import jsonschema
except ImportError:
    raise SystemExit("Use a Python environment with scripts/requirements-gate6.txt installed")

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('observations', type=Path)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    path = args.observations.resolve()
    assert path.is_relative_to((ROOT / '.local').resolve()), 'Use ignored fixture observations'
    schema_path = ROOT / 'contracts/schemas/uctp-envelope-2.schema.json'
    schema = json.loads(schema_path.read_text())
    validator = jsonschema.Draft202012Validator(schema, format_checker=jsonschema.FormatChecker())
    counts = collections.Counter()
    rows = [json.loads(line) for line in path.read_text().splitlines() if line]
    assert rows, 'No actual observations'
    for row in rows:
        algorithm, frame = row['algorithm'], row['frame']
        assert algorithm in ('EdDSA', 'ES256')
        assert frame['type'] in ('connection.ready', 'stream.opened', 'stream.closed', 'connection.quality')
        assert frame['signature']['alg'] == algorithm
        assert all(frame.get(scope) for scope in ('cid', 'sid', 'connid'))
        assert 'in_reply_to' not in frame and 'operation_id' not in frame
        validator.validate(frame)
        counts[algorithm, frame['type']] += 1
    for algorithm in ('EdDSA', 'ES256'):
        for kind in ('connection.ready', 'stream.opened', 'connection.quality'):
            assert counts[algorithm, kind], (algorithm, kind)
    assert counts['EdDSA', 'stream.closed'], 'Actual codec replacement closure was not observed'
    report = {
        'status': 'passed',
        'scope': 'Actual isolated Connect listener events already verified by the signed public SDK; this checker validates contract shape and the required sample matrix, not cryptographic signatures or broader runtime behavior.',
        'observations': str(args.observations),
        'observations_sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
        'schema_sha256': hashlib.sha256(schema_path.read_bytes()).hexdigest(),
        'actual_frames': len(rows),
        'samples': [{'algorithm': algorithm, 'type': kind, 'count': count}
                    for (algorithm, kind), count in sorted(counts.items())],
    }
    if args.output:
        args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))


if __name__ == '__main__':
    main()
