#!/usr/bin/env python3
"""Validate actual signed native offers, current selection and authority refusals."""
import argparse
import json
from pathlib import Path
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('observations', type=Path)
    parser.add_argument('negotiation_samples', type=Path)
    args = parser.parse_args()
    raw = args.observations.read_bytes()
    assert len(raw) < 1024 * 1024
    rows = [json.loads(line) for line in raw.splitlines() if line.strip()]
    assert len(rows) == 4
    contract = json.loads((ROOT / 'contracts/schemas/uctp-envelope-2.schema.json').read_text())
    wire = Draft202012Validator(contract)
    offer = Draft202012Validator({'$defs': contract['$defs'], '$ref': '#/$defs/ConnectionOffer'})
    def validate(validator, value, label):
        error = next(validator.iter_errors(value), None)
        assert error is None, f"{label}: {list(error.absolute_path)}: {error.message}"

    frames = set()
    for algorithm in ['EdDSA', 'ES256']:
        selected = [r for r in rows if r['callback_offer']['signature']['alg'] == algorithm]
        assert len(selected) == 2
        samples = json.loads((args.negotiation_samples / f'{algorithm}.json').read_text())
        assert samples['algorithm'] == algorithm and samples['bidirectional_decoded_audio']
        assert samples['current_session_audio'] == ['g.711-mu']
        assert {r['answer']['connid'] for r in selected} == {'fallback-target', 'fallback-source'}
        for row in selected:
            request, answer, callback = row['request_payload'], row['answer'], row['callback_offer']
            validate(offer, request, "request payload")
            for frame in [answer, callback]:
                validate(wire, frame, frame.get("type", "frame"))
                assert frame['signature']['alg'] == algorithm and frame['id'] not in frames
                frames.add(frame['id'])
                assert frame['sid'] == 'native-session'
            assert answer['type'] == 'connection.answer' and callback['type'] == 'connection.offer'
            assert answer['in_reply_to'] == row['request_id']
            assert callback['operation_id'] == row['operation_id']
            assert (answer['cid'], answer['sid'], answer['connid']) == (callback['cid'], callback['sid'], callback['connid'])
            a, c = answer['payload'], callback['payload']
            assert a['by_participant'] == c['by_participant'] == request['by_participant']
            assert a['substrate'] == c['substrate'] == request['substrate'] == 'quic'
            original = request['streams_offered'][0]
            offered = c['streams_offered'][0]
            prepared = c['substrate_setup']['streams'][0]
            assert original['id'] == offered['id'] == prepared['strm_id']
            assert original['kind'] == offered['kind'] == prepared['kind'] == 'audio'
            assert original['direction'] == offered['direction'] == prepared['direction'] == 'sendrecv'
            assert original['codec_preferences'] == ['g.711-a', 'g.711-mu']
            assert offered['codec_preferences'] == ['pcmu']
            assert prepared['codec']['name'] == 'pcmu'
            assert prepared['codec']['params']['sample_rate'] == 8000
            assert prepared['codec']['params']['channels'] == 1
            assert 1 <= prepared['stream_local_id'] <= 65535
            assert c['substrate_setup']['kind'] == 'native-prepared/1'
            assert c['substrate_setup']['revision'] == 1
            assert c['substrate_setup']['offer_id']
            assert [x['name'] for x in c['capabilities']['audio_codecs']] == ['pcmu']
            assert a['streams_answered'][0]['codec']['name'] == 'pcmu'
        validate(wire, samples['refused'], 'codec refusal')
        assert samples['refused']['payload']['state'] == 'failed'
        refusals = samples['authority_refusals']
        assert {r['case'] for r in refusals} == {'foreign-participant', 'foreign-substrate'}
        for item in refusals:
            frame = item['refusal']
            validate(wire, frame, item["case"])
            assert frame['signature']['alg'] == algorithm
            assert frame['in_reply_to'] == item['request_id'] and item['effects'] == 0
            assert frame['type'] == 'error'
            assert frame['payload']['code'] == (403 if item['case'] == 'foreign-participant' else 501)
            assert frame['payload']['category'] == ('auth' if item['case'] == 'foreign-participant' else 'unsupported')
            if item['case'] == 'foreign-participant':
                assert item['request_payload']['by_participant'] == samples['source_offer']['by_participant']
                assert item['request_payload']['by_participant'] != samples['target_offer']['by_participant']
            else:
                assert item['request_payload']['substrate'] == 'webtransport'
    print(json.dumps({'passed': True, 'signing_scenes': 2, 'actual_signed_callback_offers': 4,
                      'actual_signed_answers': 4, 'authority_refusals': 4,
                      'scope': 'Local Connect native QUIC/public SDK actual offer fields, signed prepared setup, current capability selection, physical/participant authority refusals without effects and independent decoded duplex recovery. No other positive substrate or complete task/acceptance claim.'}))


if __name__ == '__main__':
    main()
