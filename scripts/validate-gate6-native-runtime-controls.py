#!/usr/bin/env python3
"""Validate captured native runtime-control replies and historical gate states."""
import argparse
import json
from pathlib import Path

from jsonschema import Draft202012Validator

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('observations', type=Path)
parser.add_argument('--negotiation-samples', type=Path)
parser.add_argument('--codec-sample', type=Path)
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
wire = Draft202012Validator(json.loads(
    (root / 'contracts/schemas/uctp-envelope-2.schema.json').read_text()))
groups = {}
for line in args.observations.read_text().splitlines():
    row = json.loads(line)
    frame = row['frame']
    wire.validate(frame)
    assert frame['signature']['alg'] == row['algorithm']
    assert frame['type'] in ['operation.updated', 'ack']
    assert frame['sid'] == 'native-session' and frame['connid'] == 'codec-target'
    assert frame['in_reply_to']
    if frame['type'] == 'operation.updated':
        assert frame['operation_id'] == row['operation_id']
        groups.setdefault((row['algorithm'], frame['cid']), []).append(row)

algorithms = []
recovery = 0
compositions = 0
for (algorithm, cid), rows in groups.items():
    def outcomes(operation):
        return [r for r in rows if r['operation_id'] == operation]

    composition = bool(outcomes('gate-codec-unmute'))
    handles = set()
    for row in rows:
        payload = row['frame']['payload']
        if payload['state'] != 'completed':
            assert payload['state'] in ['preparing', 'failed']
            assert not payload.get('result')
            continue
        result = payload['result']
        assert result['action'] == row['input']['action']
        assert result['connection_id'] == 'codec-target'
        assert result['connection_generation'] == 1
        assert result['stream_revision'] == row['expected_revision'] + 1
        assert result['resources_changed'] is False
        stream, = result['gate_streams']
        binding = stream['binding']
        assert binding['stream_id'] == 'codec-monitor'
        assert binding['connection_generation'] == 1
        assert binding['direction'] == 'sendrecv'
        codec = binding['codec']['name']
        assert codec in (['pcmu', 'pcma'] if composition else ['pcmu'])
        assert binding['stream_generation'] == (2 if codec == 'pcma' else 1)
        assert binding['codec']['params']['sample_rate'] == 8000
        assert binding['codec']['params']['channels'] == 1
        handles.add(binding['stream_local_id'])
        action = row['input']['action']
        if action in ['mute', 'unmute']:
            assert stream['source_muted'] is (action == 'mute')
        elif action == 'resume':
            assert stream['state'] == 'active' and stream['held_direction'] is None
        else:
            direction = row['input'].get('details', {}).get('direction', 'recvonly')
            if direction == 'inactive':
                assert stream['state'] == 'paused'
            else:
                assert stream['state'] == 'active'
                assert stream['held_direction'] == {'sendonly': 'recvonly', 'recvonly': 'sendonly'}[direction]
    assert len(handles) == (2 if composition else 1)
    assert all(0 < handle < 65536 for handle in handles)
    if outcomes('gate-muted-paused'):
        algorithms.append(algorithm)
        replay = outcomes('gate-mute')
        assert len(replay) == 2
        assert replay[0]['frame']['payload'] == replay[1]['frame']['payload']
        assert replay[0]['frame']['id'] != replay[1]['frame']['id']
        resumed, = outcomes('gate-resume-still-muted')
        assert resumed['frame']['payload']['result']['gate_streams'][0]['source_muted'] is True
        for name in ['gate-stale', 'gate-unknown']:
            failed, = outcomes(name)
            assert failed['frame']['payload']['state'] == 'failed'
        print(f'{algorithm}: directional hold, pause, independent mute/resume, stable bindings and old-result replay verified')
    elif composition:
        replay = outcomes('gate-codec-mute')
        assert len(replay) == 2
        assert replay[0]['frame']['payload'] == replay[1]['frame']['payload']
        unmuted, = outcomes('gate-codec-unmute')
        assert unmuted['frame']['payload']['result']['gate_streams'][0]['binding']['codec']['name'] == 'pcma'
        compositions += 1
        print(f'{algorithm}: captured old mute remains historical after codec binding replacement and current unmute')
    else:
        recovered = outcomes('gate-recover')
        assert [r['frame']['payload']['state'] for r in recovered] == ['preparing', 'completed', 'completed']
        assert recovered[1]['frame']['payload'] == recovered[2]['frame']['payload']
        recovery += 1
        print(f'{algorithm}: original captured gate outcome recovered and replayed after a newer unmute')
assert sorted(algorithms) == ['ES256', 'EdDSA'] and recovery == 1
if args.codec_sample:
    assert compositions == 1
    sample = json.loads(args.codec_sample.read_text())
    wire.validate(sample['offer'])
    wire.validate(sample['codec_completion'])
    assert sample['offer']['signature']['alg'] == 'EdDSA'
    assert sample['offer']['type'] == 'connection.offer'
    assert sample['codec_completion']['payload']['state'] == 'completed'
    assert sample['codec_completion']['payload']['result']['stream_revision'] == 4
    assert sample['decoded_after_unmute'] is True
    old, = sample['mute']['result']['gate_streams']
    new, = sample['unmute']['result']['gate_streams']
    assert old['source_muted'] is True and new['source_muted'] is False
    assert old['binding']['stream_local_id'] != new['binding']['stream_local_id']
    assert new['binding']['codec']['name'] == 'pcma'

if args.negotiation_samples:
    paths = sorted(args.negotiation_samples.glob('*.json'))
    assert len(paths) == 2
    algorithms = []
    for path in paths:
        sample = json.loads(path.read_text())
        algorithm = sample['algorithm']
        algorithms.append(algorithm)
        assert sample['bidirectional_decoded_audio'] is True
        assert sample['current_session_audio'] == ['g.711-mu']
        for who in ['target', 'source']:
            offer, answer = sample[who + '_offer'], sample[who + '_answer']
            wire.validate(answer)
            assert answer['signature']['alg'] == algorithm
            offered, = offer['streams_offered']
            assert offered['codec_preferences'] == ['g.711-a', 'g.711-mu']
            accepted, = answer['payload']['streams_answered']
            assert accepted['id'] == offered['id']
            assert accepted['codec']['name'] == 'pcmu'
            assert accepted['direction'] == offered['direction'] == 'sendrecv'
            assert answer['payload']['by_participant'] == offer['by_participant']
            assert answer['payload']['substrate'] == offer['substrate'] == 'quic'
            cap, = answer['payload']['capabilities']['audio_codecs']
            assert cap['name'] == 'pcmu'
            assert answer['in_reply_to'] == answer['payload']['substrate_setup']['offer_id']
        refused = sample['refused']
        wire.validate(refused)
        assert refused['signature']['alg'] == algorithm
        assert refused['type'] == 'operation.updated'
        assert refused['payload']['state'] == 'failed'
        assert not refused['payload'].get('result')
        print(f'{algorithm}: offered codec fallback respects current answerer capability; incompatible-only offer refused')
    assert sorted(algorithms) == ['ES256', 'EdDSA']
