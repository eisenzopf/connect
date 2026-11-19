"""Pure closed-file controls. No browser/network qualification or historical regrade."""
import copy
import hashlib
import json
import os
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.rtc_source_frame_witness import (PROFILE, WitnessError, HeaderIndex, WITNESS_CYCLE_BYTES,
    capture_headers, delta, finalize, mapping_summaries, prepare_pair, prepare_association_control,
    prove_headers, read_json, report_bound, decode_census, StatsIndex, PublicationMappings, PUBLICATION_MARKER)


def encode(route, census):
    text = json.dumps(census, separators=(',', ':'))
    candidate = route['video']['original_source_frame_witness']
    candidate.update(census_json=text, encoded_bytes=len(text.encode()), rows=len(census['rows']),
                     pair_count=len(census['pairs']), legacy_draw_repeat_frames=len(census['pairs']))
    route['video']['duplicate_frames'] = len(census['pairs'])


def facts(aliases=1, index=1):
    suffix = '' if index == 1 else str(index)
    source, receiver = 'source'+suffix, 'receiver'+suffix
    sf = ('127.0.0.1', 50000+index*2, '127.0.0.1', 40000+index*2, 10+index, 120)
    tf = ('127.0.0.1', 40001+index*2, '127.0.0.1', 50001+index*2, 21+index, 120)
    epoch = dict(ordinal=0, source_connection='runtime-source'+suffix, source_logical='source_video'+suffix, source_transport='source_transport'+suffix,
        source_generation=3, source_ssrc=sf[4], target_logical='sink_video'+suffix, target_transport='sink_transport'+suffix,
        target_generation=2, target_connection_generation=1, target_ssrc=tf[4], target_pt=120,
        input_sequence=1,input_timestamp=1000,output_sequence=101,output_timestamp=2000,sequence_offset=100,timestamp_offset=1000)
    summary = dict(profile='connect/rtc-original-video-mapping/1',process=4242,timeline=index,
                   original_timeline_retired=True,complete=True,omitted=0,invalid=0,epochs=[epoch])
    def interval(owner, receive, send):
        return dict(owner=owner,receive_stream_id=receive,send_stream_id=send,start_ms=0,end_ms=None,
                    previous=None,operation_id=None,basis='original-initial-public-render')
    source_ledger=[interval(source,'source_receive'+suffix,epoch['source_logical'])]
    target_ledger=[interval(receiver,epoch['target_logical'],'receiver_send'+suffix)]
    census=dict(descriptors=[[receiver,epoch['target_logical'],'target-track',2]],
        rows=[[i,0,2.2+i*.017,i+1,200000+i*1530,7] for i in range(aliases+1)],
        pairs=[[0,i] for i in range(1,aliases+1)],
        control=dict(receiver_route_index=1,source_route_index=0,window_start_seconds=2,window_end_seconds=3,rows=[0,1]),
        public_intervals=target_ledger)
    candidate=dict(profile=PROFILE,collection_passed=True,non_duplicate_passed=True,complete=True)
    route=dict(route=1,source_route_index=0,passed=False,invalid=[],audio=dict(passed=True),sync=dict(status='passed'),
        video=dict(passed=False,duplicate_frames=aliases,changing_frames_per_second=[30]*60,
            decoded_frames_per_second=[30]*60,transition_coverage=dict(original_public_cuts=[]),original_source_frame_witness=candidate))
    encode(route,census)
    def stat(owner, direction, ssrc, stream, track, local, remote):
        return dict(connection_id=owner,type=direction,kind='video',ssrc=ssrc,
            original_track=dict(connection_id=owner,stream_id=stream,track_id=track),
            codec=dict(mime_type='video/VP8',payload_type=120,clock_rate=90000),
            selected_path=dict(nominated=True,succeeded=True,local=dict(address='127.0.0.1',port=local,protocol='udp'),
                remote=dict(address='127.0.0.1',port=remote,protocol='udp'),srtp_cipher=None))
    public_context=dict(conversation_id='conversation',session_id='session')
    source_binding=dict(profile='connect/native-publication-binding/1',complete=True,process=4242,public_connection_id=source,
        runtime_connection_id=epoch['source_connection'],conversation_id='conversation',session_id='session',participant_id='participant'+suffix,
        membership_epoch=2,public_connection_generation=7,offer_record_id='source-offer'+suffix,
        publication_operation_id='source-offer'+suffix,phase='initial-ready',
        streams=[dict(logical_id=epoch['source_logical'],transport_id=epoch['source_transport'],medium='video',direction='both',
            core_stream_generation=3,core_connection_generation=7,source_ready=True,sink_ready=True)])
    summaries=PublicationMappings([summary],source_bindings=[source_binding])
    peers=[[stat(source,'outbound-rtp',sf[4],epoch['source_logical'],'source-track',sf[1],sf[3]),
            stat(receiver,'inbound-rtp',tf[4],epoch['target_logical'],'target-track',tf[3],tf[1])]]
    cycle=dict(cycle=dict(index=index,workload_id='original'+suffix),passed=False,collection_passed=True,
        source_startup=dict(sources=[dict(index=1,attachments=[dict(connection_id=receiver,public_context=copy.deepcopy(public_context),capture_started=True)]),dict(index=0,attachments=[dict(connection_id=source,public_context=copy.deepcopy(public_context),capture_started=True)])]),
        admission=dict(operations=[dict(name='offer-0',operationId='source-offer'+suffix,state='completed')]),
        programme=dict(moves=[],sources=dict(sources=[dict(index=0,attachments=[dict(connection_id=source,public_context=copy.deepcopy(public_context),capture_started=True)]),dict(index=1,attachments=[dict(connection_id=receiver,public_context=copy.deepcopy(public_context),capture_started=True)])])),
        decoded_media=dict(passed=False,routes=[route],source_frame_public_owners=[dict(route=1,intervals=target_ledger),dict(route=0,intervals=source_ledger)],
            rtc_media_pipeline=dict(samples=[dict(requested_seconds=1,completed_seconds=1.1,peers=copy.deepcopy(peers)),
                dict(requested_seconds=3,completed_seconds=3.1,peers=copy.deepcopy(peers))])),
        failure=dict(stage='decoded-media-quality',name='ProfileBudgetsFailed'))
    def point(at):
        row=census['rows'][at]
        return dict(owner=receiver,row_index=row[0],callback_seconds=row[2],presentation=dict(connection_id=receiver,
            stream_id=epoch['target_logical'],track_id='target-track',element_id=2,presentedFrames=row[3],rtpTimestamp=row[4]))
    pair=dict(first=point(0),current=point(1))
    times=[198000]+[199000+i*1530 for i in range(aliases+1)]+[199000+(aliases+1)*1530]
    rows=[]
    for i,ts in enumerate(times):
        rows.append(dict(flow=sf,seq=2+i,ts=ts,marker=True,body_bytes=100,unix_ns=1000000000+i*20000000,record=2*i+1))
        rows.append(dict(flow=tf,seq=102+i,ts=ts+1000,marker=True,body_bytes=100,unix_ns=1005000000+i*20000000,record=2*i+2))
    return cycle,route,pair,summaries,sf,tf,epoch,rows


def pcap(path,rows):
    body=bytearray(struct.pack('<IHHIIII',0xa1b2c3d4,2,4,0,0,262144,0))
    for row in rows:
        f=row['flow'];rtp=struct.pack('!BBHII',0x80,f[5]|0x80,row['seq'],row['ts'],f[4])+bytes(row['body_bytes'])
        udp=struct.pack('!HHHH',f[1],f[3],len(rtp)+8,0)+rtp
        ip=bytes([0x45,0])+struct.pack('!H',20+len(udp))+bytes(4)+bytes([64,17])+bytes(2)+bytes([127,0,0,1])*2
        packet=bytes([2,0,0,0])+ip+udp;ns=row['unix_ns']
        body.extend(struct.pack('<IIII',ns//1000000000,(ns%1000000000)//1000,len(packet),len(packet)));body.extend(packet)
    path.write_bytes(body)
    return dict(observed=True,original_joined=True,exit_code=0,forced=False,kernel_dropped_packets=0,
        pcap_sha256=hashlib.sha256(body).hexdigest(),pcap_bytes=len(body),original_packets_captured=len(rows))


def final_case(folder,sets):
    allrows=[];maps=[];archives=[];bindings=[]
    for cycle,_,_,summaries,*rest in sets:
        index=cycle['cycle']['index'];leaf=folder/'cycles'/('cycle-'+str(index));leaf.mkdir(parents=True)
        raw=json.dumps(cycle).encode();(leaf/'cycle-observation.json').write_bytes(raw)
        archives.append(dict(index=index,workload_id=cycle['cycle']['workload_id'],observation_sha256=hashlib.sha256(raw).hexdigest()))
        bindings.extend(summaries.source_bindings);maps.extend(summaries);allrows.extend(rest[-1])
    log=folder/'server.log';logged=(b'unrelated audio facts '+b'x'*(1400*1024)+b'\n')
    for value in bindings:logged+=b'DEBUG '+PUBLICATION_MARKER+b' binding='+json.dumps(value,separators=(',',':')).encode()+b'\n'
    for value in maps:logged+=b'\x1b[34mDEBUG\x1b[0m RTC original video mapping witness \x1b[3mwitness\x1b[0m\x1b[2m=\x1b[0m'+json.dumps(value,separators=(',',':')).encode()+b'\n'
    log.write_bytes(logged);capture=pcap(folder/'original.pcap',allrows)
    browser=dict(source_revision='original-source',transport='firefox-wss-webrtc',source_frame_witness_profile=PROFILE,collection_passed=True,passed=False,
        resource_cycles=archives,cycles=[dict(index=value['index'],passed=False) for value in archives])
    report=dict(transport='firefox-wss-webrtc',stage='quality',source_revision='original-source',browser_exit_code=0,browser_owner_joined=True,
        browser_observation=browser,browser_observation_sha256='a'*64,packet_capture=capture,
        servers=[dict(name='materialized-server',original_pid=4242,original_joined=True,original_exit_code=0,original_process_group_joined=True)])
    return report,log,logged


def native_role_census(cycle):
    """Same captured dictionaries with explicit native A/B association, no IO."""
    for sample in cycle['decoded_media']['rtc_media_pipeline']['samples']:
        for group in sample['peers']:
            originals=list(group)
            for row in originals:
                if row.get('kind')!='video' or not row.get('original_track'):continue
                tag=row['original_track'];receive=row['type']=='inbound-rtp'
                tag.setdefault('mid','2' if receive else '1');tag.setdefault('transceiver_index',2 if receive else 1)
                row.setdefault('peer_index',1 if receive else 0)
                row['native_rtp_binding']=dict(profile='connect/rtc-native-rtp-role/1',complete=True,reason=None,
                    connection_id=row['connection_id'],peer_index=row['peer_index'],direction='receive' if receive else 'send',
                    native_mid=tag['mid'],transceiver_index=tag['transceiver_index'],native_track_id=tag['track_id'],codec_dictionary_type='codec')
                if receive:
                    preview=copy.deepcopy(row);preview['original_track']=None;preview['ssrc']+=2
                    preview['native_rtp_binding'].update(native_mid='1' if tag['mid']!='1' else '2',
                        transceiver_index=1 if tag['transceiver_index']!=1 else 2,native_track_id='private-preview-track')
                    rtx=copy.deepcopy(row);rtx['ssrc']+=1;rtx['codec'].update(mime_type='video/rtx',payload_type=121,associated_payload_type=row['codec']['payload_type'])
                    group.extend([preview,rtx])


class SourceFrameWitness(unittest.TestCase):
    def test_actual_native_roles_select_only_original_B_without_removing_A_RTX_or_raw_rows(self):
        values=facts();cycle,route,pair,summaries=values[:4]
        baseline=prepare_pair(cycle,route,pair,summaries);native_role_census(cycle)
        archived=copy.deepcopy(cycle)
        self.assertEqual(prepare_pair(cycle,route,pair,summaries),baseline)
        self.assertEqual(cycle,archived)
        for sample in cycle['decoded_media']['rtc_media_pipeline']['samples']:sample['peers'][0].reverse()
        self.assertEqual(prepare_pair(cycle,route,pair,summaries),baseline)
        index=StatsIndex(cycle);owner=pair['first']['owner'];stream=pair['first']['presentation']['stream_id'];track=pair['first']['presentation']['track_id']
        first=index.get(owner,'inbound-rtp',2.2,2.217,stream=stream,track=track,ssrc=values[5][4])
        self.assertEqual(index.get(owner,'inbound-rtp',2.201,2.216,stream=stream,track=track,ssrc=values[5][4]),first)

    def test_native_B_contradictions_unknown_rows_and_stripped_or_competing_roles_never_fall_back(self):
        for change in ['path','codec','ssrc','tag','mid','track','raw-mid','raw-track','refusal','unknown','competing','coherent-competing-B','stripped','rtx-unproved','rtx-apt','rtx-collision']:
            values=facts();cycle=values[0];native_role_census(cycle)
            group=cycle['decoded_media']['rtc_media_pipeline']['samples'][1]['peers'][0]
            target=next(row for row in group if row.get('type')=='inbound-rtp' and row.get('original_track') and row['codec']['mime_type']=='video/VP8')
            if change=='path':target['selected_path']['remote']['port']+=1
            elif change=='codec':target['codec']['payload_type']+=1
            elif change=='ssrc':target['ssrc']+=10
            elif change=='tag':target['original_track']['stream_id']='foreign'
            elif change=='mid':target['native_rtp_binding']['native_mid']='changed'
            elif change=='track':target['native_rtp_binding']['native_track_id']='changed'
            elif change=='raw-mid':target['native_mid']='changed'
            elif change=='raw-track':target['native_track_identifier']='changed'
            elif change=='refusal':target['original_track_refusal']='native-track-identifier-contradiction'
            elif change=='unknown':
                unknown=copy.deepcopy(target);unknown['original_track']=None;unknown.pop('native_rtp_binding');group.append(unknown)
            elif change=='competing':group.append(copy.deepcopy(target))
            elif change=='coherent-competing-B':
                competing=copy.deepcopy(target);competing['native_rtp_binding'].update(native_mid='3',transceiver_index=3)
                competing['original_track'].update(mid='3',transceiver_index=3);group.append(competing)
            elif change=='stripped':
                for row in group:row.pop('native_rtp_binding',None)
            elif change=='rtx-unproved':next(row for row in group if row['codec']['mime_type']=='video/rtx')['native_rtp_binding']['codec_dictionary_type']=None
            elif change=='rtx-apt':next(row for row in group if row['codec']['mime_type']=='video/rtx')['codec']['associated_payload_type']=None
            elif change=='rtx-collision':next(row for row in group if row['codec']['mime_type']=='video/rtx')['codec']['payload_type']=target['codec']['payload_type']
            with self.assertRaises(WitnessError):prepare_pair(*values[:4])

    def test_native_B_roles_apply_to_original_public_terminal_and_receiver_local_high_brackets(self):
        for public in [False,True]:
            values,anchor,binding=boundary_facts(public);native_role_census(values[0])
            self.assertEqual(prepare_pair(*values[:4])['native_binding_boundaries'][0]['direction'],'receive')
            if public:
                pipeline=values[0]['decoded_media']['rtc_media_pipeline']
                row=copy.deepcopy(next(row for row in pipeline['samples'][1]['peers'][0] if row.get('type')=='inbound-rtp' and row.get('original_track') and row['codec']['mime_type']=='video/VP8'))
                row['original_track']=None;row['original_track_refusal']='native-track-identifier-contradiction';pipeline['samples'][0]['peers'][0].append(row)
                with self.assertRaisesRegex(WitnessError,'original-native-role-contradiction'):prepare_pair(*values[:4])
        values,record=retirement_statistics_facts();native_role_census(values[0])
        self.assertTrue(prepare_pair(*values[:4])['native_retirement_statistics'])
        for public in [False,True]:
            for direction in ['send','receive']:
                values,anchor,binding,actual=selection_boundary_facts(public,direction);native_role_census(values[0])
                self.assertEqual(prepare_pair(*values[:4])['native_binding_boundaries'][0]['native_path_kind'],'prflx-address-redacted')
                actual['selected_path']['selected_candidate_pair_changes']+=1
                with self.assertRaises(WitnessError):prepare_pair(*values[:4])

    def test_distinct_original_frames_same_draw_use_actual_pipeline_without_changing_raw_fps(self):
        values=facts();c,r,p,m,sf,tf,e,rows=values
        proof=prepare_pair(c,r,p,m);self.assertEqual(proof['input_ts'],[199000,200530])
        self.assertEqual(prove_headers(rows,sf,tf,e,proof['input_ts'],proof['output_ts'])['matched_packets'],4)
        with tempfile.TemporaryDirectory() as tmp:
            folder=Path(tmp);sets=[facts(22,index) for index in range(1,4)]
            report,log,logged=final_case(folder,sets);raw=[(folder/'cycles'/('cycle-'+str(i))/'cycle-observation.json').read_bytes() for i in range(1,4)]
            final,cycles=finalize(report,folder,log,folder/'original.pcap')
            self.assertTrue(final['passed']);self.assertFalse(report['browser_observation']['passed'])
            proof=final['original_source_frame_adjudication'];self.assertEqual(proof['proof_rows'],66);self.assertEqual(len(proof['association_controls']),3)
            self.assertEqual(len(json.loads(proof['proof_json'])),66);self.assertEqual(proof['server_log_sha256'],hashlib.sha256(logged).hexdigest())
            for i,cycle in enumerate(cycles):
                video=cycle['decoded_media']['routes'][0]['video'];original=sets[i][1]['video']
                self.assertEqual(video['raw_draw_duplicate_frames'],22);self.assertEqual(video['duplicate_frames'],0)
                for field in ['changing_frames_per_second','decoded_frames_per_second']:self.assertEqual(video[field],original[field])
                self.assertEqual((folder/'cycles'/('cycle-'+str(i+1))/'cycle-observation.json').read_bytes(),raw[i])
        with tempfile.TemporaryDirectory() as tmp:
            folder=Path(tmp);zero=list(facts());census=json.loads(zero[1]['video']['original_source_frame_witness']['census_json'])
            census['rows'][1][5]=8;census['pairs']=[];encode(zero[1],census)
            report,log,_=final_case(folder,[zero]);final,_=finalize(report,folder,log,folder/'original.pcap')
            self.assertEqual(final['original_source_frame_adjudication']['proof_rows'],0)
            self.assertEqual(len(final['original_source_frame_adjudication']['association_controls']),1)

    def test_authoritative_source_bridge_keeps_public_and_runtime_namespaces_and_generations_distinct(self):
        cycle,route,pair,maps,*_=facts()
        original=copy.deepcopy(cycle)
        proof=prepare_pair(cycle,route,pair,maps)
        self.assertEqual(proof['input_ts'],[199000,200530])
        self.assertNotEqual(maps[0]['epochs'][0]['source_connection'],'source')
        self.assertEqual(maps.source_bindings[0]['public_connection_generation'],7)
        self.assertEqual(maps.source_bindings[0]['streams'][0]['core_stream_generation'],3)
        self.assertIn('source_publication_binding',proof)
        self.assertEqual(cycle,original,'Identity proof must not rewrite source rows or owner ledgers')
        # Actual source handoffs use a distinct physical runtime plus the exact
        # original public operation/cut, independently of receiver continuity.
        ledger=cycle['decoded_media']['source_frame_public_owners'][1]['intervals']
        old=copy.deepcopy(ledger[0]);old.update(owner='old-source',end_ms=1400,send_stream_id='old-source-video')
        ledger[0].update(start_ms=1400,previous='old-source',operation_id='source-move',basis='validated-receipts-local-public-render-cut')
        ledger.insert(0,old)
        cycle['source_startup']['sources'][1]['attachments'][0]['connection_id']='old-source'
        cycle['programme']['sources']['sources'][0]['attachments'].append(dict(connection_id='old-source',capture_started=True,
            public_context=dict(conversation_id='conversation',session_id='session')))
        cycle['programme']['moves']=[dict(source_index=0,ordinal=0,source='old-source',target='source',operation_id='source-move')]
        cycle['decoded_media']['routes'].append(dict(route=0,video=dict(transition_coverage=dict(original_public_cuts=[
            dict(id='source',atMs=1400,basis='validated-receipts-local-public-render-cut')]))))
        maps[0]['epochs'][0]['source_connection']='replacement-runtime-source'
        maps.source_bindings[0].update(runtime_connection_id='replacement-runtime-source',phase='replacement-published',
            publication_operation_id='source-move',public_connection_generation=8)
        # Actual split replacement publication is Core Receive (browser SEND),
        # with a ready source and no Core sink on this logical stream.
        maps.source_bindings[0]['streams'][0].update(direction='receive',source_ready=True,sink_ready=False)
        replacement=prepare_pair(cycle,route,pair,maps)
        self.assertEqual(replacement['source_publication_binding']['phase'],'replacement-published')
        self.assertEqual(replacement['source_publication_binding']['public_connection_generation'],8)
        self.assertEqual(replacement['input_ts'],proof['input_ts'])
        self.assertEqual(replacement['source_publication_binding']['stream'],maps.source_bindings[0]['streams'][0])
        self.assertEqual(replacement['source_publication_binding']['stream']['direction'],'receive')
        self.assertIs(replacement['source_publication_binding']['stream']['source_ready'],True)
        self.assertIs(replacement['source_publication_binding']['stream']['sink_ready'],False)
        for change in [dict(direction='send'),dict(source_ready=False)]:
            with self.subTest(replacement_core_port=change),self.assertRaisesRegex(WitnessError,'publication-core-source-port-changed'):
                bad=copy.deepcopy(maps);bad.source_bindings[0]['streams'][0].update(change);prepare_pair(cycle,route,pair,bad)
        for change in [dict(phase='initial-ready'),dict(publication_operation_id='foreign-move'),dict(runtime_connection_id='old-runtime')]:
            with self.subTest(replacement_change=change),self.assertRaises(WitnessError):
                bad=copy.deepcopy(maps);bad.source_bindings[0].update(change);prepare_pair(cycle,route,pair,bad)

    def test_missing_duplicate_or_conflicting_authoritative_public_runtime_bindings_refuse(self):
        cycle,route,pair,maps,*_=facts()
        for bindings in [[],maps.source_bindings*2,
                [maps.source_bindings[0],{**maps.source_bindings[0],'runtime_connection_id':'old-runtime'}],
                [maps.source_bindings[0],{**maps.source_bindings[0],'public_connection_id':'foreign-public'}]]:
            with self.subTest(bindings=bindings),self.assertRaises(WitnessError):
                prepare_pair(cycle,route,pair,PublicationMappings(list(maps),source_bindings=copy.deepcopy(bindings)))
        oversized=[{**maps.source_bindings[0],'public_connection_id':'public-'+str(i),'runtime_connection_id':'runtime-'+str(i)} for i in range(49)]
        with self.assertRaises(WitnessError):prepare_pair(cycle,route,pair,PublicationMappings(list(maps),source_bindings=oversized))
        with self.assertRaises(WitnessError):prepare_pair(cycle,route,pair,list(maps))
        equal=copy.deepcopy(maps);equal[0]['epochs'][0]['source_connection']='source'
        with self.assertRaises(WitnessError):prepare_pair(cycle,route,pair,equal)

    def test_publication_binding_wrong_process_session_stream_generation_or_operation_refuses(self):
        cycle,route,pair,maps,*_=facts()
        changes=[{'complete':False},{'process':4243},{'public_connection_id':'old-public'},{'runtime_connection_id':'old-runtime'},
            {'conversation_id':'foreign-conversation'},{'session_id':'foreign-session'},
            {'offer_record_id':''},{'publication_operation_id':'foreign-operation'},
            {'phase':'replacement-published'},{'profile':'unknown-profile'}]
        for change in changes:
            with self.subTest(change=change),self.assertRaises(WitnessError):
                bad=copy.deepcopy(maps);bad.source_bindings[0].update(change);prepare_pair(cycle,route,pair,bad)
        for key,value in [('logical_id','foreign-logical'),('transport_id','old-transport'),
                ('core_stream_generation',4),('medium','audio'),('direction','send'),('source_ready',False)]:
            with self.subTest(stream_field=key),self.assertRaises(WitnessError):
                bad=copy.deepcopy(maps);bad.source_bindings[0]['streams'][0][key]=value;prepare_pair(cycle,route,pair,bad)

    def test_authenticated_source_and_receiver_capture_contexts_are_required_without_cross_session_inference(self):
        cycle,route,pair,maps,*_=facts()
        for source_index in [0,1]:
            for change in [None,{'conversation_id':'foreign','session_id':'session'},
                    {'conversation_id':'conversation','session_id':'foreign'}]:
                with self.subTest(source_index=source_index,change=change),self.assertRaises(WitnessError):
                    bad=copy.deepcopy(cycle)
                    for group in [bad['source_startup']['sources'],bad['programme']['sources']['sources']]:
                        for source in group:
                            if source['index']!=source_index:continue
                            for attachment in source['attachments']:
                                if change is None:attachment.pop('public_context',None)
                                else:attachment['public_context']=copy.deepcopy(change)
                    prepare_pair(bad,bad['decoded_media']['routes'][0],pair,maps)
            with self.assertRaises(WitnessError):
                bad=copy.deepcopy(cycle);bad['programme']['sources']['sources'][source_index]['attachments'][0]['capture_started']=False
                prepare_pair(bad,bad['decoded_media']['routes'][0],pair,maps)

    def test_original_replay_retag_and_changed_footer_do_not_create_a_new_original_frame(self):
        *_,sf,tf,e,rows=facts();altered=copy.deepcopy(rows);altered[4]['ts']=altered[2]['ts']
        with self.assertRaises(WitnessError):prove_headers(altered,sf,tf,e,[199000,200530],[200000,201530])
        with self.assertRaises(WitnessError):prove_headers(rows,sf,tf,e,[199000,199000],[200000,201530])
        c,r,p,m,*_=facts();census=json.loads(r['video']['original_source_frame_witness']['census_json']);census['rows'][1][4]=census['rows'][0][4];census['rows'][1][5]=8;encode(r,census)
        with self.assertRaises(WitnessError):decode_census(r)

    def test_missing_ambiguous_omitted_and_invalid_generations_never_resolve(self):
        c,r,p,m,*_=facts()
        for maps in [[],m+m,[{**m[0],'complete':False,'omitted':1}],[{**m[0],'epochs':[{**m[0]['epochs'][0],'source_generation':0}]}],
                     [{**m[0],'epochs':[{**m[0]['epochs'][0],'target_connection':'foreign'}]}]]:
            with self.assertRaises(WitnessError):prepare_pair(c,r,p,maps)
        for change in ['missing','moved','source','window','stats','census','bytes']:
            cycle,route=copy.deepcopy(c),copy.deepcopy(r);cycle['decoded_media']['routes']=[route]
            value=json.loads(route['video']['original_source_frame_witness']['census_json'])
            if change=='missing':value['control']=None
            elif change=='moved':cycle['programme']['moves']=[dict(source_index=0,ordinal=0)]
            elif change=='source':cycle['source_startup']['sources'][1]['attachments'][0]['connection_id']='foreign'
            elif change=='window':value['rows'][0][2]=1.99
            elif change=='stats':cycle['decoded_media']['rtc_media_pipeline']['samples']=[]
            elif change=='census':value['pairs']=[]
            else:value['padding']='x'*WITNESS_CYCLE_BYTES
            encode(route,value)
            with self.assertRaises(WitnessError):prepare_association_control(cycle,m)
        with tempfile.TemporaryDirectory() as tmp:
            log=Path(tmp)/'server.log';message=b'RTC original video mapping witness';valid=PUBLICATION_MARKER+b' binding='+json.dumps(m.source_bindings[0],separators=(',',':')).encode()+b'\n'+message+b' witness='+json.dumps(m[0],separators=(',',':')).encode();noise=b'unrelated '+b'x'*(1400*1024)+b'\n'
            crossing=b'x'*(64*1024-len(message)//2)+valid;log.write_bytes(noise+crossing)
            parsed,digest=mapping_summaries(log,4242);self.assertEqual(parsed,m);self.assertEqual(parsed.source_bindings,m.source_bindings);self.assertEqual(digest,hashlib.sha256(noise+crossing).hexdigest())
            for raw in [noise,b'x'*(300*1024)+valid+b'\n',valid+b'x'*(300*1024),valid+b'\n'+valid+b'\n',message+b' witness={not-json}']:
                log.write_bytes(raw)
                with self.assertRaises(WitnessError):mapping_summaries(log,4242)
            log.write_bytes(valid)
            with self.assertRaises(WitnessError):mapping_summaries(log,9999)
            with log.open('wb') as sparse:sparse.truncate(256*1024**2+1)
            with self.assertRaises(WitnessError):mapping_summaries(log,4242)

    def test_source_or_current_target_binding_switch_inside_window_refuses(self):
        c,r,p,m,*_=facts()
        corrupt=copy.deepcopy(m);corrupt[0]['epochs'][0]['input_sequence']+=1
        with self.assertRaisesRegex(WitnessError,'mapping-coordinate-offset-mismatch'):prepare_pair(c,r,p,corrupt)
        for key in ['source_generation','target_generation','target_connection_generation','source_ssrc','target_transport']:
            changed=copy.deepcopy(m);n=copy.deepcopy(changed[0]['epochs'][0]);n.update(ordinal=1,output_timestamp=201000,input_timestamp=200000)
            n[key]='successor' if key=='target_transport' else n[key]+1;changed[0]['epochs'].append(n)
            with self.assertRaises(WitnessError):prepare_pair(c,r,p,changed)
        # A replacement's distinct SEND/RECEIVE IDs retain exact direction.
        changed=copy.deepcopy(c);pair=copy.deepcopy(p);route=changed['decoded_media']['routes'][0]
        ledger=changed['decoded_media']['source_frame_public_owners'][0]['intervals'];ledger[0]['end_ms']=1000
        ledger.append(dict(owner='new',receive_stream_id='sink_video',send_stream_id='different-send',start_ms=1000,end_ms=None,previous='receiver',operation_id='op',basis='validated-receipts-local-public-render-cut'))
        changed['programme']['moves']=[dict(source_index=1,ordinal=0,source='receiver',target='new',operation_id='op')]
        route['video']['transition_coverage']['original_public_cuts']=[dict(id='new',atMs=1000,basis='validated-receipts-local-public-render-cut')]
        changed['programme']['sources']['sources'][1]['attachments'].append(dict(connection_id='new',public_context=dict(conversation_id='conversation',session_id='session'),capture_started=True))
        for point in pair.values():point['owner']='new';point['presentation']['connection_id']='new'
        for sample in changed['decoded_media']['rtc_media_pipeline']['samples']:
            sample['peers'][0][1]['connection_id']='new';sample['peers'][0][1]['original_track']['connection_id']='new'
        self.assertEqual(prepare_pair(changed,route,pair,m)['input_ts'],[199000,200530])
        changed['decoded_media']['source_frame_public_owners'][1]['intervals'][0]['send_stream_id']='wrong-direction'
        with self.assertRaises(WitnessError):prepare_pair(changed,route,pair,m)

    def test_actual_header_duplicates_gaps_or_wrong_flow_cannot_be_fitted_away(self):
        *_,sf,tf,e,rows=facts()
        for altered in [rows[:-1],rows+[dict(rows[1])],[{**v,'body_bytes':101} if i==1 else v for i,v in enumerate(rows)],
                        [{**v,'flow':('127.0.0.1',1,*v['flow'][2:])} if i==1 else v for i,v in enumerate(rows)]]:
            with self.assertRaises(WitnessError):prove_headers(altered,sf,tf,e,[199000,200530],[200000,201530])
        *_,sf,tf,e,rows=facts(3);missing=[r for r in rows if not(r['flow']==tf and r['ts']==201530)]
        with self.assertRaisesRegex(WitnessError,'source-frame-missing-from-target'):prove_headers(missing,sf,tf,e,[199000,203590],[200000,204590])
        # Unrelated same source flow/other target epoch cannot contaminate proof.
        *_,sf,tf,e,base=facts();extra=[];tf2=(*tf[:3],tf[3]+100,tf[4]+10,tf[5])
        for row in base:extra.append({**row,'flow':tf2 if row['flow']==tf else sf,'ts':row['ts']+500000,'record':row['record']+100,'unix_ns':row['unix_ns']+2000000000})
        self.assertEqual(prove_headers(list(reversed(base+extra)),sf,tf,e,[199000,200530],[200000,201530])['matched_packets'],4)
        self.assertEqual(delta(20,0xfffffff0),36)
        with self.assertRaises(WitnessError):delta(0,0x80000000)

    def test_original_closed_capture_hash_drops_and_regular_fd_are_mandatory(self):
        *_,sf,tf,e,rows=facts()
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/'original.pcap';capture=pcap(path,rows);windows=[(sf,199000,200530),(tf,200000,201530)]
            index=capture_headers(path,capture,windows);self.assertEqual(len(index),8)
            self.assertEqual(prove_headers(index,sf,tf,e,[199000,200530],[200000,201530])['matched_packets'],4)
            for field in ['observed','original_joined']:
                with self.assertRaises(WitnessError):capture_headers(path,{**capture,field:False},windows)
            for changes in [dict(kernel_dropped_packets=1),dict(pcap_sha256='0'*64)]:
                with self.assertRaises(WitnessError):capture_headers(path,{**capture,**changes},windows)
            with self.assertRaises(WitnessError):capture_headers(path,capture,windows,deadline=0)
            with self.assertRaises(WitnessError):capture_headers(path,capture,windows,auxiliary_bytes=HeaderIndex.LIMIT)
            fifo=Path(tmp)/'fifo';os.mkfifo(fifo)
            with self.assertRaises(WitnessError):read_json(fifo)
        with tempfile.TemporaryDirectory() as tmp:
            folder=Path(tmp);report,log,_=final_case(folder,[facts()]);clock=[100.0];dump=json.dumps
            def final_dump(value,*args,**kwargs):
                encoded=dump(value,*args,**kwargs)
                if isinstance(value,dict) and 'original_source_frame_adjudication' in value:clock[0]=130.0
                return encoded
            with patch('lib.rtc_source_frame_witness.time.monotonic',side_effect=lambda:clock[0]), patch('lib.rtc_source_frame_witness.json.dumps',side_effect=final_dump):
                with self.assertRaisesRegex(WitnessError,'closed-metadata-time-bound'):finalize(report,folder,log,folder/'original.pcap')


    def test_cross_owner_outside_window_unbound_track_and_unknown_stats_remain_failures(self):
        c,r,p,m,*_=facts()
        for change in ['owner','outside','track','stats','private','peer','ambiguous','fractional']:
            cycle,pair=copy.deepcopy(c),copy.deepcopy(p)
            if change=='owner':pair['current']['owner']='replacement'
            elif change=='outside':pair['first']['callback_seconds']=.9
            elif change=='track':pair['current']['presentation']['track_id']='other'
            elif change=='stats':cycle['decoded_media']['rtc_media_pipeline']['samples']=[]
            elif change=='private':cycle['decoded_media']['source_frame_public_owners'][0]['intervals'][0]['start_ms']=2300
            elif change=='fractional':pair['current']['presentation']['rtpTimestamp']=2.5
            else:
                for sample in cycle['decoded_media']['rtc_media_pipeline']['samples']:
                    if change=='peer':sample['peers'][0][1]['original_track']['connection_id']='foreign'
                    else:
                        other=copy.deepcopy(sample['peers'][0][1]);other['connection_id']='foreign';other['original_track']['connection_id']='foreign';sample['peers'][0].append(other)
            with self.assertRaises((WitnessError,KeyError)):prepare_pair(cycle,r,pair,m)
        # Still refuse when BOTH rows are genuinely before the validated cut
        # but the retired owner's last1Hz poll cannot bracket their end.
        cycle=copy.deepcopy(c);ledger=cycle['decoded_media']['source_frame_public_owners'][0]['intervals'];ledger[0]['end_ms']=2250
        ledger.append(dict(owner='new',receive_stream_id='new-video',send_stream_id='new-send',start_ms=2250,end_ms=None,
            previous='receiver',operation_id='tail-op',basis='validated-receipts-local-public-render-cut'))
        cycle['programme']['moves']=[dict(source_index=1,ordinal=0,source='receiver',target='new',operation_id='tail-op')]
        cycle['decoded_media']['routes'][0]['video']['transition_coverage']['original_public_cuts']=[dict(id='new',atMs=2250,basis='validated-receipts-local-public-render-cut')]
        cycle['decoded_media']['rtc_media_pipeline']['samples'][1]['peers'][0].pop()
        self.assertLess(p['current']['callback_seconds'],2.25)
        with self.assertRaisesRegex(WitnessError,'owner-statistics-ambiguous'):prepare_pair(cycle,cycle['decoded_media']['routes'][0],p,m)


    def test_historical_cli_failure_wrong_transport_or_incomplete_collection_never_promotes(self):
        base=dict(transport='firefox-wss-webrtc',stage='quality',browser_exit_code=1,browser_owner_joined=True,browser_observation=dict(passed=False),servers=[])
        for value in [base,{**base,'browser_exit_code':0},{**base,'transport':'chrome-wss-webrtc','browser_exit_code':0}]:
            with self.assertRaises(WitnessError):finalize(value,Path('/not-read'),Path('/not-read'),Path('/not-read'))

    def test_new_profile_markers_cannot_fall_back_to_legacy_after_optin_is_removed(self):
        value=dict(profile=PROFILE,complete=True,passed=True,source_revision='original',pcap_sha256='b'*64,raw_browser_sha256='a'*64,
            proof_json='[]',proof_rows=0,association_controls=[dict(profile='connect/rtc-passive-source-association/1',cycle=1,route=1,matched_packets=4,original_timestamps=[10,20])])
        report=dict(transport='firefox-wss-webrtc',stage='quality',browser_exit_code=0,browser_owner_joined=True,
            source_inputs_unchanged=True,sdk_dist_inputs_unchanged=True,executable_unchanged=True,source_revision='original',
            diagnostic_options=dict(rtc_source_frame_witness=True),packet_capture=dict(pcap_sha256='b'*64),raw_browser_observation_sha256='a'*64,
            source_frame_adjudication=value,browser_observation=dict(passed=True,source_frame_witness_profile=PROFILE,original_source_frame_adjudication=value,resource_cycles=[dict(index=1)]))
        self.assertTrue(report_bound(report))
        for change in ['control','cycle','profile','flag','proof']:
            changed=copy.deepcopy(report)
            if change=='control':changed['source_frame_adjudication'].pop('association_controls')
            elif change=='cycle':changed['source_frame_adjudication']['association_controls'][0]['cycle']=2
            elif change=='profile':changed['source_frame_adjudication']['profile']='connect/native-rtc-original-source-frame/1'
            elif change=='proof':changed['source_frame_adjudication'].pop('proof_json')
            else:changed.pop('diagnostic_options')
            self.assertFalse(report_bound(changed))
        self.assertTrue(report_bound(dict(browser_observation=dict(decoded_media=dict(routes=[])))))




def boundary_facts(public=False, replacement=False):
    values=facts();cycle,route,pair=values[:3]
    cycle['programme']['source_programme']={'originMs':1000}
    pipeline=cycle['decoded_media']['rtc_media_pipeline']
    for sample in pipeline['samples']:
        for row in sample['peers'][0]:
            row['peer_index']=0 if row['type']=='outbound-rtp' else 1
            row['original_track'].update(transceiver_index=0,mid='video0')
    actual=pipeline['samples'][1 if public else 0]['peers'][0][1]
    tag=copy.deepcopy(actual['original_track']);path=actual['selected_path'];codec=actual['codec']
    captured=3050 if not public else 2500
    coordinate=pair['first' if public else 'current']['presentation']['rtpTimestamp']
    binding=dict(complete=True,reason=None,track_state='ended' if not public else 'live',original_track=tag,
        selected_udp_pair=dict(local=copy.deepcopy(path['local']),remote=copy.deepcopy(path['remote']),
            ice_state='connected',dtls_state='connected',basis='RTCIceTransport.getSelectedCandidatePair'),
        codec_declarations=dict(local=[copy.deepcopy(codec)],remote=[copy.deepcopy(codec)]),
        sync_source=dict(source=actual['ssrc'],rtpTimestamp=coordinate,timestamp=1794700000000,
            timestamp_clock='native-uninterpreted'))
    name='public_binding' if public else 'terminal_binding'
    anchor=dict(profile='connect/rtc-native-'+('public' if public else 'terminal')+'-binding/1',peer_index=1,
        connection_id='receiver',captured_at_ms=captured,captured_done_at_ms=captured+.1,complete=True,reason=None,
        bindings=dict(receive=binding))
    if not public:anchor.update(native_close_observed=True,native_closed_at_ms=captured+.2)
    pipeline['binding_boundaries']=dict(profile='connect/rtc-native-binding-boundaries/1',origin_ms=1000,
        complete=True,peers=[dict(peer_index=1,connection_id='receiver',**{name:anchor})])
    pipeline['samples'][0 if public else 1]['peers'][0].pop(1)
    ledger=cycle['decoded_media']['source_frame_public_owners'][0]['intervals']
    if replacement:
        old=copy.deepcopy(ledger[0]);old.update(owner='previous',end_ms=1400)
        ledger[0].update(start_ms=1400,previous='previous',operation_id='original-op',basis='validated-receipts-local-public-render-cut')
        ledger.insert(0,old)
        cycle['source_startup']['sources'][0]['attachments'][0]['connection_id']='previous'
        cycle['programme']['moves']=[dict(source_index=1,ordinal=0,source='previous',target='receiver',operation_id='original-op')]
        route['video']['transition_coverage']['original_public_cuts']=[dict(id='receiver',atMs=1400,basis='validated-receipts-local-public-render-cut')]
    interval=ledger[-1]
    interval['native_public_binding']=dict(profile='connect/rtc-validated-public-binding/1',complete=True,route=1,
        connection_id='receiver',previous=interval['previous'],operation_id=interval['operation_id'],cut_ms=interval['start_ms'],
        basis=interval['basis'],peer_index=1,captured_at_ms=captured,captured_done_at_ms=captured+.1)
    census=decode_census(route);census['public_intervals']=copy.deepcopy(ledger);encode(route,census)
    return values,anchor,binding


class NativeBindingBoundaryControls(unittest.TestCase):
    def test_terminal_native_binding_uses_real_lo_and_conservative_coordinate_without_mutating_samples(self):
        values,anchor,binding=boundary_facts();cycle,route,pair,summaries=values[:4]
        before=copy.deepcopy(cycle['decoded_media']['rtc_media_pipeline']['samples'])
        result=prepare_pair(cycle,route,pair,summaries)
        self.assertEqual(result['native_binding_boundaries'][0]['profile'],'connect/rtc-native-terminal-binding/1')
        self.assertEqual(before,cycle['decoded_media']['rtc_media_pipeline']['samples'])
        self.assertEqual(result['target_flow'],values[5])
        self.assertEqual(binding['track_state'],'ended')

    def test_public_start_binding_requires_original_operation_cut_and_real_hi(self):
        for replacement in (False,True):
            values,anchor,binding=boundary_facts(True,replacement);cycle,route,pair,summaries=values[:4]
            result=prepare_pair(cycle,route,pair,summaries)
            self.assertEqual(result['native_binding_boundaries'][0]['profile'],'connect/rtc-native-public-binding/1')
            pipeline=cycle['decoded_media']['rtc_media_pipeline']
            unbound=copy.deepcopy(pipeline['samples'][1]['peers'][0][1]);unbound['original_track']=None
            pipeline['samples'][0]['peers'][0].append(unbound)
            self.assertEqual(prepare_pair(cycle,route,pair,summaries)['native_binding_boundaries'][0]['direction'],'receive')
            unbound['codec']['payload_type']=121
            with self.assertRaisesRegex(WitnessError,'contradictory-unbound-regular-statistics'):
                prepare_pair(cycle,route,pair,summaries)
            unbound['codec']['payload_type']=120
            link=cycle['decoded_media']['source_frame_public_owners'][0]['intervals'][-1]['native_public_binding']
            link['operation_id']='other'
            with self.assertRaisesRegex(WitnessError,'validated-public-binding-operation-unproved'):
                prepare_pair(cycle,route,pair,summaries)

    def test_native_boundary_never_replaces_both_real_rtp_endpoints_or_regular_contradictions(self):
        values,anchor,binding=boundary_facts();cycle,route,pair,summaries=values[:4]
        cycle['decoded_media']['rtc_media_pipeline']['samples'][0]['peers'][0].pop(1)
        with self.assertRaisesRegex(WitnessError,'needs-one-real-rtp-endpoint'):prepare_pair(cycle,route,pair,summaries)
        values,anchor,binding=boundary_facts();cycle,route,pair,summaries=values[:4]
        sample=copy.deepcopy(cycle['decoded_media']['rtc_media_pipeline']['samples'][0]);sample.update(requested_seconds=2.22,completed_seconds=2.23)
        sample['peers'][0][1]['codec']['payload_type']=121
        cycle['decoded_media']['rtc_media_pipeline']['samples'].insert(1,sample)
        # This complete after-pair sample uses the normal bracket, which must
        # reject its changed codec before considering any optional boundary.
        with self.assertRaisesRegex(WitnessError,'source-or-receiver-state-changed'):prepare_pair(cycle,route,pair,summaries)
        # Put the SAME contradictory observation inside the pair, leaving the
        # real later endpoint missing so the terminal fallback is exercised.
        sample.update(requested_seconds=2.205,completed_seconds=2.210)
        with self.assertRaisesRegex(WitnessError,'contradictory-regular-statistics'):prepare_pair(cycle,route,pair,summaries)

    def test_terminal_and_public_reported_coordinates_refuse_queued_or_reordered_uncovered_pair(self):
        for public in (False,True):
            values,anchor,binding=boundary_facts(public);cycle,route,pair,summaries=values[:4]
            binding['sync_source']['rtpTimestamp']+=1 if public else -1
            with self.assertRaisesRegex(WitnessError,'coordinate-does-not-cover-pair'):prepare_pair(cycle,route,pair,summaries)

    def test_boundary_native_peer_tag_mid_path_codec_and_source_must_match_real_stats(self):
        mutations=[lambda a,b:a.update(peer_index=3),lambda a,b:b['original_track'].update(mid='other'),
            lambda a,b:b['selected_udp_pair']['local'].update(port=1234),
            lambda a,b:b['codec_declarations']['remote'][0].update(payload_type=121),
            lambda a,b:b['sync_source'].update(source=123),lambda a,b:a.update(complete=False),
            lambda a,b:a.update(native_close_observed=False)]
        for mutate in mutations:
            values,anchor,binding=boundary_facts();mutate(anchor,binding)
            with self.assertRaises(WitnessError):prepare_pair(*values[:4])

    def test_public_anchor_reuse_after_new_cut_or_late_capture_and_duplicate_owner_are_unknown(self):
        for mode in ('old-cut','after-pair','duplicate','missing-link'):
            values,anchor,binding=boundary_facts(True,True);cycle,route,pair,summaries=values[:4]
            if mode=='old-cut':anchor['captured_at_ms']=2000
            elif mode=='after-pair':anchor['captured_done_at_ms']=3300
            elif mode=='duplicate':cycle['decoded_media']['rtc_media_pipeline']['binding_boundaries']['peers']*=2
            else:del cycle['decoded_media']['source_frame_public_owners'][0]['intervals'][-1]['native_public_binding']
            with self.assertRaises(WitnessError):prepare_pair(cycle,route,pair,summaries)

    def test_cached_source_binding_rechecks_each_pairs_original_temporal_bounds(self):
        values,anchor,binding=boundary_facts(True);cycle=values[0]
        pipeline=cycle['decoded_media']['rtc_media_pipeline'];source=pipeline['samples'][1]['peers'][0][0]
        # Exercise the outbound cache (no receive-coordinate key) explicitly.
        send=copy.deepcopy(binding);send.pop('sync_source');send['original_track']=copy.deepcopy(source['original_track'])
        send['primary_ssrc']=source['ssrc'];send['selected_udp_pair']['local']=copy.deepcopy(source['selected_path']['local'])
        send['selected_udp_pair']['remote']=copy.deepcopy(source['selected_path']['remote'])
        anchor.update(connection_id='source',peer_index=0,bindings=dict(send=send))
        pipeline['samples'][0]['peers'][0]=[]
        pipeline['binding_boundaries']['peers']=[dict(connection_id='source',peer_index=0,public_binding=anchor)]
        target=cycle['decoded_media']['source_frame_public_owners'][1]['intervals'][0]
        target['native_public_binding']=dict(profile='connect/rtc-validated-public-binding/1',complete=True,route=0,
            connection_id='source',previous=None,operation_id=None,cut_ms=0,basis=target['basis'],peer_index=0,
            captured_at_ms=anchor['captured_at_ms'],captured_done_at_ms=anchor['captured_done_at_ms'])
        stats=StatsIndex(cycle)
        stats.get('source','outbound-rtp',2.2,2.217,stream=source['original_track']['stream_id'])
        with self.assertRaisesRegex(WitnessError,'public-binding-after-pair'):
            stats.get('source','outbound-rtp',1.4,2.217,stream=source['original_track']['stream_id'])

    def test_regular_rtp_bracket_is_preferred_even_when_optional_native_anchor_is_incomplete(self):
        values=facts();cycle,route,pair,summaries=values[:4]
        cycle['decoded_media']['rtc_media_pipeline']['binding_boundaries']={'complete':False}
        result=prepare_pair(cycle,route,pair,summaries)
        self.assertEqual(result['native_binding_boundaries'],[])

    def test_source_terminal_anchor_requires_original_local_sdp_primary_not_msid_track_equality(self):
        values,anchor,binding=boundary_facts();cycle,route,pair,summaries=values[:4]
        pipeline=cycle['decoded_media']['rtc_media_pipeline'];source=pipeline['samples'][0]['peers'][0][0]
        # Restore target's ordinary hi, remove only source's hi.
        restored=copy.deepcopy(pipeline['samples'][0]['peers'][0][1]);pipeline['samples'][1]['peers'][0]=[restored]
        send=copy.deepcopy(binding);send.pop('sync_source');send['original_track']=copy.deepcopy(source['original_track']);send['primary_ssrc']=source['ssrc']
        send['selected_udp_pair']['local']=copy.deepcopy(source['selected_path']['local']);send['selected_udp_pair']['remote']=copy.deepcopy(source['selected_path']['remote'])
        anchor.update(connection_id='source',peer_index=0,bindings=dict(send=send))
        pipeline['binding_boundaries']['peers']=[dict(connection_id='source',peer_index=0,terminal_binding=anchor)]
        self.assertEqual(prepare_pair(cycle,route,pair,summaries)['native_binding_boundaries'][0]['direction'],'send')
        send['primary_ssrc']+=1
        with self.assertRaisesRegex(WitnessError,'sender-primary-changed'):prepare_pair(cycle,route,pair,summaries)



def selection_boundary_facts(public=False,direction='receive'):
    values,anchor,binding=boundary_facts(public,public)
    cycle=values[0];pipeline=cycle['decoded_media']['rtc_media_pipeline']
    actual_index=1 if public else 0
    actual=pipeline['samples'][actual_index]['peers'][0][1]
    if direction=='send':
        source=pipeline['samples'][actual_index]['peers'][0][0]
        pipeline['samples'][0 if public else 1]['peers'][0]=[copy.deepcopy(actual)]
        send=copy.deepcopy(binding);send.pop('sync_source')
        send['original_track']=copy.deepcopy(source['original_track']);send['primary_ssrc']=source['ssrc']
        send['selected_udp_pair']['local']=copy.deepcopy(source['selected_path']['local'])
        send['selected_udp_pair']['remote']=copy.deepcopy(source['selected_path']['remote'])
        binding=send;actual=source
        anchor.update(connection_id='source',peer_index=0,bindings=dict(send=send))
        name='public_binding' if public else 'terminal_binding'
        pipeline['binding_boundaries']['peers']=[dict(connection_id='source',peer_index=0,**{name:anchor})]
        interval=cycle['decoded_media']['source_frame_public_owners'][1]['intervals'][0]
        interval['native_public_binding']=dict(profile='connect/rtc-validated-public-binding/1',complete=True,route=0,
            connection_id='source',previous=None,operation_id=None,cut_ms=0,basis=interval['basis'],peer_index=0,
            captured_at_ms=anchor['captured_at_ms'],captured_done_at_ms=anchor['captured_done_at_ms'])
    epoch=dict(profile='connect/rtc-native-selection-epoch/1',transport_id=f"p{anchor['peer_index']}t1",
        armed_before_selection=True,initial_selection_count=0,selection_count=1,state_version=3)
    actual['selected_path'].update(transport_epoch=copy.deepcopy(epoch),selected_candidate_pair_changes=1)
    path=binding['selected_udp_pair'];path.update(kind='prflx-address-redacted',transport_epoch=copy.deepcopy(epoch))
    path['remote'].update(address=None,type='prflx',candidate_empty=True)
    return values,anchor,binding,actual


class NativeSelectionEpochControls(unittest.TestCase):
    def test_typed_prflx_selection_epoch_binds_all_roles_and_boundaries_without_address_fill(self):
        for public in (False,True):
            for direction in ('send','receive'):
                with self.subTest(public=public,direction=direction):
                    values,anchor,binding,actual=selection_boundary_facts(public,direction)
                    before=copy.deepcopy(values[0])
                    result=prepare_pair(*values[:4]);proof=result['native_binding_boundaries'][0]
                    self.assertEqual(proof['native_path_kind'],'prflx-address-redacted')
                    self.assertEqual(proof['transport_epoch'],actual['selected_path']['transport_epoch'])
                    self.assertEqual(result['source_flow'],values[4]);self.assertEqual(result['target_flow'],values[5])
                    self.assertIsNone(binding['selected_udp_pair']['remote']['address'])
                    self.assertEqual(values[0],before)

    def test_redacted_selection_epoch_missing_changed_late_or_foreign_refuses(self):
        def impossible_epoch(binding,row,**changes):
            for path in (binding['selected_udp_pair'],row['selected_path']):
                path['transport_epoch'].update(changes)
            row['selected_path']['selected_candidate_pair_changes']=row['selected_path']['transport_epoch']['selection_count']
        mutations=[lambda b,r:b['selected_udp_pair'].pop('transport_epoch'),
            lambda b,r:r['selected_path'].pop('transport_epoch'),
            lambda b,r:b['selected_udp_pair']['transport_epoch'].update(transport_id='p0t1'),
            lambda b,r:b['selected_udp_pair']['transport_epoch'].update(armed_before_selection=False),
            lambda b,r:b['selected_udp_pair']['transport_epoch'].update(initial_selection_count=1),
            lambda b,r:b['selected_udp_pair']['transport_epoch'].update(selection_count=3),
            lambda b,r:b['selected_udp_pair']['transport_epoch'].update(state_version=4),
            lambda b,r:b['selected_udp_pair']['transport_epoch'].update(selection_count=True),
            lambda b,r:r['selected_path'].update(selected_candidate_pair_changes=2),
            lambda b,r:b['selected_udp_pair']['remote'].update(candidate_empty=False),
            lambda b,r:b['selected_udp_pair']['remote'].update(type='host'),
            lambda b,r:b['selected_udp_pair']['remote'].update(address='127.0.0.1'),
            lambda b,r:b['selected_udp_pair']['local'].update(port=4321),
            lambda b,r:impossible_epoch(b,r,selection_count=129,state_version=129),
            lambda b,r:impossible_epoch(b,r,selection_count=4,state_version=3)]
        for mutate in mutations:
            values,anchor,binding,actual=selection_boundary_facts();mutate(binding,actual)
            with self.subTest(mutation=mutate),self.assertRaises(ValueError):prepare_pair(*values[:4])

    def test_redacted_native_needs_numeric_real_rtp_and_rejects_available_epoch_contradiction(self):
        values,anchor,binding,actual=selection_boundary_facts()
        actual['selected_path']['remote']['address']=None
        with self.assertRaises(ValueError):prepare_pair(*values[:4])
        values,anchor,binding,actual=selection_boundary_facts()
        values[0]['decoded_media']['rtc_media_pipeline']['samples'][0]['peers'][0].pop(1)
        with self.assertRaisesRegex(WitnessError,'needs-one-real-rtp-endpoint'):prepare_pair(*values[:4])
        values,anchor,binding,actual=selection_boundary_facts()
        pipeline=values[0]['decoded_media']['rtc_media_pipeline']
        sample=copy.deepcopy(pipeline['samples'][0]);sample.update(requested_seconds=2.205,completed_seconds=2.210)
        sample['peers'][0][1]['selected_path']['transport_epoch'].update(selection_count=2)
        sample['peers'][0][1]['selected_path']['selected_candidate_pair_changes']=2
        pipeline['samples'].insert(1,sample)
        with self.assertRaisesRegex(WitnessError,'contradictory-regular-selection-epoch'):prepare_pair(*values[:4])

    def test_numeric_native_and_regular_brackets_preserve_optional_metadata_independence(self):
        values=facts()
        for n,sample in enumerate(values[0]['decoded_media']['rtc_media_pipeline']['samples']):
            for row in sample['peers'][0]:
                row['selected_path'].update(transport_epoch={'unproved':n},selected_candidate_pair_changes=n)
        result=prepare_pair(*values[:4]);self.assertEqual(result['native_binding_boundaries'],[])
        self.assertEqual(result['source_flow'],values[4]);self.assertEqual(result['target_flow'],values[5])
        values,anchor,binding=boundary_facts()
        values[0]['decoded_media']['rtc_media_pipeline']['samples'][0]['peers'][0][1]['selected_path'].update(
            transport_epoch={'unproved':True},selected_candidate_pair_changes=None)
        result=prepare_pair(*values[:4])
        self.assertNotIn('native_path_kind',result['native_binding_boundaries'][0])
        self.assertEqual(result['target_flow'],values[5])

def retirement_statistics_facts():
    values,anchor,binding=boundary_facts()
    cycle,route,pair=values[:3];pipeline=cycle['decoded_media']['rtc_media_pipeline']
    # Existing conservative native tail fails, so this positive cannot quietly
    # succeed through the old fallback. Its coordinate guard is not changed.
    binding['sync_source']['rtpTimestamp']=(pair['current']['presentation']['rtpTimestamp']-9000)%2**32
    actual=copy.deepcopy(pipeline['samples'][0]['peers'][0][1])
    at=1000+pair['current']['callback_seconds']*1000+20
    time_origin=1794700000000
    actual.update(native_track_identifier=actual['original_track']['track_id'],native_mid=actual['original_track']['mid'],
                  stats_timestamp_ms=time_origin+at+.2)
    record=dict(profile='connect/rtc-receiver-retirement-statistics/1',connection_id='receiver',peer_index=1,
                state='fulfilled',complete=True,reason=None,requested_at_ms=at,completed_at_ms=at+.5,time_origin_ms=time_origin,
                native_close_returned=True,native_closed=True,native_closed_at_ms=at+.1,rows=[actual])
    pipeline['binding_boundaries']['peers'][0]['receiver_retirement_statistics']=record
    return values,record


class ReceiverRetirementStatisticsControls(unittest.TestCase):
    def test_real_receiver_local_high_replaces_no_global_poll_and_keeps_other_owner_brackets(self):
        values,record=retirement_statistics_facts();cycle,route,pair,summaries=values[:4]
        before=copy.deepcopy(cycle['decoded_media']['rtc_media_pipeline']['samples'])
        result=prepare_pair(cycle,route,pair,summaries)
        self.assertEqual(result['native_binding_boundaries'],[])
        self.assertEqual(len(result['native_retirement_statistics']),1)
        proof=result['native_retirement_statistics'][0]
        self.assertEqual(proof['profile'],'connect/rtc-receiver-retirement-statistics/1')
        self.assertEqual(proof['stats_timestamp_ms'],record['rows'][0]['stats_timestamp_ms'])
        self.assertEqual(proof['direction'],'inbound-rtp');self.assertEqual(result['target_flow'],values[5])
        self.assertEqual(before,cycle['decoded_media']['rtc_media_pipeline']['samples'])
        source=StatsIndex(cycle).get('source','outbound-rtp',pair['first']['callback_seconds'],pair['current']['callback_seconds'],stream='source_video')
        self.assertNotIn('native_retirement_statistics',source)
        # Actual numeric IPv6 is copied exactly and validated by the same parser.
        for row in [cycle['decoded_media']['rtc_media_pipeline']['samples'][0]['peers'][0][1],record['rows'][0]]:
            row['selected_path']['local']['address']='::1';row['selected_path']['remote']['address']='::1'
        result=prepare_pair(cycle,route,pair,summaries);self.assertEqual(result['target_flow'][0],'::1')
        self.assertEqual(result['target_flow'][2],'::1')

    def test_receiver_high_does_not_benefit_public_head_outward_or_unsettled_and_partial_queries(self):
        for state in ['pending','rejected','not-requested']:
            values,record=retirement_statistics_facts();record['state']=state;record['complete']=False
            with self.assertRaises(WitnessError):prepare_pair(*values[:4])
        for change in ['partial','not-closed','not-returned','foreign-owner','ambiguous-record','missing-real-low']:
            values,record=retirement_statistics_facts();pipeline=values[0]['decoded_media']['rtc_media_pipeline']
            if change=='partial':record['complete']=False
            elif change=='not-closed':record['native_closed']=False
            elif change=='not-returned':record['native_close_returned']=False
            elif change=='foreign-owner':record['connection_id']='foreign'
            elif change=='ambiguous-record':pipeline['binding_boundaries']['peers']*=2
            elif change=='missing-real-low':pipeline['samples'][0]['peers'][0].pop(1)
            with self.assertRaises(WitnessError):prepare_pair(*values[:4])
        values,record=retirement_statistics_facts();pipeline=values[0]['decoded_media']['rtc_media_pipeline']
        # Remove the real source high. A receiver-only report cannot replace it.
        pipeline['samples'][1]['peers'][0].pop(0)
        with self.assertRaises(WitnessError):prepare_pair(*values[:4])
        values,anchor,binding=boundary_facts(public=True);cycle,route,pair=values[:3]
        _,record=retirement_statistics_facts()
        cycle['decoded_media']['rtc_media_pipeline']['binding_boundaries']['peers'][0]['receiver_retirement_statistics']=record
        result=prepare_pair(*values[:4]);self.assertEqual(result['native_retirement_statistics'],[])
        self.assertEqual(result['native_binding_boundaries'][0]['profile'],'connect/rtc-native-public-binding/1')

    def test_every_original_row_and_actual_native_report_contradiction_remains_fatal(self):
        for change in ['ssrc','codec','path','track','native-track','native-mid','competing','interior','early-missing']:
            values,record=retirement_statistics_facts();cycle,route,pair=values[:3];pipeline=cycle['decoded_media']['rtc_media_pipeline']
            low=pipeline['samples'][0]['peers'][0][1]
            if change=='ssrc':low['ssrc']+=1
            elif change=='codec':low['codec']['payload_type']+=1
            elif change=='path':low['selected_path']['remote']['port']+=1
            elif change=='track':low['original_track']['mid']='other-mid'
            elif change=='native-track':record['rows'][0]['native_track_identifier']='other'
            elif change=='native-mid':record['rows'][0]['native_mid']='other'
            elif change=='competing':record['rows']*=2
            elif change=='interior':
                sample=copy.deepcopy(pipeline['samples'][0]);sample.update(requested_seconds=2.205,completed_seconds=2.210)
                sample['peers'][0][1]['codec']['payload_type']+=1;pipeline['samples'].insert(1,sample)
            elif change=='early-missing':
                sample=copy.deepcopy(pipeline['samples'][0]);sample.update(requested_seconds=2.1,completed_seconds=2.11)
                sample['peers'][0].pop(1);pipeline['samples'].insert(1,sample)
            with self.assertRaises(WitnessError):prepare_pair(*values[:4])

    def test_native_time_not_request_or_late_settlement_must_cover_each_cached_pair(self):
        for change in ['old-sample','future-sample','wrong-clock','close-before-request','early-request']:
            values,record=retirement_statistics_facts();last=values[2]['current']['callback_seconds'];origin=1000
            if change=='old-sample':record['rows'][0]['stats_timestamp_ms']=record['time_origin_ms']+origin+last*1000-.1
            elif change=='future-sample':record['rows'][0]['stats_timestamp_ms']=record['time_origin_ms']+record['completed_at_ms']+.1
            elif change=='wrong-clock':record['time_origin_ms']+=100000
            elif change=='close-before-request':record['native_closed_at_ms']=record['requested_at_ms']-.1
            elif change=='early-request':record['requested_at_ms']=origin+last*1000-.1
            with self.assertRaises(WitnessError):prepare_pair(*values[:4])
        values,record=retirement_statistics_facts();cycle,route,pair,summaries=values[:4]
        first,last=pair['first']['callback_seconds'],pair['current']['callback_seconds']
        record['rows'][0]['stats_timestamp_ms']=record['time_origin_ms']+1000+last*1000
        index=StatsIndex(cycle);index.get('receiver','inbound-rtp',first,last,stream='sink_video',track='target-track',coordinates=[200000,201530])
        with self.assertRaisesRegex(WitnessError,'native-time-does-not-cover-pair'):
            index.get('receiver','inbound-rtp',first,last+.01,stream='sink_video',track='target-track',coordinates=[200000,201530])
        # The real owner-local high may be the ONLY endpoint after all regular
        # polling stops. Its cache window then has hi==len(original samples).
        pipeline=cycle['decoded_media']['rtc_media_pipeline'];pipeline['samples']=pipeline['samples'][:1]
        index=StatsIndex(cycle);first_value=index.get('receiver','inbound-rtp',first,last,stream='sink_video',track='target-track',coordinates=[200000,201530])
        cached=index.get('receiver','inbound-rtp',first+.001,last,stream='sink_video',track='target-track',coordinates=[200000,201530])
        self.assertEqual(first_value,cached)
        with self.assertRaisesRegex(WitnessError,'native-time-does-not-cover-pair'):
            index.get('receiver','inbound-rtp',first,last+.01,stream='sink_video',track='target-track',coordinates=[200000,201530])
        with self.assertRaisesRegex(WitnessError,'owner-statistics-too-distant'):
            index.get('receiver','inbound-rtp',first,last+.03,stream='sink_video',track='target-track',coordinates=[200000,201530])


if __name__=='__main__':unittest.main()
