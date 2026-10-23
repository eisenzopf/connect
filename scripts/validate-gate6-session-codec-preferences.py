#!/usr/bin/env python3
"""Validate actual signed initial offers under current Session codec policy.

The production test separately verifies signatures, immutable original offers,
current policy/replay, explicit live-update refusal and FFmpeg-decoded audio.
"""
import argparse
import json
from pathlib import Path
from jsonschema import Draft202012Validator, FormatChecker

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('observations', type=Path)
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
wire = Draft202012Validator(json.loads((root / 'contracts/schemas/uctp-envelope-2.schema.json').read_text()), format_checker=FormatChecker())
seen = set()
for line in args.observations.read_text().splitlines():
    row = json.loads(line)
    algorithm, mode = row['algorithm'], row['mode']
    assert (algorithm, mode) not in seen
    seen.add((algorithm, mode))
    latest = mode == 'latest'
    target_codec = 'pcmu' if latest else 'pcma'
    source_codec = 'pcma' if mode == 'preferred' else 'pcmu'
    assert row['current_preferences'] == ['g.711-mu' if latest else 'g.711-a']
    assert row['offered_preferences'] == (['g.711-a', 'g.711-mu'] if latest else ['g.711-mu', 'g.711-a'])
    for role, selected in [('target', target_codec), ('source', source_codec)]:
        frame = row[role]
        wire.validate(frame)
        assert frame['type'] == 'connection.answer' and frame['signature']['alg'] == algorithm
        assert frame['sid'] == 'native-session' and frame['connid'] == 'policy-' + role
        assert frame['payload']['streams_answered'][0]['codec']['name'] == selected
        assert frame['payload']['substrate_setup']['streams'][0]['codec']['name'] == selected
        codecs = {c['name'] for c in frame['payload']['capabilities']['audio_codecs']}
        assert codecs == ({'pcmu'} if role == 'source' and mode == 'accepted-fallback' else {'pcmu', 'pcma'})
    print(f'{algorithm}/{mode}: current preference, offered formats and accepted-capability fallback verified')
assert seen == {(alg, mode) for alg in ['EdDSA', 'ES256'] for mode in ['preferred', 'latest', 'accepted-fallback']}
