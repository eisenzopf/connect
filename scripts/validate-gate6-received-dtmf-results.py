#!/usr/bin/env python3
"""Check controlled Store received-SIP status samples, not physical delivery."""
import argparse
import json
from pathlib import Path
from jsonschema import Draft202012Validator
ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('samples', type=Path)
    args = parser.parse_args()
    raw = args.samples.read_bytes()
    assert len(raw) < 1024 * 1024
    rows = [json.loads(line) for line in raw.splitlines() if line.strip()]
    assert len(rows) == 5
    contract = json.loads((ROOT/'contracts/openapi-operations.json').read_text())
    validator = Draft202012Validator({**contract, '$ref': '#/components/schemas/ReceivedDtmfSubmission'})
    record_validator = Draft202012Validator({**contract, '$ref': '#/components/schemas/OperationRecord'})
    states = set()
    for row in rows:
        record_validator.validate(row)
        states.add(row['state'])
        signal = row['result'] if row['state'] == 'completed' else row['problem']['signal']
        validator.validate(signal)
        assert signal['signal_source'] == signal['source_observation']['kind'] == 'sip-info'
        assert signal['source_observation_id'] == signal['source_observation']['observation_id']
        assert signal['received_at'] == signal['source_observation']['received_at']
        assert signal['source_observation']['digits'] == '5'
        assert signal['source_observation']['duration_ms'] == 237
        for key in ['stream_revision', 'envelope_id', 'signing_key_id', 'signing_algorithm']:
            assert key not in signal and key not in signal['source_observation']
    assert states == {'completed', 'failed', 'cancelled', 'unknown'}
    print(json.dumps({'passed': True, 'controlled_store_results': len(rows), 'states': sorted(states), 'scope': 'Controlled Store status against HTTP schema. No physical receive, signature or delivery qualification.'}))

if __name__ == '__main__':
    main()
