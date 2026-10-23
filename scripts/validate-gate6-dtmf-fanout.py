#!/usr/bin/env python3
"""Validate captured public SDK fanout results against the existing HTTP schema."""
import argparse
import json
import pathlib
from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('samples', type=pathlib.Path)
    args = parser.parse_args()
    raw = args.samples.read_bytes()
    assert len(raw) <= 1024 * 1024
    rows = [json.loads(line) for line in raw.splitlines() if line.strip()]
    assert len(rows) == 4
    assert {(r['algorithm'], r['partial']) for r in rows} == {
        (algorithm, partial) for algorithm in ('EdDSA', 'ES256') for partial in (False, True)
    }
    contract = json.loads((ROOT / 'contracts/openapi-operations.json').read_text())
    validator = Draft202012Validator(contract['components']['schemas']['NativeDtmfSubmission'])
    schedules, sources = set(), set()
    for row in rows:
        record = row['completed']
        assert row['calls'] == [1, 1]
        assert record['operation_id'] == ('fanout-partial' if row['partial'] else 'fanout-long')
        assert record['state'] == ('unknown' if row['partial'] else 'completed')
        result = record['problem']['signal'] if row['partial'] else record['result']
        validator.validate(result)
        assert result['signal_source'] == 'uctp-native'
        assert result['connection_id'] == 'initial-native'
        assert result['local_work_complete'] and result['local_submission_only']
        assert result['resources_changed'] is False
        assert result['source_observation_id'] not in sources
        sources.add(result['source_observation_id'])
        assert len(result['recipients']) == 2
        assert len({r['connection_id'] for r in result['recipients']}) == 2
        assert sorted(r['disposition'] for r in result['recipients']) == (
            ['local_submitted', 'outcome_unknown'] if row['partial'] else ['local_submitted'] * 2
        )
        for target in result['recipients']:
            assert target['frame_id'].startswith('dtmfg_')
            assert target['frame_id'] not in schedules
            schedules.add(target['frame_id'])
        if row['partial']:
            assert row['method'] == 'info' and row['duration_ms'] == 1000
            assert row['info_packets'] == [1, 2] and row['events'] is None
            assert 1070 <= row['elapsed_ms'] < 18000
        else:
            assert row['method'] == 'rfc4733' and row['duration_ms'] == 6000
            assert row['info_packets'] == [0, 0]
            assert 12070 <= row['elapsed_ms'] < 18000
            first, second = row['events']
            assert first[1] == second[1] == 2
            assert abs(first[0] - second[0]) < 1000
    print(json.dumps({
        'passed': True, 'actual_sdk_summaries': len(rows), 'distinct_schedules': len(schedules),
        'scope': 'Two actual initial SIP recipients; overlapping long RFC4733 schedules and partial INFO refusal for EdDSA/ES256.',
        'signature_verification': 'Producing SDK verifies signatures; this validator checks captured DTOs and HTTP/OpenAPI schema.',
        'full_dtmf_clause_qualification': False,
    }))


if __name__ == '__main__':
    main()
