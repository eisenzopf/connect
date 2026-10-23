#!/usr/bin/env python3
"""Check clause0115 against actual native, SIP INFO and RTP-event captures."""
import argparse
import json
from pathlib import Path
from jsonschema import Draft202012Validator
ROOT=Path(__file__).resolve().parents[1]

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('native',type=Path);parser.add_argument('sip',type=Path)
    args=parser.parse_args()
    assert args.native.stat().st_size<1024*1024 and args.sip.stat().st_size<1024*1024
    native=json.loads(args.native.read_text());sip=[json.loads(line) for line in args.sip.read_text().splitlines() if line.strip()]
    assert len(native)==2 and len(sip)==4
    contract=json.loads((ROOT/'contracts/openapi-operations.json').read_text())
    validators={kind:Draft202012Validator({**contract,'$ref':'#/components/schemas/'+schema}) for kind,schema in [('uctp-native','NativeDtmfSubmission'),('sip','ReceivedDtmfSubmission')]}
    source=json.loads((ROOT/'contracts/schemas/uctp-envelope-2.schema.json').read_text());wire=Draft202012Validator(source)
    profiles=set();ids=set()
    for row,frame,record,expected in [(r,r['actual_received'],r['http_operation'],'uctp-native') for r in native]+[(r,r['native_frame'],r['operation'],r['operation']['result']['signal_source']) for r in sip]:
        wire.validate(frame);summary=record['result'];validators['uctp-native' if expected=='uctp-native' else 'sip'].validate(summary)
        assert record['state']=='completed' and frame['type']=='dtmf.received'
        assert frame['payload']['source']==summary['signal_source']==expected
        assert frame['operation_id']==record['operation_id']
        assert frame['signature']['alg']==row['algorithm']
        assert frame['payload']['received_at']==summary['received_at']
        assert summary['recipients'][0]['frame_id']==frame['id']
        assert frame['id'] not in ids;ids.add(frame['id'])
        profiles.add((row['algorithm'],expected))
        if expected=='uctp-native':
            # The producing SDK test requested method=info. That destination
            # method must not relabel this actual native source as SIP INFO.
            assert frame['payload']['digits']=='8#' and frame['payload']['duration_ms']==100
            assert 'source_observation' not in summary
        else:
            assert expected in ['sip-info','rfc4733']
            assert summary['source_observation']['kind']==expected
            assert frame['payload']['digits']=='5' and frame['payload']['duration_ms']==237
            for key in ['stream_revision','envelope_id','signing_key_id','signing_algorithm']:
                assert key not in summary and key not in summary['source_observation']
    assert profiles=={(a,s) for a in ['EdDSA','ES256'] for s in ['uctp-native','sip-info','rfc4733']}
    print(json.dumps({'passed':True,'case_id':'u6-req-0115','signed_native_frames':6,'http_summaries':6,'source_kinds':['uctp-native','sip-info','rfc4733'],'algorithms':['EdDSA','ES256'],'scope':'Local source-field attribution: actual native INFO-method requests remain uctp-native; actual initial SIP INFO and RFC4733 inputs retain their physical source kinds. SDK verifies signatures. This does not qualify all gateway legs, WebRTC, fanout or full DTMF delivery.'}))

if __name__=='__main__':
    main()
