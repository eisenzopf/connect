#!/usr/bin/env python3
"""Validate private real HTTP enrollment samples against the generated reference.

The HTTP fixture contains synthetic credentials; samples and the output report
must remain under this checkout's ignored .local directory. No network access.
"""
import argparse
import hashlib
import json
from pathlib import Path

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--samples', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    for path in (args.samples, args.output):
        assert path.resolve().is_relative_to((ROOT / '.local').resolve())
    api = json.loads((ROOT / 'contracts/openapi-enrollment.json').read_text())
    for schema in api['components']['schemas'].values():
        Draft202012Validator.check_schema(schema)
    fields = json.loads((ROOT / 'contracts/schemas/enrollment-1.schema.json').read_text())['properties']
    samples = sorted(args.samples.glob('*.json'))
    assert len(samples) == 6, 'Expected six independent algorithm/transport scenes'
    checked = 0
    for sample in samples:
        values = json.loads(sample.read_text())
        assert set(values) == {'request', 'claim', 'revoke', 'offer_response',
                               'credential_response', 'revoke_response'}
        for name, value in values.items():
            reference = fields[name]['$ref'].replace('#/$defs/', '#/components/schemas/')
            validator = Draft202012Validator({'$ref': reference, 'components': api['components']})
            # Do not print ValidationError: its context may contain fixture secrets.
            assert validator.is_valid(value), f'HTTP {name} failed schema validation'
            checked += 1
    report = {'passed': True, 'scenes': len(samples), 'actual_http_bodies': checked,
              'schema_sha256': hashlib.sha256((ROOT / 'contracts/openapi-enrollment.json').read_bytes()).hexdigest(),
              'scope': 'Real isolated HTTPS fixture bodies match derived DTO/OpenAPI shapes; no browser/native transport or full G6-055 qualification.'}
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))


if __name__ == '__main__':
    main()
