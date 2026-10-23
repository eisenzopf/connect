#!/usr/bin/env python3
"""Check actual received-signal authority facts; fenced scope edits are fault setup."""
import argparse
import json
from pathlib import Path
from jsonschema import Draft202012Validator
ROOT=Path(__file__).resolve().parents[1]

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('samples',type=Path)
    args=parser.parse_args()
    raw=args.samples.read_bytes()
    assert len(raw)<1024*1024
    rows=[json.loads(line) for line in raw.splitlines() if line.strip()]
    assert len(rows)==2 and {r['algorithm'] for r in rows}=={'EdDSA','ES256'}
    contract=json.loads((ROOT/'contracts/openapi-operations.json').read_text())
    result=Draft202012Validator({**contract,'$ref':'#/components/schemas/ReceivedDtmfSubmission'})
    wire_contract=json.loads((ROOT/'contracts/schemas/uctp-envelope-2.schema.json').read_text())
    wire=Draft202012Validator(wire_contract)
    status=Draft202012Validator({'$defs':wire_contract['$defs'],'$ref':'#/$defs/OperationUpdated'})
    frame_ids=set()
    for row in rows:
        assert row['calls']==1 and row['operations']==7
        assert row['source_missing_send_accepted']==0
        assert row['marked_from_phone']==row['marked_to_phone']==0
        samples=row['samples']
        assert len(samples)==5
        assert sorted(x['phase'] for x in samples)==['both-signal-only','recipient-denied','recipient-recovered','source-signal-only','source-signal-only']
        assert {x['operation']['result']['signal_source'] for x in samples if x['phase']=='source-signal-only'}=={'sip-info','rfc4733'}
        for sample in samples:
            record=sample['operation']
            status.validate(record)
            signal=record['problem']['signal'] if sample['phase']=='recipient-denied' else record['result']
            result.validate(signal)
            if sample['phase']=='recipient-denied':
                assert record['state']=='failed' and signal['recipients']==[]
                assert signal['source_observation']['digits']=='8'
                assert 'frame' not in sample
            else:
                assert record['state']=='completed'
                frame=sample['frame'];wire.validate(frame)
                assert frame['type']=='dtmf.received' and frame['signature']['alg']==row['algorithm']
                assert frame['operation_id']==record['operation_id']
                assert frame['payload']['digits']=='5' and frame['payload']['duration_ms']==237
                assert frame['payload']['source']==signal['signal_source']
                assert frame['payload']['received_at']==signal['received_at']
                assert len(signal['recipients'])==1
                assert signal['recipients'][0]['frame_id']==frame['id']
                assert signal['recipients'][0]['disposition']=='local_submitted'
                assert frame['id'] not in frame_ids;frame_ids.add(frame['id'])
    print(json.dumps({'passed':True,'algorithms':['EdDSA','ES256'],'installed_scenes':2,'captured_signed_frames':8,'captured_http_summaries':10,'calls':2,'scope':'Actual initial SIP signal-only INFO/RFC4733, missing source-send and recipient-receive refusal, same-call recovery, and no marked audio after media withdrawal. Fenced credential edits are controlled fault setup, not a public credential-edit API pass. Native metadata protocol floors remain granted.'}))

if __name__=='__main__':
    main()
