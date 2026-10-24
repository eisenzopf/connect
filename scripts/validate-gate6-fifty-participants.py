#!/usr/bin/env python3
"""Validate actual fifty-peer native Session captures; SDK verifies signatures."""
import argparse
import json
from pathlib import Path
from jsonschema import Draft202012Validator
ROOT=Path(__file__).resolve().parents[1]

def validate_startup(phase, allowed, connections):
    assert phase['source_index']==0 and set(phase['allowed'])==allowed
    assert phase['deadline_ms']==8000 and 300<=phase['elapsed_ms']<8000
    assert set(phase['flow_connection_ids'])==connections
    assert phase['flow_facts']==len(connections)==len(phase['flow_connection_ids'])
    assert len(phase['decoded_marked'])==50
    assert all(count>0 if i in allowed else count==0 for i,count in enumerate(phase['decoded_marked']))

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('samples',type=Path)
    args=parser.parse_args()
    raw=args.samples.read_bytes()
    assert len(raw)<16*1024*1024
    rows=[json.loads(line) for line in raw.splitlines() if line.strip()]
    assert len(rows)==2 and {r['algorithm'] for r in rows}=={'EdDSA','ES256'}
    wire=Draft202012Validator(json.loads((ROOT/'contracts/schemas/uctp-envelope-2.schema.json').read_text()))
    total_joined=total_started=total_left=0
    for row in rows:
        pids=row['participants'];assert len(pids)==len(set(pids))==50
        assert row['configured_participants']==50 and row['remaining_connections']==95
        handles=row['peer_handles'];assert len(handles)==50 and all(0<h<=65535 for h in handles)
        assert handles[0]!=handles[1]
        connections={f'large-publisher-{i}' for i in range(50)}
        publisher_handles=[a['payload']['substrate_setup']['streams'][0]['stream_local_id'] for a in row['answers']]
        assert handles[0]==publisher_handles[0] and all(handles[i]!=publisher_handles[i] for i in range(1,50))
        assert len(row['answers'])==50
        for i,answer in enumerate(row['answers']):
            wire.validate(answer)
            assert answer['type']=='connection.answer' and answer['signature']['alg']==row['algorithm']
            assert answer['connid']==f'large-publisher-{i}' and answer['sid']=='native-session'
            assert answer['payload']['by_participant']==pids[i]
            stream=answer['payload']['substrate_setup']['streams'][0]
            assert stream['stream_local_id']==publisher_handles[i] and stream['strm_id']==f'large-voice-{i}'
        assert len(row['initial_events'])==50
        self_joined_seen=[False]*50
        conversations=set()
        for i,peer in enumerate(row['initial_events']):
            assert peer['peer_index']==i and peer['participant_id']==pids[i]
            expected=set(pids)-{pids[i]}
            assert set(peer['joined'])==expected
            joined=set();started=ready=opened=False;self_joined=False
            for frame in peer['frames']:
                wire.validate(frame);conversations.add(frame['cid'])
                assert frame['signature']['alg']==row['algorithm'] and frame['sid']=='native-session'
                kind=frame['type'];body=frame['payload']
                if kind=='session.started':
                    assert not started;started=True;total_started+=1
                    assert set(body['participants_present'])==set(pids)
                    assert {c['connid'] for c in body['active_connections']}==connections
                elif kind=='session.participant.joined':
                    pid=body['participant']['participant_id']
                    assert body['via_connection']==f'large-publisher-{pids.index(pid)}'
                    if pid==pids[i]:
                        assert not self_joined;self_joined=True;continue
                    assert pid in expected and pid not in joined
                    joined.add(pid);total_joined+=1
                elif kind=='connection.ready':
                    assert not ready and frame['connid']==f'large-publisher-{i}';ready=True
                elif kind=='stream.opened':
                    assert ready and not opened and body['stream']['stream_local_id']==publisher_handles[i];opened=True
                else:assert kind in {'connection.quality','recording.vcon-ready'}
            assert started and ready and opened and joined==expected
            self_joined_seen[i]=self_joined
        assert len(conversations)==1
        assert len(row['receiver_answers'])==len(row['receiver_events'])==49
        validate_startup(row['receiver_startup'], set(range(1,50)), {f'large-receiver-{i}' for i in range(1,50)})
        for i,(answer,event) in enumerate(zip(row['receiver_answers'],row['receiver_events']),1):
            wire.validate(answer)
            assert answer['signature']['alg']==row['algorithm'] and answer['connid']==f'large-receiver-{i}'
            assert answer['payload']['by_participant']==pids[i]
            stream=answer['payload']['substrate_setup']['streams'][0]
            assert stream['stream_local_id']==handles[i] and stream['strm_id']==f'large-monitor-{i}'
            assert event['peer_index']==i
            ready=opened=False
            for frame in event['frames']:
                wire.validate(frame)
                assert frame['signature']['alg']==row['algorithm']
                if frame['type']=='connection.ready':
                    assert not ready and frame['connid']==f'large-receiver-{i}';ready=True
                elif frame['type']=='stream.opened':
                    assert ready and not opened and frame['payload']['stream']['stream_local_id']==handles[i];opened=True
                else:assert frame['type'] in {'connection.quality','recording.vcon-ready'}
            assert ready and opened
        future=row['future_publication'];answer=future['answer'];wire.validate(answer)
        assert answer['signature']['alg']==row['algorithm'] and answer['connid']=='large-future-publisher'
        assert answer['cid'] in conversations and answer['sid']=='native-session'
        assert answer['payload']['by_participant']==pids[0]
        stream=answer['payload']['substrate_setup']['streams'][0]
        assert stream['strm_id']=='large-future-voice' and stream['stream_local_id']==future['publish_handle']!=publisher_handles[0]
        assert future['peak_connections']==100 and future['connections_after_end']==99 and future['state_after_end']=='retired'
        validate_startup(future['startup'], {i for i in range(1,50) if i%3}, {'large-future-publisher'})
        ready=opened=False
        for frame in future['source_events']:
            wire.validate(frame);assert frame['signature']['alg']==row['algorithm'] and frame['cid'] in conversations and frame['sid']=='native-session'
            if frame['type']=='connection.ready':
                assert not ready and frame['connid']=='large-future-publisher';ready=True
            elif frame['type']=='stream.opened':
                assert ready and not opened and frame['connid']=='large-future-publisher';opened=True
                assert frame['payload']['stream']['strm_id']=='large-future-voice'
                assert frame['payload']['stream']['stream_local_id']==future['publish_handle']
            else:assert frame['type'] in {'connection.quality','recording.vcon-ready'}
        assert ready and opened
        for name,allowed in [('publication',{i for i in range(1,50) if i%3}),('after_end',set(range(1,50)))]:
            phase=future[name];assert phase['source_index']==0 and set(phase['allowed'])==allowed
            assert len(phase['decoded_marked'])==50
            assert all(count>10 if i in allowed else count==0 for i,count in enumerate(phase['decoded_marked']))
            for i,seen in enumerate(phase['egress_handles']):assert set(seen)<={handles[i]}
        peers=row['subscription_receipts']
        assert len(peers)==50 and [p['peer_index'] for p in peers]==list(range(50))
        for peer in peers:
            index=peer['peer_index']
            for late in peer['late_self_joins']:
                wire.validate(late)
                assert not self_joined_seen[index];self_joined_seen[index]=True
                assert late['type']=='session.participant.joined' and late['signature']['alg']==row['algorithm']
                assert late['cid'] in conversations and late['sid']=='native-session'
                assert late['payload']['participant']['participant_id']==pids[index]
                assert late['payload']['via_connection']==f'large-publisher-{index}'
            expected=['large-future-end']*2 if index==0 else [f'large-subscribe-{index}']
            if index==49:expected+=['large-unsubscribe']*2+['large-unsubscribe-again']
            frames=peer['frames'];assert len(frames)==2*len(expected)
            for operation,result,ack in zip(expected,frames[::2],frames[1::2]):
                for frame in (result,ack):
                    wire.validate(frame);assert frame['signature']['alg']==row['algorithm']
                assert result['type']=='operation.updated' and result['payload']['state']=='completed'
                assert result['operation_id']==operation and ack['type']=='ack'
                assert result['in_reply_to']==ack['in_reply_to']
            results=frames[::2]
            assert len({f['in_reply_to'] for f in results})==len(results)
            if index==49:
                assert results[1]['payload']['record_id']==results[2]['payload']['record_id']
                assert results[3]['payload']['record_id']!=results[1]['payload']['record_id']
        ended=[f['payload'] for p in row['subscription_receipts'] if p['peer_index']==0 for f in p['frames'] if f['type']=='operation.updated' and f['operation_id']=='large-future-end']
        assert len(ended)==2 and ended[0]['record_id']==ended[1]['record_id']
        assert 0<=row['burst_ready_count']<=50
        assert len(row['recovered_ready_indexes'])==50-row['burst_ready_count']
        assert len(row['selectors'])==49
        for i,command in enumerate(row['selectors'],1):
            assert command['connection']==f'large-receiver-{i}'
            selector=command['selector']
            if i%3==0:assert selector=={'strm_id':'large-voice-0'}
            elif i%3==1:assert selector=={'from_participant':pids[0]}
            else:assert selector=={'from_participant':pids[0],'kinds':['audio']}
        for name,allowed in [('unsubscribed',set()),('fanout',set(range(1,50))),('after_unsubscribe',set(range(1,49))),('after_leave',set(range(1,49))),('after_remove',set(range(1,48)))]:
            phase=row['phases'][name];assert phase['source_index']==0 and set(phase['allowed'])==allowed
            counts=phase['decoded_marked'];assert len(counts)==50
            assert all(count>10 if i in allowed else count==0 for i,count in enumerate(counts))
            assert len(phase['received_packets'])==50
            if name=='unsubscribed':assert phase['received_packets']==[0]*50
            for i,seen in enumerate(phase['egress_handles']):assert set(seen)<={handles[i]}
        for field,last,departed in [('left_events',49,pids[49]),('removed_events',48,pids[48])]:
            frames=row[field];assert len(frames)==last and {f['peer_index'] for f in frames}==set(range(last))
            for item in frames:
                frame=item['frame'];wire.validate(frame)
                assert frame['type']=='session.participant.left' and frame['signature']['alg']==row['algorithm']
                assert frame['payload']['participant_id']==departed;total_left+=1
    assert (total_joined,total_started,total_left)==(4900,100,194)
    print(json.dumps({'passed':True,'signing_scenes':2,'participants_per_scene':50,'signed_publisher_answers':100,'signed_receiver_answers':98,'signed_join_multicasts':total_joined,'signed_started_frames':total_started,'signed_left_multicasts':total_left,'actual_subscribers_per_scene':49,'scope':'Configured local native QUIC50-peer profile with explicit decoded fanout, per-peer handles, unsubscribe/replay and membership lifecycle. Includes later actual publication to33 retained Participant selectors while16 exact selectors remain excluded, and original49-way audio after source end/replay. Does not by itself qualify other substrates, remote/hosting profiles, whole clause or Gate6.'}))

if __name__=='__main__':main()
