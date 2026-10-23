#!/usr/bin/env python3
"""Validate actual HTTP context samples against OpenAPI and both SDK decoders."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import subprocess
import sys
from jsonschema import Draft202012Validator, FormatChecker

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--samples', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    samples = json.loads(args.samples.read_text())
    api = json.loads((ROOT / 'contracts/openapi-context.json').read_text())
    schema = {'$ref': '#/components/schemas/Response', 'components': api['components']}
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema, format_checker=FormatChecker())
    sys.path.insert(0, str(ROOT / 'sdk/python'))
    from connect_sdk.protocol import decode_response
    seen = {}
    for sample in samples:
        response = sample['response']
        validator.validate(response)
        decoded = decode_response(json.dumps(response), 'context_view')
        view = decoded['data']
        records = [item for item in view['items'] if 'record' in item]
        assert records
        for item in records:
            reference = item['record']
            assert item['source_id'] == reference['uuid']
            if reference['state'] == 'available':
                digest = base64.urlsafe_b64encode(hashlib.sha512(item['body'].encode()).digest()).decode().rstrip('=')
                assert reference['content_hash'] == 'sha512-' + digest
                assert json.loads(item['body'])['uuid'] == reference['uuid']
            else:
                assert item['body'] == '' and reference['content_hash'] is None
        key = (sample['algorithm'], sample['case'])
        assert key not in seen
        seen[key] = records
    assert set(seen) == {(algorithm, case) for algorithm in ('EdDSA', 'ES256')
                         for case in ('available', 'recipient-ceiling', 'deleted')}
    for algorithm in ('EdDSA', 'ES256'):
        full = seen[(algorithm, 'available')][0]
        narrow = seen[(algorithm, 'recipient-ceiling')][0]
        assert full['record']['content_hash'] != narrow['record']['content_hash']
        assert len(narrow['body']) < len(full['body'])
        assert all(item['record']['state'] == 'deleted' for item in seen[(algorithm, 'deleted')])
    subprocess.run(['node', '--input-type=module', '-e',
        'import fs from "node:fs"; const {decodeResponse}=await import(process.argv[1]); '
        'for(const s of JSON.parse(fs.readFileSync(0,"utf8"))) decodeResponse(s.response,"context_view");',
        (ROOT / 'sdk/typescript/dist/protocol.js').as_uri()],
        input=json.dumps(samples), text=True, check=True, capture_output=True)
    report = dict(passed=True, actual_responses=len(samples),
                  algorithms=['EdDSA', 'ES256'], validators=['OpenAPI', 'Python SDK', 'TypeScript SDK'],
                  scope='Actual installed HTTP context responses; source identity, exact projected digest, recipient attenuation and public local erasure. No provider-memory or native IO qualification.')
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))


if __name__ == '__main__':
    main()
