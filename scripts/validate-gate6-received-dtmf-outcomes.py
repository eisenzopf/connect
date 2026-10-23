#!/usr/bin/env python3
"""Check actual captured output and durable recovery after source revocation."""
import argparse
import json
from pathlib import Path
from jsonschema import Draft202012Validator
ROOT=Path(__file__).resolve().parents[1]

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('samples',type=Path)
    args=parser.parse_args()
    raw=args.samples.read_bytes();assert len(raw)<1024*1024
    rows=[json.loads(line) for line in raw.splitlines() if line.strip()]
    assert len(rows)==2 and {r['algorithm'] for r in rows}=={'EdDSA','ES256'}
    contract=json.loads((ROOT/'contracts/openapi-operations.json').read_text())
    result=Draft202012Validator({**contract,'$ref':'#/components/schemas/ReceivedDtmfSubmission'})
    source=json.loads((ROOT/'contracts/schemas/uctp-envelope-2.schema.json').read_text())
    status=Draft202012Validator({'$defs':source['$defs'],'$ref':'#/$defs/OperationUpdated'})
    wire=Draft202012Validator(source)
    for row in rows:
        before,after,frame=row['preparing'],row['completed'],row['frame']
        status.validate(before);status.validate(after);wire.validate(frame)
        assert before['state']=='preparing' and before.get('result') is None
        assert after['state']=='completed' and after['operation_id']==before['operation_id']==frame['operation_id']
        assert after['record_id']==before['record_id'] and after['revision']>before['revision']
        summary=after['result'];result.validate(summary)
        assert summary['signal_source']==frame['payload']['source']=='sip-info'
        assert summary['received_at']==frame['payload']['received_at']
        assert summary['source_observation']['digits']==frame['payload']['digits']=='5'
        assert summary['source_observation']['duration_ms']==frame['payload']['duration_ms']==237
        assert len(summary['recipients'])==1
        assert summary['recipients'][0]['disposition']=='local_submitted'
        assert summary['recipients'][0]['frame_id']==frame['id']
        assert frame['signature']['alg']==row['algorithm']
        assert row['attempts']==row['calls']==1 and row['source_retired'] is True
        assert row['original_expiry']==row['final_expiry'] and 0<=row['revocation_ms']<3000
    print(json.dumps({'passed':True,'algorithms':['EdDSA','ES256'],'captured_signed_frames':2,'http_summaries':4,'scope':'Actual initial SIP INFO to native output, forced outcome-storage failure, public Store source revocation and physical call retirement, captured settlement under a fresh same-owner HTTP credential, unchanged expiry/one attempt and no replay. No process-restart or broad recovery qualification.'}))

if __name__=='__main__':
    main()
