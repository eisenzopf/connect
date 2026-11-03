"""Whole-window custody and actual independent codec calibration; no transport."""
import base64
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'scripts'))
from lib.package_environment import fixture_environment
from lib.transport_av import opus_packets, ivf_frames, decode_completed, marker_profile
from lib.transport_moving_av import audio_decode, video_decode, timeline, rows, summarize, marker_alignment
from lib.transport_movement_profile import MOVE_START_OFFSETS_MS, MOVES_PER_SESSION, TOTAL_MOVES, MOVEMENT_PROFILE, MOVEMENT_PROFILE_SHA256


class MovingCustody(unittest.TestCase):
    def setUp(self):
        (ROOT/'.local').mkdir(exist_ok=True)
        self.temp=tempfile.TemporaryDirectory(dir=ROOT/'.local',prefix='moving-codec-')
        self.directory=Path(self.temp.name);self.metadata={'original_capture_sha256':[]}
        peers=[3,0,1,2,5,4];origin=1800000000000000
        streams=[{'audio':f'audio-{i}','video':f'video-{i}','data':f'data-{i}'} for i in range(6)]
        bundle=dict(format='connect/physical-movement-segments/1',original_move_records_bound=TOTAL_MOVES,reliable_data_exchanges_bound=TOTAL_MOVES,
            movement_profile=MOVEMENT_PROFILE,movement_profile_sha256=MOVEMENT_PROFILE_SHA256,scheduled_start_offsets_ms=list(MOVE_START_OFFSETS_MS),
            program_start_unix_us=origin,original_connection_ids=[f'original-{i}' for i in range(6)],original_stream_ids=streams,segments=[])
        def packet(at,stream=None):
            return dict(elapsed_ms=at,timestamp=int(at*48),ssrc=7,payload='AQ==',**({'stream_id':stream,'key':True} if stream else {}))
        for receiver,peer in enumerate(peers):
            captured=dict(format='connect/native-av-received/1',index=receiver,peer_index=peer,connection_id=f'original-{receiver}',
                session_index=int(receiver>=4),generation=1,original_audio_stream_id=streams[receiver]['audio'],initial_video_stream_id=streams[peer]['video'],
                unix_start_ms=origin//1000,audio=[packet(at) for at in [1000,2050,4050,60000]],
                video=[packet(at,streams[peer]['video']) for at in [1000,2050,4050,60000]],clock_reports={'audio':[],'video':[]})
            path=self.directory/f'received-{receiver}.json';path.write_text(json.dumps(captured));path.chmod(0o600)
            self.metadata['original_capture_sha256'].append(hashlib.sha256(path.read_bytes()).hexdigest())
        for receiver in (0,4):
            peer=peers[receiver]
            for ordinal in range(MOVES_PER_SESSION):
                start=MOVE_START_OFFSETS_MS[ordinal]+1000
                arrivals=[start+100, MOVE_START_OFFSETS_MS[ordinal+1]+1100 if ordinal+1 < MOVES_PER_SESSION else 61000]
                capture=dict(format='connect/physical-av-segment/1',program_start_unix_us=origin,root_completed_observed_programme_us=start*1000,
                    audio=[dict(ssrc=7,rows=[packet(at, f'physical-audio-{receiver}-{ordinal}') for at in arrivals])],
                    video=[dict(ssrc=7,rows=[packet(at,streams[peer]['video']) for at in arrivals])],clock_reports={'audio':[],'video':[]})
                bundle['segments'].append(dict(source_index=receiver,ordinal=ordinal,connection_id=f'candidate-{receiver}-{ordinal}',
                    scheduled_start_ms=MOVE_START_OFFSETS_MS[ordinal],actual_start_ms=MOVE_START_OFFSETS_MS[ordinal]+100,
                    published_stream_ids=dict(audio=f'physical-audio-{receiver}-{ordinal}',video=f'physical-video-{receiver}-{ordinal}'),capture=capture,
                    reliable_data=dict(original_data_messages_bound=2)))
        self.bundle=bundle;self.save()

    def save(self):
        path=self.directory/'movement-segments.json';path.write_text(json.dumps(self.bundle));path.chmod(0o600)
        self.metadata['sha256']=hashlib.sha256(path.read_bytes()).hexdigest()

    def tearDown(self):self.temp.cleanup()

    def test_original_and_four_scheduled_physical_captures_partition_owner_intervals_without_counting_late_rows(self):
        bundle,routes=timeline(self.directory,self.metadata)
        self.assertEqual(len(bundle['segments']),4);self.assertEqual(len(routes),6)
        for index in (0,4):
            audio=[h for h in routes[index]['histories'] if h['medium']=='audio']
            self.assertEqual(len(audio),3)
            self.assertEqual(sum(len(h['selected_indices']) for h in audio),5)
            self.assertEqual(audio[0]['rows'][0]['elapsed_ms'],1000,'actual warmup initializes only its own decoder')
            self.assertEqual(audio[0]['selected_indices'],[1,2])
            self.assertEqual(audio[1]['rows'][0]['elapsed_ms'],16100)
            self.assertEqual(audio[1]['selected_indices'],[0])
            self.assertGreater(routes[index]['excluded_rows']['audio'],0)

    def test_missing_roots_wrong_original_bindings_and_changed_capture_bytes_fail_before_decoder(self):
        self.bundle['segments'][1]['capture']['root_completed_observed_programme_us']=16000000
        self.save()
        with self.assertRaises(ValueError):timeline(self.directory,self.metadata)
        self.bundle['segments'][1]['capture']['root_completed_observed_programme_us']=41000000
        self.save();path=self.directory/'received-0.json';path.write_bytes(path.read_bytes()+b' ')
        with self.assertRaisesRegex(ValueError,'digest'):timeline(self.directory,self.metadata)

    def test_quarantined_precompletion_audio_cannot_initialize_an_independent_decoder(self):
        self.bundle['segments'][0]['capture']['audio'][0]['rows'][0]['elapsed_ms']=15999
        self.save()
        with self.assertRaisesRegex(ValueError,'pre-completion'):timeline(self.directory,self.metadata)

    def test_missing_data_receipts_do_not_become_full_custody(self):
        self.bundle['reliable_data_exchanges_bound']=3;self.save()
        with self.assertRaises(ValueError):timeline(self.directory,self.metadata)

    def test_stress_count_profile_substitution_and_early_ownership_refuse_before_decoder(self):
        import copy
        original=copy.deepcopy(self.bundle)
        for key,value in [('original_move_records_bound',20),('movement_profile','historical-stress'),
                ('movement_profile_sha256','f'*64),('scheduled_start_offsets_ms',[0,1000])]:
            with self.subTest(field=key):
                self.bundle=copy.deepcopy(original);self.bundle[key]=value;self.save()
                with self.assertRaises(ValueError):timeline(self.directory,self.metadata)
        self.bundle=copy.deepcopy(original)
        self.bundle['segments'][0]['actual_start_ms']=14999;self.save()
        with self.assertRaisesRegex(ValueError,'ownership boundary'):timeline(self.directory,self.metadata)

    def test_same_ssrc_different_streams_remain_separate_and_repeated_clocks_fail(self):
        sample=dict(elapsed_ms=2000,timestamp=3000,ssrc=7,payload='AQ==',key=True,stream_id='first')
        value=rows([sample,dict(sample,elapsed_ms=2010,stream_id='second')],'video',{'first','second'})
        self.assertEqual(len(value),2)
        with self.assertRaisesRegex(ValueError,'repeated'):
            rows([sample,dict(sample,elapsed_ms=2010)],'video',{'first'})

    def test_moving_workload_can_never_fall_back_to_original_only_decode(self):
        observed=dict(device_movement={},native_av=dict(original_receive_tasks_joined=True),native_movement_capture={'sha256':'original'})
        with patch('lib.transport_moving_av.verify',return_value={'passed':False}) as moving,patch('lib.transport_av.verify') as original:
            self.assertEqual(decode_completed(observed,self.directory,{}),{'passed':False})
            moving.assert_called_once_with(self.directory,{},observed['native_movement_capture']);original.assert_not_called()

    def test_empty_decodes_fail_frozen_60_second_coverage_frame_rate_and_marker_limits(self):
        result=summarize(dict(index=0,peer_index=3,excluded_rows={},received_clocks={'audio':1,'video':1}),[],[])
        self.assertFalse(result['passed']);self.assertEqual(len(result['video']['frames_by_observation_second']),60)
        self.assertEqual(result['independently_decoded_receive_marker_alignment']['limits'],{'median_abs_skew_ms':80,'p95_abs_skew_ms':120})

    def test_onset_detector_cannot_rearm_across_separate_codec_histories(self):
        old={'measurements':[dict(level=.03,row=dict(elapsed_ms=2000+i*20)) for i in range(12)]}
        new={'measurements':[dict(level=.7,row=dict(elapsed_ms=2240))]}
        observed=marker_alignment([old,new],[])
        self.assertEqual(observed['audio_observation']['edges_seconds'],[])
        self.assertEqual(observed['audio_observation']['unproven_onsets'],1)
        self.assertFalse(observed['audio_observation']['decoder_and_onset_state_crossed_history_boundary'])

    def test_duplicate_programme_frames_across_hops_cannot_inflate_coverage(self):
        repeated=dict(row=dict(elapsed_ms=2000,timestamp=9000),level=0,hash='original',wrong_source=0)
        videos=[dict(measurements=[repeated,dict(repeated,row=dict(elapsed_ms=2010,timestamp=9000))],undecodable_frames=0)]
        with self.assertRaisesRegex(ValueError,'counted twice'):
            summarize(dict(index=0,peer_index=3,excluded_rows={},received_clocks={'audio':1,'video':1}),[],videos)


@unittest.skipUnless(shutil.which('ffmpeg'),'Independent FFmpeg prerequisite')
class IndependentMovingCodecs(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.directory=Path(self.temp.name)
    def tearDown(self):self.temp.cleanup()

    def test_actual_opus_and_vp8_decode_each_history_with_original_packet_and_pts_mapping(self):
        audio=self.directory/'known.ogg';video=self.directory/'known.ivf'
        common=['ffmpeg','-nostdin','-hide_banner','-loglevel','error']
        subprocess.run(common+['-f','lavfi','-i','sine=frequency=440:sample_rate=48000:duration=1',
            '-ac','1','-c:a','libopus','-b:a','32k','-vbr','off','-frame_duration','20',str(audio)],check=True,timeout=15,env=fixture_environment())
        subprocess.run(common+['-f','lavfi','-i','testsrc2=size=640x360:rate=30:duration=1',
            '-vf','drawbox=x=0:y=0:w=80:h=80:color=red:t=fill,drawbox=x=96:y=0:w=80:h=80:color=black:t=fill',
            '-an','-c:v','libvpx','-threads','1','-deadline','realtime','-cpu-used','8','-b:v','500k','-g','30',str(video)],check=True,timeout=15,env=fixture_environment())
        audio_rows=[dict(elapsed_ms=2000+i*20,timestamp=i*960,ssrc=7,payload=base64.b64encode(packet).decode()) for i,packet in enumerate(opus_packets(audio.read_bytes())[:50])]
        ah=dict(rows=audio_rows,selected_indices=list(range(len(audio_rows))))
        a=audio_decode(self.directory,'audio',ah,440)
        self.assertEqual(a['measured_samples'],48000);self.assertEqual(a['bad_identity_blocks'],0)
        # ivf_frames intentionally requires the full62s source, so this local
        # short calibration retains each encoder frame directly from its IVF.
        raw=video.read_bytes();at=32;video_rows=[]
        import struct
        while at<len(raw):
            length,pts=struct.unpack_from('<IQ',raw,at);at+=12;body=raw[at:at+length];at+=length
            video_rows.append(dict(elapsed_ms=2000+pts*1000/30,timestamp=pts*3000,ssrc=8,key=not body[0]&1,payload=base64.b64encode(body).decode()))
        v=video_decode(self.directory,'video',dict(rows=video_rows,selected_indices=list(range(30))),0)
        self.assertEqual(len(v['measurements']),30);self.assertEqual(v['undecodable_frames'],0)
        self.assertEqual(sum(m['wrong_source'] for m in v['measurements']),0)
        no_key=video_decode(self.directory,'unkeyed',dict(rows=video_rows[1:],selected_indices=list(range(29))),0)
        self.assertEqual(no_key['measurements'],[]);self.assertEqual(no_key['undecodable_frames'],29)
        self.assertFalse(marker_alignment([a],[v])['status']=='passed','one second cannot satisfy42 paired markers')


if __name__=='__main__':unittest.main()
