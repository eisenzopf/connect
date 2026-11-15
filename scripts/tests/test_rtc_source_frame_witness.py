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
    prove_headers, read_json, report_bound, decode_census, StatsIndex)


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
    epoch = dict(ordinal=0, source_connection=source, source_logical='source_video'+suffix, source_transport='source_transport'+suffix,
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
    peers=[[stat(source,'outbound-rtp',sf[4],epoch['source_logical'],'source-track',sf[1],sf[3]),
            stat(receiver,'inbound-rtp',tf[4],epoch['target_logical'],'target-track',tf[3],tf[1])]]
    cycle=dict(cycle=dict(index=index,workload_id='original'+suffix),passed=False,collection_passed=True,
        source_startup=dict(sources=[dict(index=1,attachments=[dict(connection_id=receiver)]),dict(index=0,attachments=[dict(connection_id=source)])]),
        programme=dict(moves=[],sources=dict(sources=[dict(index=0,attachments=[dict(connection_id=source)])])),
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
    return cycle,route,pair,[summary],sf,tf,epoch,rows


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
    allrows=[];maps=[];archives=[]
    for cycle,_,_,summaries,*rest in sets:
        index=cycle['cycle']['index'];leaf=folder/'cycles'/('cycle-'+str(index));leaf.mkdir(parents=True)
        raw=json.dumps(cycle).encode();(leaf/'cycle-observation.json').write_bytes(raw)
        archives.append(dict(index=index,workload_id=cycle['cycle']['workload_id'],observation_sha256=hashlib.sha256(raw).hexdigest()))
        maps.extend(summaries);allrows.extend(rest[-1])
    log=folder/'server.log';logged=(b'unrelated audio facts '+b'x'*(1400*1024)+b'\n')
    for value in maps:logged+=b'\x1b[34mDEBUG\x1b[0m RTC original video mapping witness \x1b[3mwitness\x1b[0m\x1b[2m=\x1b[0m'+json.dumps(value,separators=(',',':')).encode()+b'\n'
    log.write_bytes(logged);capture=pcap(folder/'original.pcap',allrows)
    browser=dict(source_revision='original-source',transport='firefox-wss-webrtc',source_frame_witness_profile=PROFILE,collection_passed=True,passed=False,
        resource_cycles=archives,cycles=[dict(index=value['index'],passed=False) for value in archives])
    report=dict(transport='firefox-wss-webrtc',stage='quality',source_revision='original-source',browser_exit_code=0,browser_owner_joined=True,
        browser_observation=browser,browser_observation_sha256='a'*64,packet_capture=capture,
        servers=[dict(name='materialized-server',original_pid=4242,original_joined=True,original_exit_code=0,original_process_group_joined=True)])
    return report,log,logged


class SourceFrameWitness(unittest.TestCase):
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
            log=Path(tmp)/'server.log';message=b'RTC original video mapping witness';valid=message+b' witness='+json.dumps(m[0],separators=(',',':')).encode();noise=b'unrelated '+b'x'*(1400*1024)+b'\n'
            crossing=b'x'*(64*1024-len(message)//2)+valid;log.write_bytes(noise+crossing)
            parsed,digest=mapping_summaries(log,4242);self.assertEqual(parsed,m);self.assertEqual(digest,hashlib.sha256(noise+crossing).hexdigest())
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

if __name__=='__main__':unittest.main()
