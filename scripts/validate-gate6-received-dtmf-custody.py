#!/usr/bin/env python3
"""Check captured actual SIP receive facts; these are not public wire DTOs."""
import argparse
import datetime
import json
import pathlib
import re


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('samples', type=pathlib.Path)
    args = parser.parse_args()
    raw = args.samples.read_bytes()
    assert len(raw) < 1024 * 1024
    rows = [json.loads(line) for line in raw.splitlines() if line.strip()]
    assert len(rows) == 2
    assert {row['algorithm'] for row in rows} == {'EdDSA', 'ES256'}
    for row in rows:
        assert row['source'] == 'actual-initial-sip' and row['calls'] == 1
        assert len(row['observations']) == 2
        assert {event['kind'] for event in row['observations']} == {'sip-info', 'rfc4733'}
        assert len({event['observation_id'] for event in row['observations']}) == 2
        for event in row['observations']:
            assert set(event) == {'kind', 'observation_id', 'received_at', 'duration_ms', 'sealed_audience', 'rtp'}
            assert event['duration_ms'] == 237 and event['sealed_audience'] is True
            assert event['observation_id']
            assert datetime.datetime.fromisoformat(event['received_at'].replace('Z', '+00:00')).tzinfo
            if event['kind'] == 'sip-info':
                assert re.fullmatch('[0-9a-f]{64}', event['observation_id'])
                assert event['rtp'] is None
            else:
                assert event['rtp'] == {
                    'ssrc': 0x87654321, 'payload_type': 101, 'clock_rate_hz': 8000,
                    'duration_samples': 1896, 'start_observed': True,
                }
                assert event['rtp']['duration_samples'] * 1000 == event['duration_ms'] * event['rtp']['clock_rate_hz']
    print(json.dumps({
        'passed': True, 'actual_initial_sip_scenes': 2, 'actual_received_observations': 4,
        'algorithms': ['EdDSA', 'ES256'], 'kinds': ['sip-info', 'rfc4733'],
        'scope': 'Actual configured Connect initial SIP calls feeding the managed Core source queue through a test-support accessor. Facts capture only; no public received DTO, durable forwarding, native output or new clause/task qualification.',
    }))


if __name__ == '__main__':
    main()
