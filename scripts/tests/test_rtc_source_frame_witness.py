"""Pure source-frame witness/closed-file controls, not signed network proof."""
import copy
import hashlib
import json
import os
import struct
import sys
import tempfile
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.rtc_source_frame_witness import (PROFILE, WitnessError, capture_headers, delta,
    epoch_for_pair, finalize, prepare_pair, prove_headers, read_json, report_bound)


def facts():
    source_flow = ('127.0.0.1', 50000, '127.0.0.1', 40000, 11, 120)
    target_flow = ('127.0.0.1', 40001, '127.0.0.1', 50001, 22, 120)
    epoch = dict(ordinal=0, source_connection='source', source_logical='source_video', source_transport='source_transport',
        source_generation=3, source_ssrc=11, target_logical='sink_video', target_transport='sink_transport',
        target_generation=2, target_connection_generation=1, target_ssrc=22, target_pt=120,
        input_sequence=1, input_timestamp=1000, output_sequence=101, output_timestamp=2000,
        sequence_offset=100, timestamp_offset=1000)
    summary = dict(profile='connect/rtc-original-video-mapping/1', process=4242, timeline=1,
                   original_timeline_retired=True, complete=True, omitted=0, invalid=0, epochs=[epoch])
    def point(counter, timestamp):
        return dict(owner='receiver', row_index=counter, callback_seconds=10+counter/1000,
            source_frame_identity=dict(status='identified', ordinal=7, observed_source_id=0),
            presentation=dict(profile='connect/rtc-video-presentation/1', connection_id='receiver',
                stream_id='sink_video', track_id='target-track', element_id=2,
                presentedFrames=counter, rtpTimestamp=timestamp))
    pair = dict(first=point(1, 200000), current=point(2, 201530))
    def stat(owner, direction, ssrc, stream, track, local, remote):
        return dict(connection_id=owner, type=direction, kind='video', ssrc=ssrc,
            original_track=dict(connection_id=owner, stream_id=stream, track_id=track),
            codec=dict(mime_type='video/VP8', payload_type=120, clock_rate=90000),
            selected_path=dict(nominated=True, succeeded=True,
                local=dict(address='127.0.0.1',port=local,protocol='udp'),
                remote=dict(address='127.0.0.1',port=remote,protocol='udp'),srtp_cipher=None))
    peers=[[stat('source','outbound-rtp',11,'source_video','source-track',50000,40000),
            stat('receiver','inbound-rtp',22,'sink_video','target-track',50001,40001)]]
    candidate=dict(profile=PROFILE, collection_passed=True, non_duplicate_passed=True,
                   complete=True, initial_ordinary_receiver=True, omitted=0, pairs=[pair])
    route=dict(route=1,source_route_index=0,passed=False,invalid=[],audio=dict(passed=True),sync=dict(status='passed'),
        video=dict(passed=False,duplicate_frames=1,changing_frames_per_second=[30]*60,
                   decoded_frames_per_second=[30]*60,original_source_frame_witness=candidate))
    cycle=dict(cycle=dict(index=1,workload_id='original'),passed=False,collection_passed=True,
        source_startup=dict(sources=[dict(index=1,attachments=[dict(connection_id='receiver')])]),
        programme=dict(moves=[dict(source_index=0)],sources=dict(sources=[dict(index=0,attachments=[dict(connection_id='source')])])),
        decoded_media=dict(passed=False,routes=[route],rtc_media_pipeline=dict(samples=[
            dict(requested_seconds=9,completed_seconds=9.1,peers=copy.deepcopy(peers)),
            dict(requested_seconds=11,completed_seconds=11.1,peers=copy.deepcopy(peers))])),
        failure=dict(stage='decoded-media-quality',name='ProfileBudgetsFailed'))
    rows=[]
    for i,ts in enumerate([198000,199000,200530,202000]):
        rows.append(dict(flow=source_flow,seq=2+i,ts=ts,marker=True,body_bytes=100,
                         unix_ns=1000000000+i*20000000,record=2*i+1))
        rows.append(dict(flow=target_flow,seq=102+i,ts=ts+1000,marker=True,body_bytes=100,
                         unix_ns=1005000000+i*20000000,record=2*i+2))
    return cycle,route,pair,[summary],source_flow,target_flow,epoch,rows


def pcap(path,rows):
    body=bytearray(struct.pack('<IHHIIII',0xa1b2c3d4,2,4,0,0,262144,0))
    for row in rows:
        flow=row['flow'];rtp=struct.pack('!BBHII',0x80,flow[5]|0x80,row['seq'],row['ts'],flow[4])+bytes(row['body_bytes'])
        udp=struct.pack('!HHHH',flow[1],flow[3],len(rtp)+8,0)+rtp
        ip=bytes([0x45,0])+struct.pack('!H',20+len(udp))+bytes(4)+bytes([64,17])+bytes(2)+bytes([127,0,0,1])*2
        packet=bytes([2,0,0,0])+ip+udp;ns=row['unix_ns']
        body.extend(struct.pack('<IIII',ns//1000000000,(ns%1000000000)//1000,len(packet),len(packet)));body.extend(packet)
    path.write_bytes(body)
    return dict(observed=True,original_joined=True,exit_code=0,forced=False,kernel_dropped_packets=0,
                pcap_sha256=hashlib.sha256(body).hexdigest(),pcap_bytes=len(body),original_packets_captured=len(rows))


class SourceFrameWitness(unittest.TestCase):
    def test_distinct_original_frames_same_draw_use_actual_pipeline_without_changing_raw_fps(self):
        cycle,route,pair,maps,sf,tf,epoch,rows=facts()
        proof=prepare_pair(cycle,route,pair,maps)
        self.assertEqual(proof['input_ts'],[199000,200530])
        self.assertEqual(prove_headers(rows,sf,tf,epoch,proof['input_ts'],proof['output_ts'])['matched_packets'],4)
        with tempfile.TemporaryDirectory() as tmp:
            folder=Path(tmp);(folder/'cycles/cycle-1').mkdir(parents=True)
            raw=json.dumps(cycle).encode();(folder/'cycles/cycle-1/cycle-observation.json').write_bytes(raw)
            log=folder/'server.log';log.write_text('DEBUG RTC original video mapping witness witness='+json.dumps(maps[0],separators=(',',':'))+'\n')
            capture=pcap(folder/'original.pcap',rows)
            browser=dict(source_revision='original-source',transport='firefox-wss-webrtc',source_frame_witness_profile=PROFILE,collection_passed=True,passed=False,
                resource_cycles=[dict(index=1,workload_id='original',observation_sha256=hashlib.sha256(raw).hexdigest())],
                cycles=[dict(index=1,passed=False,failure=cycle['failure'])],failure=cycle['failure'])
            report=dict(transport='firefox-wss-webrtc',stage='quality',source_revision='original-source',browser_exit_code=0,
                browser_owner_joined=True,browser_observation=browser,browser_observation_sha256='a'*64,packet_capture=capture,
                servers=[dict(name='materialized-server',original_pid=4242,original_joined=True,original_exit_code=0,original_process_group_joined=True)])
            final,cycles=finalize(report,folder,log,folder/'original.pcap')
            self.assertTrue(final['passed']);self.assertFalse(browser['passed']);self.assertFalse(cycle['passed'])
            self.assertEqual((folder/'cycles/cycle-1/cycle-observation.json').read_bytes(),raw)
            video=cycles[0]['decoded_media']['routes'][0]['video'];self.assertEqual(video['duplicate_frames'],0)
            self.assertEqual(video['raw_draw_duplicate_frames'],1)
            self.assertEqual(video['changing_frames_per_second'],route['video']['changing_frames_per_second'])
            self.assertEqual(video['decoded_frames_per_second'],route['video']['decoded_frames_per_second'])

    def test_original_replay_retag_and_changed_footer_do_not_create_a_new_original_frame(self):
        _,_,_,_,sf,tf,e,rows=facts();rows=copy.deepcopy(rows)
        rows[4]['ts']=rows[2]['ts']
        with self.assertRaises(WitnessError):prove_headers(rows,sf,tf,e,[199000,200530],[200000,201530])
        with self.assertRaises(WitnessError):prove_headers(rows,sf,tf,e,[199000,199000],[200000,201530])

    def test_missing_ambiguous_omitted_and_invalid_generations_never_resolve(self):
        c,r,p,m,*_=facts();receiver=prepare_pair(c,r,p,m)
        for maps in [[],m+m,[{**m[0],'complete':False,'omitted':1}],
                     [{**m[0],'epochs':[{**m[0]['epochs'][0],'source_generation':0}]}]]:
            with self.assertRaises(WitnessError):prepare_pair(c,r,p,maps)
        self.assertEqual(receiver['epoch']['source_generation'],3)

    def test_source_or_current_target_binding_switch_inside_window_refuses(self):
        c,r,p,m,*_=facts()
        for key in ['source_generation','target_generation','target_connection_generation','source_ssrc','target_transport']:
            changed=copy.deepcopy(m);next_epoch=copy.deepcopy(changed[0]['epochs'][0])
            next_epoch.update(ordinal=1,output_timestamp=201000,input_timestamp=200000)
            next_epoch[key]='successor_transport' if key=='target_transport' else next_epoch[key]+1
            changed[0]['epochs'].append(next_epoch)
            with self.assertRaises(WitnessError):prepare_pair(c,r,p,changed)

    def test_actual_header_duplicates_gaps_or_wrong_flow_cannot_be_fitted_away(self):
        _,_,_,_,sf,tf,e,rows=facts()
        for altered in [rows[:-1],rows+[dict(rows[1])],
                        [{**v,'body_bytes':101} if i==1 else v for i,v in enumerate(rows)],
                        [{**v,'flow':('127.0.0.1',1,*v['flow'][2:])} if i==1 else v for i,v in enumerate(rows)]]:
            with self.assertRaises(WitnessError):prove_headers(altered,sf,tf,e,[199000,200530],[200000,201530])
        self.assertEqual(delta(20,0xfffffff0),36)
        with self.assertRaises(WitnessError):delta(0,0x80000000)

    def test_original_closed_capture_hash_drops_and_regular_fd_are_mandatory(self):
        _,_,_,_,sf,tf,_,rows=facts()
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/'original.pcap';capture=pcap(path,rows);windows=[(sf,199000,200530),(tf,200000,201530)]
            self.assertEqual(len(capture_headers(path,capture,windows)),8)
            for field in ['observed','original_joined']:
                with self.assertRaises(WitnessError):capture_headers(path,{**capture,field:False},windows)
            with self.assertRaises(WitnessError):capture_headers(path,{**capture,'kernel_dropped_packets':1},windows)
            with self.assertRaises(WitnessError):capture_headers(path,{**capture,'pcap_sha256':'0'*64},windows)
            fifo=Path(tmp)/'fifo';os.mkfifo(fifo)
            with self.assertRaises(WitnessError):read_json(fifo)

    def test_cross_owner_outside_window_unbound_track_and_unknown_stats_remain_failures(self):
        c,r,p,m,*_=facts()
        for change in ['owner','outside','track','stats']:
            cycle,pair=copy.deepcopy(c),copy.deepcopy(p)
            if change=='owner':pair['current']['owner']='replacement'
            elif change=='outside':pair['first']['callback_seconds']=.9
            elif change=='track':pair['current']['presentation']['track_id']='other'
            else:cycle['decoded_media']['rtc_media_pipeline']['samples']=[]
            with self.assertRaises((WitnessError,KeyError)):prepare_pair(cycle,r,pair,m)

    def test_historical_cli_failure_wrong_transport_or_incomplete_collection_never_promotes(self):
        base=dict(transport='firefox-wss-webrtc',stage='quality',browser_exit_code=1,browser_owner_joined=True,
                  browser_observation=dict(passed=False),servers=[])
        for value in [base,{**base,'browser_exit_code':0},
                      {**base,'transport':'chrome-wss-webrtc','browser_exit_code':0}]:
            with self.assertRaises(WitnessError):finalize(value,Path('/not-read'),Path('/not-read'),Path('/not-read'))


    def test_new_profile_markers_cannot_fall_back_to_legacy_after_optin_is_removed(self):
        value=dict(profile=PROFILE,complete=True,passed=True,source_revision='original',pcap_sha256='b'*64,raw_browser_sha256='a'*64)
        report=dict(transport='firefox-wss-webrtc',stage='quality',browser_exit_code=0,browser_owner_joined=True,
            source_inputs_unchanged=True,sdk_dist_inputs_unchanged=True,executable_unchanged=True,source_revision='original',
            diagnostic_options=dict(rtc_source_frame_witness=True),packet_capture=dict(pcap_sha256='b'*64),
            raw_browser_observation_sha256='a'*64,source_frame_adjudication=value,
            browser_observation=dict(passed=True,source_frame_witness_profile=PROFILE,original_source_frame_adjudication=value))
        self.assertTrue(report_bound(report))
        for flag in [None,False,'true']:
            changed=copy.deepcopy(report)
            if flag is None:changed.pop('diagnostic_options')
            else:changed['diagnostic_options']['rtc_source_frame_witness']=flag
            self.assertFalse(report_bound(changed))
        legacy=dict(browser_observation=dict(decoded_media=dict(routes=[])))
        self.assertTrue(report_bound(legacy))
        legacy['browser_observation']['decoded_media']['routes']=[dict(video=dict(source_frame_profile=PROFILE,duplicate_frames=0))]
        self.assertFalse(report_bound(legacy))


if __name__=='__main__':
    unittest.main()
