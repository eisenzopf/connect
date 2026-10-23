#!/usr/bin/env python3
"""Validate actual committed SIP input, signed native output and HTTP facts."""
import argparse
import collections
import datetime
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
    assert len(rows)==8
    contract=json.loads((ROOT/'contracts/openapi-operations.json').read_text())
    result_validator=Draft202012Validator({**contract,'$ref':'#/components/schemas/ReceivedDtmfSubmission'})
    wire_contract=json.loads((ROOT/'contracts/schemas/uctp-envelope-2.schema.json').read_text())
    record_validator=Draft202012Validator({'$defs':wire_contract['$defs'],'$ref':'#/$defs/OperationUpdated'})
    wire_validator=Draft202012Validator(wire_contract)
    profiles=collections.Counter()
    refused=set()
    frames,operations,observations=set(),set(),set()
    parents={}
    for row in rows:
        assert row['sip_calls']==1 and row['media_denied'] is True
        algorithm=row['algorithm']
        if row['phase']=='source-scope-refusal':
            assert row['source_rows']==0 and algorithm not in refused
            refused.add(algorithm)
            continue
        assert row['phase']=='delivered'
        record,frame,source=row['operation'],row['native_frame'],row['source']
        record_validator.validate(record)
        wire_validator.validate(frame)
        assert record['state']=='completed'
        result=record['result']
        result_validator.validate(result)
        facts,payload=result['source_observation'],frame['payload']
        profiles[(algorithm,facts['kind'])]+=1
        assert frame['signature']['alg']==algorithm
        assert frame['type']=='dtmf.received' and frame['connid']=='codec-source' and frame['sid']=='native-session'
        assert frame['operation_id']==record['operation_id'] and record['operation_id'].startswith('dtmfr_')
        assert result['connection_id']==source['connection_id']!=frame['connid']
        assert source['generation']==2 and source['sip_call_id']
        assert source['route_owner_id']!=source['identity_id']
        assert not {'endpoint_credential_id','endpoint_revision','signing_key_id','key_id'}.intersection(source)
        assert parents.setdefault(algorithm,row['source_parent'])==row['source_parent']
        assert result['signal_source']==payload['source']==facts['kind']
        assert result['source_observation_id']==facts['observation_id']
        assert result['received_at']==payload['received_at']==facts['received_at']
        assert datetime.datetime.fromisoformat(facts['received_at'].replace('Z','+00:00')).tzinfo
        assert payload['digits']==facts['digits']=='5' and payload['duration_ms']==facts['duration_ms']==237
        assert len(result['recipients'])==1
        recipient=result['recipients'][0]
        assert recipient['frame_id']==frame['id'] and recipient['connection_id']==frame['connid']
        assert recipient['disposition']=='local_submitted'
        assert frame['id'] not in frames and record['operation_id'] not in operations and facts['observation_id'] not in observations
        frames.add(frame['id']);operations.add(record['operation_id']);observations.add(facts['observation_id'])
        for field in ['stream_revision','envelope_id','signing_key_id','signing_algorithm']:
            assert field not in result and field not in facts
        if facts['kind']=='rfc4733':
            assert facts['rtp']['payload_type']==101 and facts['rtp']['clock_rate_hz']==8000
            assert facts['rtp']['duration_samples']==1896 and facts['rtp']['ssrc']==0x87654321 and facts['rtp']['start_observed'] is True
        else:
            assert facts['kind']=='sip-info' and facts['rtp'] is None
    assert refused=={'EdDSA','ES256'}
    assert profiles==collections.Counter({(algorithm,kind):count for algorithm in ['EdDSA','ES256'] for kind,count in [('sip-info',2),('rfc4733',1)]})
    print(json.dumps({'passed':True,'installed_moved_scenes':2,'source_permission_refusals':2,'received_signals':6,'http_summaries':6,'signed_native_frames':6,'scope':'Local native QUIC and actual moved loopback SIP. SDK verifies signatures; validator checks captured wire and HTTP schemas. No full task or gate qualification.'}))

if __name__=='__main__':main()
