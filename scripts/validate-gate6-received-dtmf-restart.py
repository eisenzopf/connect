#!/usr/bin/env python3
"""Validate successor uncertainty for actual received SIP output without replay."""
import argparse
import json
from pathlib import Path
from jsonschema import Draft202012Validator
ROOT=Path(__file__).resolve().parents[1]

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('samples',type=Path)
    args=parser.parse_args();raw=args.samples.read_bytes();assert len(raw)<1024*1024
    rows=[json.loads(line) for line in raw.splitlines() if line.strip()]
    assert len(rows)==2 and {r['algorithm'] for r in rows}=={'EdDSA','ES256'}
    contract=json.loads((ROOT/'contracts/openapi-operations.json').read_text())
    result=Draft202012Validator({**contract,'$ref':'#/components/schemas/ReceivedDtmfSubmission'})
    source=json.loads((ROOT/'contracts/schemas/uctp-envelope-2.schema.json').read_text())
    status=Draft202012Validator({'$defs':source['$defs'],'$ref':'#/$defs/OperationUpdated'})
    wire=Draft202012Validator(source)
    for row in rows:
        before,after,frame=row['before'],row['recovered'],row['frame_before_restart']
        status.validate(before);status.validate(after);wire.validate(frame)
        assert before['state']=='preparing' and before.get('result') is None
        assert after['state']=='unknown' and after.get('result') is None
        assert after['record_id']==before['record_id']
        assert after['operation_id']==before['operation_id']==frame['operation_id']
        assert after['revision']>before['revision']
        summary=after['problem']['signal'];result.validate(summary)
        assert summary['signal_source']==frame['payload']['source']=='sip-info'
        assert summary['received_at']==frame['payload']['received_at']
        assert summary['source_observation']['digits']==frame['payload']['digits']=='5'
        assert len(summary['recipients'])==1
        assert summary['recipients'][0]['disposition']=='outcome_unknown'
        assert summary['recipients'][0]['frame_id']==frame['id']
        assert row['attempts']==row['calls']==1
        assert row['original_expiry']==row['final_expiry']
        assert row['new_runtime'] is row['new_observer_no_redelivery'] is True
        assert frame['signature']['alg']==row['algorithm']
    print(json.dumps({'passed':True,'algorithms':['EdDSA','ES256'],'before_restart_signed_frames':2,'http_summaries':4,'scope':'Actual received initial SIP INFO output with deliberately unrecorded outcome; successor retains original unknown reservation/expiry/one attempt, and a fresh native connection receives no redelivery. No remote playback, abrupt process crash or full recovery qualification.'}))

if __name__=='__main__':
    main()
