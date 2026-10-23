#!/usr/bin/env python3
"""Validate captured installed SIP input, native output and HTTP status facts."""
import argparse
import datetime
import json
from pathlib import Path
from jsonschema import Draft202012Validator
ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('samples', type=Path)
    parser.add_argument('--failure-samples', type=Path)
    args = parser.parse_args()
    raw = args.samples.read_bytes()
    assert len(raw) < 1024 * 1024
    rows = [json.loads(line) for line in raw.splitlines() if line.strip()]
    assert len(rows) == 4
    contract = json.loads((ROOT/'contracts/openapi-operations.json').read_text())
    validator = Draft202012Validator({**contract, '$ref': '#/components/schemas/ReceivedDtmfSubmission'})
    wire_contract = json.loads((ROOT/'contracts/schemas/uctp-envelope-2.schema.json').read_text())
    record_validator = Draft202012Validator({'$defs': wire_contract['$defs'], '$ref': '#/$defs/OperationUpdated'})
    wire = Draft202012Validator(wire_contract)
    profiles, frames = set(), set()
    for row in rows:
        record, frame = row['operation'], row['native_frame']
        record_validator.validate(record)
        assert record['state'] == 'completed'
        result = record['result']
        validator.validate(result)
        wire.validate(frame)
        facts, payload = result['source_observation'], frame['payload']
        profiles.add((row['algorithm'], facts['kind']))
        assert frame['signature']['alg'] == row['algorithm']
        assert frame['type'] == 'dtmf.received' and frame['sid'] == 'native-session'
        assert frame['operation_id'] == record['operation_id'] and record['operation_id'].startswith('dtmfr_')
        assert result['signal_source'] == payload['source'] == facts['kind']
        assert result['source_observation_id'] == facts['observation_id']
        assert result['received_at'] == payload['received_at'] == facts['received_at']
        assert datetime.datetime.fromisoformat(facts['received_at'].replace('Z', '+00:00')).tzinfo
        assert payload['digits'] == facts['digits'] == '5'
        assert payload['duration_ms'] == facts['duration_ms'] == 237
        assert len(result['recipients']) == 1
        recipient = result['recipients'][0]
        assert recipient['frame_id'] == frame['id'] and recipient['connection_id'] == frame['connid']
        assert recipient['disposition'] == 'local_submitted' and frame['id'] not in frames
        frames.add(frame['id'])
        for key in ['stream_revision', 'envelope_id', 'signing_key_id', 'signing_algorithm']:
            assert key not in result and key not in facts
        if facts['kind'] == 'rfc4733':
            assert facts['rtp']['duration_samples'] == 1896 and facts['rtp']['clock_rate_hz'] == 8000
            assert facts['rtp']['ssrc'] == 0x87654321 and facts['rtp']['payload_type'] == 101
            assert facts['rtp']['start_observed'] is True
        else:
            assert facts['rtp'] is None
    assert profiles == {(a, k) for a in ['EdDSA', 'ES256'] for k in ['sip-info', 'rfc4733']}
    failures = []
    if args.failure_samples:
        raw = args.failure_samples.read_bytes()
        assert len(raw) < 1024 * 1024
        failures = [json.loads(line) for line in raw.splitlines() if line.strip()]
        assert len(failures) == 4
        assert {(r['algorithm'], r['profile']) for r in failures} == {(a, p) for a in ['EdDSA', 'ES256'] for p in ['fractional', 'missing-start']}
        for row in failures:
            record = row['operation']
            record_validator.validate(record)
            assert record['state'] == 'failed' and record.get('result') is None
            signal = record['problem']['signal']
            validator.validate(signal)
            assert signal['signal_source'] == 'rfc4733'
            facts = signal['source_observation']
            assert facts['duration_ms'] == (238 if row['profile'] == 'fractional' else 237)
            assert facts['rtp']['clock_rate_hz'] == 8000
            assert facts['rtp']['start_observed'] is (row['profile'] == 'fractional')
            assert facts['rtp']['duration_samples'] == (1897 if row['profile'] == 'fractional' else 1896)
            assert len(signal['recipients']) == 1
            assert signal['recipients'][0]['disposition'] == 'not_started'
            assert signal['recipients'][0]['frame_id'] is None
    print(json.dumps({'passed': True, 'installed_scenes': 2, 'received_signals': 4, 'http_summaries': 4, 'signed_native_frames': 4, 'unforwardable_failure_summaries': len(failures), 'scope': 'Local configured initial SIP INFO/RFC4733 to native output with source-owner HTTP status. SDK verifies signatures; validator checks schemas and captured identity. No remote playback, other legs or complete Gate6 qualification.'}))

if __name__ == '__main__':
    main()
