#!/usr/bin/env python3
"""Validate actual mixed SIP/native DTMF credential-epoch observations."""
import argparse
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
    assert len(rows)==2 and {r['algorithm'] for r in rows}=={'EdDSA','ES256'}
    contract=json.loads((ROOT/'contracts/openapi-operations.json').read_text())
    received=Draft202012Validator({**contract,'$ref':'#/components/schemas/ReceivedDtmfSubmission'})
    native=Draft202012Validator({**contract,'$ref':'#/components/schemas/NativeDtmfSubmission'})
    schema=json.loads((ROOT/'contracts/schemas/uctp-envelope-2.schema.json').read_text())
    record=Draft202012Validator({'$defs':schema['$defs'],'$ref':'#/$defs/OperationUpdated'})
    wire=Draft202012Validator(schema)
    frames,operations=set(),set()
    for row in rows:
        assert row['before']==row['after'] and row['sip_calls']==1
        assert 0<=row['input_age_at_refresh_ms']<800
        old=row['queued'];record.validate(old)
        assert old['state']=='failed'
        summary=old['problem']['signal'];received.validate(summary)
        assert summary['signal_source']=='sip-info'
        assert len(summary['recipients'])==1
        assert summary['recipients'][0]['disposition']=='not_started' and summary['recipients'][0]['frame_id'] is None
        assert row['queued_target']['credential_epoch']==0
        assert row['queued_target']['credential_id']!=row['current_credential']
        assert len(row['received'])==2
        kinds=set()
        registrations=set()
        for item in row['received']:
            done,frame,target=item['operation'],item['native_frame'],item['target']
            record.validate(done);wire.validate(frame)
            assert done['state']=='completed'
            result=done['result'];received.validate(result)
            facts=result['source_observation'];kinds.add(facts['kind'])
            assert frame['type']=='dtmf.received' and frame['signature']['alg']==row['algorithm']
            assert frame['operation_id']==done['operation_id']!=old['operation_id']
            assert frame['payload']['source']==result['signal_source']==facts['kind']
            assert frame['payload']['digits']==facts['digits']=='5' and frame['payload']['duration_ms']==facts['duration_ms']==237
            assert frame['payload']['received_at']==result['received_at']==facts['received_at']
            assert datetime.datetime.fromisoformat(facts['received_at'].replace('Z','+00:00')).tzinfo
            assert target['credential_epoch']==1 and target['credential_id']==row['current_credential']
            assert target['connection_id']==frame['connid']==row['before']['connection_id']
            assert target['generation']==row['before']['generation'] and target['participant_id']==row['before']['participant_id']
            assert item['registration']!=row['queued_registration'];registrations.add(item['registration'])
            assert result['recipients'][0]['frame_id']==frame['id'] and result['recipients'][0]['disposition']=='local_submitted'
            assert frame['id'] not in frames and done['operation_id'] not in operations
            frames.add(frame['id']);operations.add(done['operation_id'])
            if facts['kind']=='rfc4733':
                assert facts['rtp']['payload_type']==101 and facts['rtp']['clock_rate_hz']==8000 and facts['rtp']['duration_samples']==1896
            else:assert facts['rtp'] is None
        assert kinds=={'sip-info','rfc4733'} and len(registrations)==1
        outgoing=row['outgoing'];record.validate(outgoing)
        assert outgoing['state']=='completed';native.validate(outgoing['result'])
        source=row['outgoing_source']
        assert source['credential_revision']==2 and source['signing_algorithm']==row['algorithm']
        assert source['stream_revision']==row['before']['stream_revision']==outgoing['result']['stream_revision']
        assert source['envelope_id']==outgoing['result']['source_observation_id']
        assert outgoing['result']['signal_source']=='uctp-native' and outgoing['result']['recipients'][0]['disposition']=='local_submitted'
    print(json.dumps({'passed':True,'signing_scenes':2,'queued_epoch_refusals':2,'fresh_signed_received_frames':4,'received_http_summaries':6,'native_outgoing_http_summaries':2,'connection_stream_lineage_preserved':True,'scope':'Local actual moved SIP/native refresh; SDK verifies signatures and validator checks captured wire/status facts. No whole-task or gate qualification.'}))

if __name__=='__main__':main()
