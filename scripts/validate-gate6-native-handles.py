#!/usr/bin/env python3
"""Validate actual signed events and decoded peer-local native RTP observations."""
import argparse
import json
from pathlib import Path
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('samples', type=Path)
    parser.add_argument('--replacement-samples', type=Path)
    args = parser.parse_args()
    raw = args.samples.read_bytes()
    assert len(raw) < 1024 * 1024
    rows = [json.loads(line) for line in raw.splitlines() if line.strip()]
    assert len(rows) == 2 and {r['algorithm'] for r in rows} == {'EdDSA', 'ES256'}
    wire = Draft202012Validator(json.loads((ROOT / 'contracts/schemas/uctp-envelope-2.schema.json').read_text()))
    events = set()
    answers = 0
    for row in rows:
        bindings = {}
        target_handles = set()
        for frame in row['target_bindings'] + [row['source_binding']]:
            wire.validate(frame)
            answers += 1
            assert frame['type'] == 'connection.answer'
            assert frame['signature']['alg'] == row['algorithm']
            assert frame['cid'] == row['conversation_id'] and frame['sid'] == 'native-session'
            stream = frame['payload']['substrate_setup']['streams'][0]
            handle = stream['stream_local_id']
            assert isinstance(handle, int) and 1 <= handle <= 65535
            bindings[frame['connid']] = (stream['strm_id'], handle)
            if frame['connid'] in {'handles-a', 'handles-b'}:
                target_handles.add(handle)
        assert len(target_handles) == 2
        assert set(bindings) == {'handles-a', 'handles-b', 'handles-source'}
        source_handle = bindings['handles-source'][1]
        # These captured cases deliberately exercise numeric overlap across peers.
        assert source_handle in target_handles
        assert row['foreign_handle'] in target_handles and row['foreign_handle'] != source_handle
        assert row['foreign_input_produced_no_output_or_lifecycle']
        assert {r['stream_local_id'] for r in row['forward']} == target_handles
        assert {r['stream_local_id'] for r in row['reverse']} == {source_handle}
        for observed in row['forward'] + row['reverse']:
            assert observed['marked_frames'] > 10 and observed['decoded_loud_samples'] > 500
        assert len(row['target_opened']) == 2 and len(row['source_opened']) == 1
        opened = set()
        for frame in row['target_opened'] + row['source_opened']:
            wire.validate(frame)
            assert frame['type'] == 'stream.opened' and frame['signature']['alg'] == row['algorithm']
            assert frame['cid'] == row['conversation_id'] and frame['sid'] == 'native-session'
            assert frame['id'] not in events
            events.add(frame['id'])
            assert frame['connid'] not in opened
            opened.add(frame['connid'])
            stream = frame['payload']['stream']
            assert (stream['strm_id'], stream['stream_local_id']) == bindings[frame['connid']]
            assert stream['kind'] == 'audio' and stream['direction'] == 'sendrecv'
            assert stream['codec']['name'] == 'pcmu'
            assert stream['codec']['params']['sample_rate'] == 8000
            assert stream['codec']['params']['channels'] == 1
        assert opened == set(bindings)
    replacement_scenes = 0
    replacement_events = 0
    if args.replacement_samples:
        replacement_raw = args.replacement_samples.read_bytes()
        assert len(replacement_raw) < 1024 * 1024
        replacements = [json.loads(line) for line in replacement_raw.splitlines() if line.strip()]
        for algorithm in ['EdDSA', 'ES256']:
            frames = [row['frame'] for row in replacements if row['algorithm'] == algorithm]
            assert frames
            for frame in frames:
                wire.validate(frame)
                assert frame['signature']['alg'] == algorithm and frame['connid'] == 'codec-target'
                assert frame['id'] not in events
                events.add(frame['id'])
            opened_frames = [f for f in frames if f['type'] == 'stream.opened']
            closed_frames = [f for f in frames if f['type'] == 'stream.closed']
            assert len(opened_frames) == 2 and len(closed_frames) == 1
            old, new = [f['payload']['stream'] for f in opened_frames]
            assert old['strm_id'] == new['strm_id'] == closed_frames[0]['payload']['strm_id'] == 'codec-monitor'
            assert old['codec']['name'] == 'pcmu' and new['codec']['name'] == 'pcma'
            assert old['stream_local_id'] != new['stream_local_id']
            assert 1 <= new['stream_local_id'] <= 65535
            assert frames.index(opened_frames[0]) < frames.index(closed_frames[0]) < frames.index(opened_frames[1])
            assert closed_frames[0]['payload']['reason'] == 'codec-renegotiated'
            replacement_scenes += 1
            replacement_events += len(frames)
    print(json.dumps({'passed': True, 'installed_scenes': 2, 'signed_answers': answers,
                      'signed_stream_opened': 6, 'replacement_scenes': replacement_scenes,
                      'replacement_signed_events': replacement_events,
                      'scope': 'Actual local native QUIC physical-peer handle namespace, two Connections on one peer, rejected foreign input before flow and independently decoded bidirectional recovery. No other substrate, capacity, complete task or Gate6 acceptance claim.'}))


if __name__ == '__main__':
    main()
