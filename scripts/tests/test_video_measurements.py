"""Decoder/audio marker qualification must reject static content and bad skew."""
import array
import asyncio
import hashlib
import importlib.util
import math
import pathlib
import subprocess
import struct
import sys
import os
import json
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('media_processes', pathlib.Path(__file__).parents[1] / 'video/native-processes.py')
MEDIA = importlib.util.module_from_spec(SPEC); SPEC.loader.exec_module(MEDIA)
RTC_SPEC = importlib.util.spec_from_file_location('rtc_impairment', pathlib.Path(__file__).parents[1] / 'video/webrtc-impairment.py')
RTC = importlib.util.module_from_spec(RTC_SPEC); RTC_SPEC.loader.exec_module(RTC)
sys.path.insert(0, str(pathlib.Path(__file__).parents[1]))
FIXTURE_SPEC = importlib.util.spec_from_file_location('video_fixture_protocol', pathlib.Path(__file__).parents[1] / 'video/fixture_protocol.py')
FIXTURE = importlib.util.module_from_spec(FIXTURE_SPEC); FIXTURE_SPEC.loader.exec_module(FIXTURE)


class MediaMeasurements(unittest.TestCase):
    def write_ivf(self, directory, stamps):
        target = pathlib.Path(directory) / 'received.ivf'
        header = b'DKIF\x00\x00\x20\x00VP80' + struct.pack('<HHIIII', 640, 360, 90000, 1, 0, 0)
        target.write_bytes(header + b''.join(struct.pack('<IQ', 1, stamp) + b'x' for stamp in stamps))
        return target

    def frame_metadata(self, stamps, hashes):
        return '#tb 0: 1/90000\n' + '\n'.join(f'0,{stamp},{stamp},3000,64,{value}' for stamp, value in zip(stamps, hashes))
    def diagnostic_fixture(self):
        return {'scope': 'server-process', 'queue_depth_observation': 'sampled-at-enqueue-and-pump-poll',
            'forwarding_boundary': 'adapter-frame-publication-to-core-destination-queue-handoff',
            'queues': [{'stage': 'quic-video-egress', 'kind': 'video', 'unit': 'frame-packets',
                'observations': 200, 'capacity_packets': 64, 'sampled_high_water_packets': 8,
                'max_forwarded_frame_age_ms': 100, 'frame_age_boundary': 'dequeue-before-handoff-or-write'}],
            'forwarding': [{'stage': 'core-video-fanout', 'samples': 200, 'p50_us': 127, 'p95_us': 511,
                'max_us': 400, 'percentile_method': 'log2-bucket-upper-bound'}],
            'quic': {'samples': 200, 'accepted_datagrams': 200, 'video_budget_observations': 200,
                'min_video_rtp_budget_bytes': 1100, 'max_video_rtp_budget_bytes': 1100}}

    def test_forwarding_diagnostics_require_real_samples_and_bounded_queues(self):
        snapshot = self.diagnostic_fixture()
        self.assertEqual(MEDIA.forwarding_diagnostics(snapshot, 'native-native')['status'], 'measured')
        snapshot['forwarding'][0]['samples'] = 0
        self.assertEqual(MEDIA.forwarding_diagnostics(snapshot, 'native-native')['status'], 'failed')
        snapshot = self.diagnostic_fixture(); snapshot['queues'][0]['capacity_packets'] = 65
        self.assertEqual(MEDIA.forwarding_diagnostics(snapshot, 'native-native')['status'], 'failed')
        snapshot = self.diagnostic_fixture(); snapshot['queues'][0]['max_forwarded_frame_age_ms'] = None
        self.assertEqual(MEDIA.forwarding_diagnostics(snapshot, 'native-native')['required_application_video_age_unobserved_stages'], ['quic-video-egress'])
        client = self.diagnostic_fixture()
        client.update(scope='native-session-handle', queue_depth_observation='sampled-at-sdk-send-receive-and-pump-poll',
            ingress_age_boundary='client-adapter-local-receipt-to-sdk-receive', egress_age_boundary='dequeue-before-handoff-or-write')
        client['queues'] = [dict(client['queues'][0], stage=stage, capacity_packets=capacity, sampled_high_water_packets=1)
            for stage, capacity in [('quic-audio-ingress', 10), ('quic-audio-egress', 10), ('quic-video-ingress', 64), ('quic-video-egress', 64)]]
        client['quic']['send_buffer'] = {'observations': 2, 'capacity_bytes': 76032,
            'minimum_available_bytes': 100, 'maximum_available_bytes': 76032, 'sampled_high_water_bytes': 75932}
        self.assertEqual(MEDIA.native_sdk_diagnostics(client)['status'], 'measured')
        client['queues'].pop()
        self.assertEqual(MEDIA.native_sdk_diagnostics(client)['status'], 'failed')

    def test_forwarding_diagnostics_preserve_typed_outcomes_without_invented_drop_targets(self):
        self.assertEqual(MEDIA.forwarding_diagnostics(None, 'native-native')['status'], 'unobserved')
        snapshot = self.diagnostic_fixture(); snapshot['quic']['too_large'] = 1
        observed = MEDIA.forwarding_diagnostics(snapshot, 'native-native')
        self.assertEqual(observed['status'], 'measured')
        self.assertEqual(observed['snapshot']['quic']['too_large'], 1)
        snapshot['quic']['video_packetizer_discards'] = 2
        self.assertEqual(MEDIA.forwarding_diagnostics(snapshot, 'native-native')['status'], 'measured')
        snapshot = self.diagnostic_fixture(); snapshot['quic']['min_video_rtp_budget_bytes'] = None
        self.assertEqual(MEDIA.forwarding_diagnostics(snapshot, 'native-native')['status'], 'failed')
        snapshot['forwarding'][0]['stage'] = 'core-video-bridge'
        snapshot['queues'] = [dict(snapshot['queues'][0], stage=stage)
            for stage in ['webrtc-video-ingress', 'webrtc-video-egress', 'webrtc-video-reorder']]
        snapshot['quic']['video_budget_observations'] = 0
        self.assertEqual(MEDIA.forwarding_diagnostics(snapshot, 'browser-browser')['status'], 'measured')
        snapshot['queues'][-1]['max_forwarded_frame_age_ms'] = None
        absent_reorder = MEDIA.forwarding_diagnostics(snapshot, 'browser-browser')
        self.assertEqual(absent_reorder['status'], 'failed')
        self.assertEqual(absent_reorder['required_application_video_age_unobserved_stages'], ['webrtc-video-reorder'])
        snapshot['queues'][-1]['max_forwarded_frame_age_ms'] = 100
        snapshot['quic']['send_buffer'] = {'observations': 2, 'capacity_bytes': 76032,
            'minimum_available_bytes': 100, 'maximum_available_bytes': 76032,
            'sampled_high_water_bytes': 75932, 'send_exceeded_sampled_available_bytes': 1}
        self.assertEqual(MEDIA.forwarding_diagnostics(snapshot, 'browser-browser')['status'], 'measured')
        snapshot['queues'][-1]['max_forwarded_frame_age_ms'] = None
        absent_reorder = MEDIA.forwarding_diagnostics(snapshot, 'browser-browser')
        self.assertEqual(absent_reorder['status'], 'failed')
        self.assertEqual(absent_reorder['required_application_video_age_unobserved_stages'], ['webrtc-video-reorder'])
        snapshot['queues'][-1]['max_forwarded_frame_age_ms'] = 100
        snapshot['quic']['send_buffer']['maximum_available_bytes'] = 76033
        self.assertEqual(MEDIA.forwarding_diagnostics(snapshot, 'browser-browser')['status'], 'failed')

    def test_resource_sampling_requires_time_coverage_and_exposes_errors_and_gaps(self):
        sampler = MEDIA.ResourceSampler([]); sampler.started = 0
        row = lambda at: {'seconds': at, 'rss_bytes': 10, 'cpu_percent': 2, 'processes': 1}
        with patch.object(MEDIA.time, 'monotonic', return_value=10):
            sampler.samples = [row(index / 2) for index in range(20)]
            self.assertTrue(sampler.report()['coverage_complete'])
            sampler.samples = [row(0)]
            self.assertFalse(sampler.report()['coverage_complete'])
            sampler.samples = [row(index / 2) for index in range(20) if not 4 <= index / 2 < 5.5]
            sampler.sampling_errors = {'TimeoutExpired': 1}
            observed = sampler.report()
            self.assertFalse(observed['coverage_complete'])
            self.assertGreater(observed['maximum_sampling_gap_seconds'], 1.25)
            self.assertEqual(observed['sampling_errors']['count'], 1)

    def test_constant_dc_is_not_identifiable_audio(self):
        with self.assertRaisesRegex(ValueError, 'zero frequency is unsupported'):
            MEDIA.decode_audio('unused', 0)
        values = array.array('f', [.1] * 16000).tobytes()
        with patch.object(MEDIA.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, values, b'')):
            observed = MEDIA.decode_audio('unused', 440)
        self.assertFalse(observed['identifiable_tone'])

    def test_wrong_tone_cannot_pass_expected_frequency(self):
        values = array.array('f', [.1 * math.sin(2 * math.pi * 660 * i / 8000) for i in range(16000)]).tobytes()
        with patch.object(MEDIA.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, values, b'')):
            self.assertTrue(MEDIA.decode_audio('unused', 660)['identifiable_tone'])
            self.assertFalse(MEDIA.decode_audio('unused', 440)['identifiable_tone'])

    def test_static_decoded_pixels_do_not_count_as_motion(self):
        data = '# frame hashes\n0,0,0,1,4,same\n0,1,1,1,4,same\n'
        with patch.object(MEDIA.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, data, '')):
            observed = MEDIA.decode_video('unused', 60)
        self.assertEqual(observed['decoded_frames'], 2)
        self.assertEqual(observed['changing_frames'], 0)

    def test_independent_visual_identity_rejects_video_only_swap_loopback_unknown_and_white(self):
        def frame(identity):
            pixels = bytearray([100] * 64)
            for index, high in zip([0, 1, 8, 9], [identity & 1, identity & 2, not identity & 1, not identity & 2]):
                pixels[index] = 200 if high else 40
            return pixels
        cases = []
        for identity in range(4):
            pixels = frame(identity) + bytes([255]) * 64 + frame(identity)
            self.assertEqual(MEDIA.source_identity(pixels, identity)['status'], 'passed')
            for wrong in range(4):
                if wrong != identity:
                    self.assertEqual(MEDIA.source_identity(pixels, wrong)['status'], 'failed')
            cases.append({'pixels': list(pixels), 'expected': identity})
        for bad in [bytes([100]) * 64, bytes([255]) * 64, bytes([0]) * 64]:
            self.assertEqual(MEDIA.source_identity(bad, 0)['status'], 'failed')
            cases.append({'pixels': list(bad), 'expected': 0})
        black_after_valid = frame(0) + bytes(64)
        self.assertEqual(MEDIA.source_identity(black_after_valid, 0, True)['status'], 'passed')
        cases.append({'pixels': list(black_after_valid), 'expected': 0, 'allowBlack': True})
        # Exercise the real browser observer classifier independently, without
        # starting Chrome/capture, and compare semantic outcomes for all cases.
        module = (pathlib.Path(__file__).parents[1] / 'video/source_identity.mjs').resolve().as_uri()
        code = """import{classifySourceFrame,sourceIdentityReport}from MODULE;
            let text='';for await(const chunk of process.stdin)text+=chunk;
            console.log(JSON.stringify(JSON.parse(text).map(row=>{
                const counts={};for(let at=0;at<row.pixels.length;at+=64){
                    const kind=classifySourceFrame(row.pixels.slice(at,at+64));counts[kind]=(counts[kind]??0)+1;}
                return sourceIdentityReport(counts,row.expected,Boolean(row.allowBlack));})));""".replace('MODULE', json.dumps(module))
        result = subprocess.run([os.environ.get('CONNECT_NODE_BINARY', 'node'), '--input-type=module', '-e', code],
            input=json.dumps(cases), text=True, capture_output=True, check=True)
        javascript = json.loads(result.stdout)
        for row, actual in zip(cases, javascript):
            self.assertEqual(actual, MEDIA.source_identity(bytes(row['pixels']), row['expected'], row.get('allowBlack', False)))

    def test_file_load_requires_full_actual_window_containment_and_live_keyframe(self):
        file = {'start_unix_seconds': 12, 'end_unix_seconds': 19}
        self.assertEqual(FIXTURE.transfer_containment(file, [(10, 70), (11, 71)])['status'], 'passed')
        for windows in [[(20, 80)], [(10, 18)], [(13, 70)], []]:
            self.assertEqual(FIXTURE.transfer_containment(file, windows)['status'], 'failed')
        self.assertEqual(FIXTURE.transfer_containment({'start_unix_seconds': 12}, [(10, 70)])['status'], 'failed')
        with tempfile.TemporaryDirectory() as directory:
            target = self.write_ivf(directory, [])
            keyframe = b'\x00\x00\x00\x9d\x01\x2a\x80\x02\x68\x01'
            frame = struct.pack('<IQ', len(keyframe), 0) + keyframe
            with target.open('ab') as output: output.write(frame[:-1])
            self.assertFalse(MEDIA.decoder.live_keyframe_present(target))
            with target.open('ab') as output: output.write(frame[-1:])
            self.assertTrue(MEDIA.decoder.live_keyframe_present(target))

    def test_clean_full_window_freshness_rejects_midwindow_freeze_despite_passing_average(self):
        observe = MEDIA.decoder.motion_observer.motion_observation
        regular = [index / 30 for index in range(1, 1800)]
        content = {'window_start_unix_seconds': 0, 'window_end_unix_seconds': 60,
            'changing_times': regular}
        self.assertEqual(observe(content)['status'], 'passed')
        frozen = dict(content, changing_times=[at for at in regular if not 20 <= at < 25])
        self.assertGreater(len(frozen['changing_times']) / 60, 24)
        self.assertEqual(observe(frozen)['status'], 'failed')
        self.assertGreater(observe(frozen)['maximum_unexcluded_freshness_gap_seconds'], 5)
        self.assertEqual(observe(dict(content, changing_times=[at for at in regular if at >= 3]))['status'], 'failed')
        self.assertEqual(observe(dict(content, changing_times=[at for at in regular if at < 57]))['status'], 'failed')
        self.assertEqual(observe(dict(content, changing_times=[1, 1, 2]))['status'], 'failed')
        # The primary native motion decoder, not receipt alone, supplies times.
        with tempfile.TemporaryDirectory() as directory:
            target = self.write_ivf(directory, [0, 3000, 6000, 9000])
            report = {'unix_start_seconds': 100, 'video': [{'timestamp_rtp': stamp,
                'timestamp_ivf': stamp, 'arrival_seconds': at, 'emit_seconds': at, 'keyframe': stamp == 0}
                for stamp, at in zip([0, 3000, 6000, 9000], [0, .5, 3, 3.1])]}
            window = {'start_emit_seconds': 0, 'start_arrival_seconds': 0, 'seconds': 4}
            metadata = self.frame_metadata([0, 3000, 6000, 9000], ['a', 'a', 'b', 'c'])
            with patch.object(MEDIA.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, metadata, '')) as run:
                decoded = MEDIA.decode_video(target, 4, 'selected-ffmpeg', report, window)
            self.assertIn('-copyts', run.call_args.args[0])
            self.assertIn('1:90000', run.call_args.args[0])
            self.assertEqual(decoded['motion_content']['changing_times'], [103, 103.1])
            self.assertEqual(decoded['decoded_mapping']['mapped_decoded_frames'], 4)
            self.assertEqual(observe(decoded['motion_content'])['status'], 'failed')

    def test_motion_fault_exclusions_do_not_hide_unrelated_freeze_and_use_actual_camera_events(self):
        observe = MEDIA.decoder.motion_observer.motion_observation
        regular = [index / 30 for index in range(1, 1800)]
        base = {'window_start_unix_seconds': 0, 'window_end_unix_seconds': 60,
            'changing_times': regular}
        bursts = [{'start_unix_seconds': 5, 'end_unix_seconds': 5.25}]
        impaired = dict(base, changing_times=[at for at in regular if not 5 <= at < 7.15])
        # First post-burst motion arrives within the existing two-second bound,
        # even though the entire gap includes the deliberately stalled interval.
        self.assertEqual(observe(impaired, 'impairment', bursts=bursts)['status'], 'passed')
        unrelated = dict(impaired, changing_times=[at for at in impaired['changing_times'] if not 20 <= at < 25])
        self.assertEqual(observe(unrelated, 'impairment', bursts=bursts)['status'], 'failed')
        late = dict(base, changing_times=[at for at in regular if not 5 <= at < 7.3])
        self.assertEqual(observe(late, 'impairment', bursts=bursts)['status'], 'failed')
        camera = [{'kind': 'video', 'muted': True, 'unix_seconds': 10},
            {'kind': 'video', 'muted': False, 'unix_seconds': 14}]
        muted = dict(base, changing_times=[at for at in regular if not 10 <= at < 15.9])
        self.assertEqual(observe(muted, 'mute', camera)['status'], 'passed')
        self.assertEqual(observe(muted, 'clean')['status'], 'failed')
        self.assertEqual(observe(muted, 'mute', [])['status'], 'failed')
        audio_only = [dict(row, kind='audio') for row in camera]
        self.assertEqual(observe(muted, 'mute', audio_only)['status'], 'failed')
        unrelated_mute = dict(muted, changing_times=[at for at in muted['changing_times'] if not 30 <= at < 35])
        self.assertEqual(observe(unrelated_mute, 'mute', camera)['status'], 'failed')
        rows = [{'content': base, 'mode': 'clean'}, {'content': unrelated, 'mode': 'impairment', 'bursts': bursts},
            {'content': impaired, 'mode': 'impairment', 'bursts': bursts}, {'content': late, 'mode': 'impairment', 'bursts': bursts},
            {'content': muted, 'mode': 'mute', 'events': camera}, {'content': muted, 'mode': 'mute', 'events': audio_only},
            {'content': unrelated_mute, 'mode': 'mute', 'events': camera}]
        module = (pathlib.Path(__file__).parents[1] / 'video/motion_observation.mjs').resolve().as_uri()
        code = """import{motionObservation}from MODULE;let text='';for await(const chunk of process.stdin)text+=chunk;
            console.log(JSON.stringify(JSON.parse(text).map(row=>motionObservation(row.content,row.mode,row.events??[],row.bursts??[]))));""".replace('MODULE', json.dumps(module))
        result = subprocess.run([os.environ.get('CONNECT_NODE_BINARY', 'node'), '--input-type=module', '-e', code],
            input=json.dumps(rows), text=True, capture_output=True, check=True)
        for row, browser in zip(rows, json.loads(result.stdout)):
            self.assertEqual(browser, observe(row['content'], row['mode'], row.get('events', []), row.get('bursts', [])))

    def test_worker_pipeline_rejects_zero_missing_or_uncompleted_activity_without_claiming_gap_bounds(self):
        module = (pathlib.Path(__file__).parents[1] / 'video/encoded_worker_observation.mjs').resolve().as_uri()
        code = """import{workerPipeline}from MODULE;
            const initial={observedFrames:30,observedKeyframes:1,keyframeRequests:0,keyframeCompletions:0,keyframeFailures:0};
            const good={observedFrames:390,observedKeyframes:13,keyframeRequests:12,keyframeCompletions:12,keyframeFailures:0};
            const cases=[[initial,good],[initial,initial],[null,good],[initial,null],
                [initial,{...good,keyframeCompletions:0}],[initial,{...good,observedKeyframes:1}],
                [initial,{...good,keyframeFailures:1}]];
            console.log(JSON.stringify(cases.map(([a,b])=>workerPipeline(a,b,[{unix_seconds:6,outbound:{worker:good}}],0,12))));""".replace('MODULE', json.dumps(module))
        result = subprocess.run([os.environ.get('CONNECT_NODE_BINARY', 'node'), '--input-type=module', '-e', code],
            capture_output=True, text=True, check=True)
        observed = json.loads(result.stdout)
        self.assertEqual([row['status'] for row in observed], ['passed'] + ['failed'] * 6)
        self.assertEqual(observed[0]['deltas']['observedFrames'], 360)
        self.assertEqual(observed[0]['maximum_snapshot_observation_gap_seconds'], 6)
        self.assertIn('do not bound inter-keyframe gaps', observed[0]['scope'])

    def test_spectral_received_audio_identity_survives_pulse_envelope_and_phase_shift_without_expected_tone_bias(self):
        module = (pathlib.Path(__file__).parents[1] / 'video/audio_spectrum.mjs').resolve().as_uri()
        code = """import{spectralTone}from MODULE;const n=2048,rate=48000;
            function signal(hz,mutate){return Array.from({length:n},(_,index)=>{
                if(hz===0)return mutate==='dc'?.05:0;
                const amplitude=mutate==='pulse'?(index<900?.7:.05):.05;
                const phase=mutate==='phase'&&index>=1000?Math.PI/2:0;
                return amplitude*Math.sin(2*Math.PI*hz*index/rate+phase);});}
            function fft(input){return Array.from({length:n/2},(_,bin)=>{
                if(bin*rate/n>3100)return-Infinity;
                let real=0,imag=0;for(let index=0;index<n;index++){
                    const window=.42-.5*Math.cos(2*Math.PI*index/n)+.08*Math.cos(4*Math.PI*index/n);
                    const angle=2*Math.PI*bin*index/n;real+=input[index]*window*Math.cos(angle);imag-=input[index]*window*Math.sin(angle);}
                return 10*Math.log10(real*real+imag*imag);});}
            const cases=[[440,'normal'],[440,'pulse'],[440,'phase'],[660,'normal'],[0,'dc'],[0,'silence']];
            console.log(JSON.stringify(cases.map(([hz,mode])=>{const data=signal(hz,mode);return spectralTone(data,fft(data),rate,n);})));""".replace('MODULE', json.dumps(module))
        result = subprocess.run([os.environ.get('CONNECT_NODE_BINARY', 'node'), '--input-type=module', '-e', code],
            capture_output=True, text=True, check=True)
        tones = json.loads(result.stdout)
        for tone in tones[:3]: self.assertLess(abs(tone['frequency_hz'] - 440), 35)
        self.assertLess(abs(tones[3]['frequency_hz'] - 660), 35)
        self.assertGreater(abs(tones[3]['frequency_hz'] - 440), 35)
        for tone in tones[4:]:
            self.assertLess(tone['rms'], .001)
            self.assertEqual(tone['frequency_hz'], 0)

    def audio_content_fixture(self):
        return {'window_start_unix_seconds': 0, 'window_end_unix_seconds': 20,
            'samples': [{'unix_seconds': index / 10, 'rms': .05, 'frequency_hz': 660} for index in range(201)]}

    def test_browser_audio_requires_throughout_window_identity_and_complete_sample_coverage(self):
        observe = MEDIA.decoder.audio_observer.audio_observation
        self.assertEqual(observe(self.audio_content_fixture(), 660)['status'], 'passed')
        for mutation in ['wrong-tone', 'silence']:
            content = self.audio_content_fixture()
            for row in content['samples']:
                if 8 <= row['unix_seconds'] < 10:
                    if mutation == 'wrong-tone': row['frequency_hz'] = 440
                    else: row['rms'] = 0
            self.assertEqual(content['samples'][-1]['frequency_hz'], 660)
            self.assertGreater(content['samples'][-1]['rms'], .001)
            self.assertEqual(observe(content, 660)['status'], 'failed')
        content = self.audio_content_fixture(); del content['samples'][50:55]
        self.assertFalse(observe(content, 660)['coverage_complete'])
        content = self.audio_content_fixture(); content['samples'][50]['unix_seconds'] = content['samples'][49]['unix_seconds']
        self.assertFalse(observe(content, 660)['coverage_complete'])

    def test_browser_audio_fault_exceptions_use_actual_mute_burst_and_observed_recovery_only(self):
        observe = MEDIA.decoder.audio_observer.audio_observation
        muted = self.audio_content_fixture()
        events = [{'kind': 'audio', 'muted': True, 'unix_seconds': 5},
            {'kind': 'audio', 'muted': False, 'unix_seconds': 7}]
        for row in muted['samples']:
            if 5.1 <= row['unix_seconds'] < 7.3: row.update(rms=0, frequency_hz=0)
        positive = observe(muted, 660, 'mute', events)
        self.assertEqual(positive['status'], 'passed')
        self.assertEqual(positive['observed_exceptions'][0]['observed_identified_recovery_unix_seconds'], 7.3)
        self.assertEqual(observe(muted, 660, 'mute', [])['status'], 'failed')
        invalid = self.audio_content_fixture()
        for row in invalid['samples']:
            if 8 <= row['unix_seconds'] < 10: row.update(rms=0, frequency_hz=0)
        self.assertEqual(observe(invalid, 660, 'mute', events)['status'], 'failed')
        burst = [{'start_unix_seconds': 9, 'end_unix_seconds': 9.25}]
        impaired = self.audio_content_fixture()
        for row in impaired['samples']:
            if 9 <= row['unix_seconds'] < 9.6: row.update(rms=0, frequency_hz=0)
        recovered = observe(impaired, 660, 'impairment', bursts=burst)
        self.assertEqual(recovered['status'], 'passed')
        impaired['samples'][110]['frequency_hz'] = 440
        self.assertEqual(observe(impaired, 660, 'impairment', bursts=burst)['status'], 'failed')
        self.assertEqual(observe(impaired, 660, 'impairment', bursts=[])['status'], 'failed')
        for row in impaired['samples']:
            if 9 <= row['unix_seconds'] < 12: row.update(rms=0, frequency_hz=0)
        self.assertEqual(observe(impaired, 660, 'impairment', bursts=burst)['status'], 'failed')
        # Real host/browser classifier parity, without browser/device/network runtime.
        module = (pathlib.Path(__file__).parents[1] / 'video/audio_observation.mjs').resolve().as_uri()
        code = """import{audioObservation}from MODULE;let text='';for await(const chunk of process.stdin)text+=chunk;
            const rows=JSON.parse(text);console.log(JSON.stringify(rows.map(row=>audioObservation(row.content,660,row.mode,row.events??[],row.bursts??[]))));""".replace('MODULE', json.dumps(module))
        rows = [{'content': muted, 'mode': 'mute', 'events': events},
            {'content': self.audio_content_fixture(), 'mode': 'clean'}, {'content': invalid, 'mode': 'clean'}]
        result = subprocess.run([os.environ.get('CONNECT_NODE_BINARY', 'node'), '--input-type=module', '-e', code],
            input=json.dumps(rows), text=True, capture_output=True, check=True)
        for row, browser in zip(rows, json.loads(result.stdout)):
            self.assertEqual(browser, observe(row['content'], 660, row['mode'], row.get('events', []), row.get('bursts', [])))

    def marker_fixture(self, skew):
        seconds = 12
        video_rows = [{'arrival_seconds': 0, 'timestamp_rtp': 0}]
        audio_rows = [{'arrival_seconds': 0, 'bytes': 160}]
        frames = bytearray(bytes([80]) * 64); samples = array.array('f', [.005] * 160)
        for at in [frame / 30 for frame in MEDIA.decoder.markers.marker_frames() if frame / 30 < seconds]:
            for when, high in [(at, True), (at + .15, False)]:
                video_rows.append({'arrival_seconds': when, 'timestamp_rtp': round(when * 90000)})
                frames.extend(bytes([255 if high else 80]) * 64)
                if when + skew < seconds:
                    audio_rows.append({'arrival_seconds': when + skew, 'bytes': 160})
                    samples.extend([.1 if high else .005] * 160)
        stamps = [row['timestamp_rtp'] for row in video_rows]
        hashes = [hashlib.md5(frames[index:index + 64]).hexdigest() for index in range(0, len(frames), 64)]
        with tempfile.TemporaryDirectory() as directory:
            target = self.write_ivf(directory, stamps)
            with patch.object(MEDIA.decoder, 'decode_gray_frames', return_value=(frames, self.frame_metadata(stamps, hashes), 0)), \
                    patch.object(MEDIA.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, samples.tobytes(), b'')):
                return MEDIA.audiovisual_markers(target, 'unused', {'video': video_rows, 'audio': audio_rows},
                    {'start_arrival_seconds': 0, 'seconds': seconds})

    def test_marker_skew_uses_decoded_content_and_frozen_thresholds(self):
        self.assertEqual(MEDIA.decoder.observed_rising_edges([True, True, False, True], [0, .1, .5, 1]), [1])
        self.assertEqual(MEDIA.decoder.observed_rising_edges([True] * 4, [0, .1, .5, 1]), [])
        clean = self.marker_fixture(.05)
        self.assertEqual(clean['status'], 'passed')
        self.assertEqual(clean['video_markers'], 20)
        self.assertEqual(clean['audio_markers'], 20)
        self.assertEqual(len(clean['marker_pairs']), 20)
        self.assertEqual(clean['video_calibration']['ordinals'], list(range(20)))
        self.assertEqual(self.marker_fixture(.2)['status'], 'failed')

    def test_coded_ordinals_reject_whole_period_lag_and_fail_closed_on_missing_or_ambiguous_proof(self):
        marker = MEDIA.decoder.markers
        schedule = [frame / 30 for frame in marker.marker_frames()]
        edges = [at + 10 for at in schedule if at < 60]
        for lag in [1, 2]:
            # The former identical1Hz/nearest rule would pair against the next
            # cycle and report zero skew for almost the entire window.
            periodic_audio = list(range(61)); periodic_video = [at + lag for at in range(60)]
            nearest = [min(abs(at - other) for other in periodic_audio) for at in periodic_video]
            self.assertGreaterEqual(sum(value == 0 for value in nearest), 58)
            observed = marker.ordinal_sync([at + lag for at in edges], edges, 60)
            self.assertEqual(observed['video_calibration']['ordinals'], observed['audio_calibration']['ordinals'])
            self.assertEqual(observed['paired'], len(edges))
            self.assertAlmostEqual(observed['p95_abs_skew_ms'], lag * 1000)
            self.assertEqual(observed['status'], 'failed')
        self.assertEqual(marker.ordinal_sync(edges[:6], edges, 60)['status'], 'failed')
        self.assertEqual(marker.ordinal_sync(edges[:20], edges[:20], 60)['status'], 'failed')
        self.assertEqual(marker.calibrate(list(range(7)), list(range(20)))['status'], 'ambiguous')
        self.assertEqual(marker.calibrate([1, 1, 2, 3, 4, 5, 6])['status'], 'missing-proof')
        omitted = [at for index, at in enumerate(edges) if index not in [7, 13, 24]]
        unique_gap = marker.ordinal_sync(omitted, edges, 60)
        self.assertEqual(unique_gap['status'], 'passed')
        self.assertEqual(unique_gap['video_calibration']['missing_internal_markers'], 3)
        self.assertEqual(unique_gap['unmatched_audio_markers'], 3)

    def test_marker_program_and_browser_calibration_match_with_bounded_jitter_and_crop(self):
        marker = MEDIA.decoder.markers
        schedule = [frame / 30 for frame in marker.marker_frames()]
        intervals = [b - a for a, b in zip(marker.marker_frames(), marker.marker_frames()[1:])]
        self.assertEqual(len(set(tuple(intervals[at:at + 6]) for at in range(len(intervals) - 5))), len(intervals) - 5)
        self.assertLessEqual(len(schedule), 512)
        self.assertLess(max(schedule), 300)
        audio = [at + 30 + (index % 3) * .01 for index, at in enumerate(schedule) if 2 < at < 62]
        video = [at + 30 + .03 + (index % 4) * .02 for index, at in enumerate(schedule) if 2 < at < 62]
        observed = marker.ordinal_sync(video, audio, 60)
        self.assertEqual(observed['status'], 'passed')
        # A cropped first high level never supplies an edge; the remaining
        # nonperiodic word independently establishes its real source ordinal.
        self.assertGreater(observed['video_calibration']['ordinals'][0], 0)
        module = (pathlib.Path(__file__).parents[1] / 'video/marker_calibration.mjs').resolve().as_uri()
        code = """import{markerFrames,ordinalSync,calibrate}from MODULE;
            let text='';for await(const chunk of process.stdin)text+=chunk;const rows=JSON.parse(text);
            console.log(JSON.stringify({frames:markerFrames(),proof:ordinalSync(rows.video,rows.audio,60),
                ambiguous:calibrate([0,1,2,3,4,5,6],Array.from({length:20},(_,at)=>at))}));""".replace('MODULE', json.dumps(module))
        result = subprocess.run([os.environ.get('CONNECT_NODE_BINARY', 'node'), '--input-type=module', '-e', code],
            input=json.dumps({'video': video, 'audio': audio}), text=True, capture_output=True, check=True)
        browser = json.loads(result.stdout)
        self.assertEqual(browser['frames'], marker.marker_frames())
        self.assertEqual(browser['proof']['status'], observed['status'])
        self.assertEqual(browser['proof']['video_calibration']['ordinals'], observed['video_calibration']['ordinals'])
        self.assertAlmostEqual(browser['proof']['p95_abs_skew_ms'], observed['p95_abs_skew_ms'])
        self.assertEqual(browser['ambiguous']['status'], 'ambiguous')

    def test_relative_rtp_origin_cannot_claim_absolute_sync(self):
        observed = MEDIA.clock_observation({'video': [], 'audio': []})
        self.assertEqual(observed['absolute_av_sync'], 'unmeasured')

    def test_recovery_cannot_pass_repeated_frozen_decoder_content(self):
        rows = [{'arrival_seconds': value, 'timestamp_rtp': index * 3000} for index, value in enumerate([.1, .2, 1, 1.1, 1.2])]
        stamps = [row['timestamp_rtp'] for row in rows]
        with tempfile.TemporaryDirectory() as directory:
            target = self.write_ivf(directory, stamps)
            def observe(hashes):
                with patch.object(MEDIA.decoder, 'decode_hash_frames', return_value=(self.frame_metadata(stamps, hashes), 0)):
                    return MEDIA.decode_recovery(target, {'unix_start_seconds': 10, 'video': rows},
                        {'start_arrival_seconds': 0, 'seconds': 2}, [{'start_unix_seconds': 10.3, 'end_unix_seconds': 10.8}])
            self.assertEqual(observe(['same'] * 5)['status'], 'failed')
            self.assertEqual(observe(range(5))['status'], 'failed')
            rows[2]['keyframe'] = True
            self.assertEqual(observe(range(5))['status'], 'passed')

    def test_recovery_unmapped_decoder_journal_is_failure(self):
        rows = [{'arrival_seconds': index / 30, 'timestamp_rtp': index * 3000} for index in range(3)]
        report = {'unix_start_seconds': 10, 'video': rows}; window = {'start_arrival_seconds': 0, 'seconds': 2}
        with tempfile.TemporaryDirectory() as directory:
            target = self.write_ivf(directory, [0, 3000, 6000])
            for stamps in [[999], [0, 0], [6000, 3000]]:
                with self.assertRaisesRegex(RuntimeError, 'unique ordered'):
                    MEDIA.map_decoded_frames(target, report, window, self.frame_metadata(stamps, ['hash'] * len(stamps)))
            mapped, observation = MEDIA.map_decoded_frames(target, report, window, self.frame_metadata([0, 6000], ['first', 'last']))
            self.assertEqual([entry[0] for entry in mapped], [rows[0], rows[2]])
            self.assertEqual(observation['decoder_skipped_complete_inputs'], 1)
            rows[2]['timestamp_rtp'] = 6001
            with self.assertRaisesRegex(RuntimeError, 'timestamps differ'):
                MEDIA.map_decoded_frames(target, report, window, self.frame_metadata([0, 6000], ['first', 'last']))

    def mute_fixture(self):
        content = {'window_start_unix_seconds': 0, 'window_end_unix_seconds': 20,
            'changing_times': [index / 30 for index in range(1, 600) if not 6 <= index / 30 < 8],
            'audio_samples': [{'unix_seconds': index / 10, 'rms': 0 if 10 <= index / 10 < 12 else .1,
                'frequency_hz': 440} for index in range(200)]}
        transitions = [{'kind': kind, 'muted': state, 'unix_seconds': at} for kind, state, at in [
            ('video', True, 6), ('video', False, 8), ('audio', True, 10), ('audio', False, 12)]]
        return content, transitions

    def test_muting_requires_decoded_freeze_silence_and_fresh_recovery(self):
        content, transitions = self.mute_fixture()
        self.assertEqual(MEDIA.mute_observation(content, transitions, 440)['status'], 'passed')
        content['decoded_keyframe_times'] = []
        self.assertEqual(MEDIA.mute_observation(content, transitions, 440)['status'], 'failed')
        content['decoded_keyframe_times'] = [8.1]
        self.assertEqual(MEDIA.mute_observation(content, transitions, 440)['status'], 'passed')
        content['changing_times'].append(7)
        self.assertEqual(MEDIA.mute_observation(content, transitions, 440)['status'], 'failed')
        content, transitions = self.mute_fixture()
        for sample in content['audio_samples']:
            if 10.5 <= sample['unix_seconds'] < 11.9: sample['rms'] = .1
        self.assertEqual(MEDIA.mute_observation(content, transitions, 440)['status'], 'failed')

    def test_muting_requires_other_media_to_continue_and_actual_setter_causality(self):
        content, transitions = self.mute_fixture()
        for sample in content['audio_samples']:
            if 6.5 <= sample['unix_seconds'] < 7.9: sample['rms'] = 0
        self.assertEqual(MEDIA.mute_observation(content, transitions, 440)['status'], 'failed')
        content, transitions = self.mute_fixture()
        self.assertEqual(MEDIA.mute_observation(content, transitions[:2], 440)['status'], 'failed')


class DatagramProxy(unittest.IsolatedAsyncioTestCase):
    def selected_fixture(self, ports):
        from types import SimpleNamespace
        relay = RTC.WebRtcImpairment('ws://127.0.0.1:3000', 100, MEDIA.DatagramImpairment, 4)
        relay.relays = {3001: SimpleNamespace(address=('127.0.0.1', 5001), drop_probability=.03),
            3002: SimpleNamespace(address=('127.0.0.1', 5002), drop_probability=0)}
        observations = [{'elapsed_seconds': 4, 'selected_ice_samples': [
            {'unix_seconds': index / 2, 'remote_port': port, 'transport': 'udp', 'candidate_type': 'host'}
            for index in range(9)]} for port in ports]
        return relay, observations

    def test_selected_media_route_rejects_direct_peer_reflexive_bypass(self):
        relay, observations = self.selected_fixture([5001])
        observations[0]['selected_ice_samples'][4].update(remote_port=3001, candidate_type='prflx')
        self.assertEqual(relay.verify_selected_paths(observations)['status'], 'failed')

    def test_selected_media_route_requires_live_observations_and_exactly_one_impaired_hop(self):
        relay, observations = self.selected_fixture([5001, 5002])
        self.assertEqual(relay.verify_selected_paths(observations)['status'], 'passed')
        self.assertEqual(relay.verify_selected_paths(observations[:1] * 2)['status'], 'failed')
        observations[0]['selected_ice_samples'] = []
        self.assertEqual(relay.verify_selected_paths(observations)['status'], 'failed')

    def test_selected_media_route_rejects_missing_and_late_switched_selection(self):
        relay, observations = self.selected_fixture([5001])
        observations[0]['selected_ice_samples'][-1].pop('remote_port')
        self.assertEqual(relay.verify_selected_paths(observations)['status'], 'failed')

    async def test_concurrent_clients_keep_their_own_bidirectional_path(self):
        loop = asyncio.get_running_loop()
        class Echo(asyncio.DatagramProtocol):
            def connection_made(self, transport): self.transport = transport
            def datagram_received(self, data, address): self.transport.sendto(data, address)
        server, _ = await loop.create_datagram_endpoint(Echo, local_addr=('127.0.0.1', 0))
        clients = []
        try:
            async with MEDIA.DatagramImpairment(server.get_extra_info('sockname'), drop_probability=0, maximum_delay=0) as proxy:
                received = []
                class Client(asyncio.DatagramProtocol):
                    def __init__(self): self.future = loop.create_future()
                    def datagram_received(self, data, _):
                        if not self.future.done(): self.future.set_result(data)
                for message in [b'first-public-client', b'second-public-client']:
                    transport, protocol = await loop.create_datagram_endpoint(Client, local_addr=('127.0.0.1', 0))
                    clients.append(transport); transport.sendto(message, proxy.address)
                    received.append(protocol.future)
                self.assertEqual(await asyncio.wait_for(asyncio.gather(*received), 2),
                    [b'first-public-client', b'second-public-client'])
                observed = proxy.report()
                self.assertEqual(observed['backend_sockets'], 2)
                self.assertEqual(observed['directions']['to_client']['forwarded'], 2)
                counter = observed['directions']['to_client']
                self.assertEqual(counter['received_network_bytes'], counter['received_bytes'] + counter['received'] * 28)
                self.assertEqual(observed['directions']['to_server']['received'], 2)
                self.assertEqual(counter['dispatch_delay_samples'], counter['forwarded'])
                self.assertGreaterEqual(counter['maximum_observed_dispatch_delay_ms'], 0)
                self.assertLessEqual(observed['pending_datagrams_high_water'], 512)
        finally:
            for client in clients: client.close()
            server.close()

    async def test_signaling_frame_roundtrip_and_size_bound(self):
        for masked in [True, False]:
            for content in [b'private-fixture-auth', b'x' * 2048]:
                reader = asyncio.StreamReader(); reader.feed_data(RTC.encode(1, content, masked)); reader.feed_eof()
                self.assertEqual(await RTC.frame(reader), (1, content))
        with self.assertRaises(ValueError): RTC.encode(1, b'x' * 65537, True)

    async def test_ice_rewrite_rejects_unowned_server_port(self):
        import json
        relay = RTC.WebRtcImpairment('ws://127.0.0.1:3000', 100, MEDIA.DatagramImpairment, 60)
        sdp = {'type': 'answer', 'sdp': 'a=candidate:1 1 UDP 2130706431 192.168.1.2 4000 typ host\r\n'}
        with patch.object(RTC, 'owned_udp_ports', return_value={5000}):
            with self.assertRaisesRegex(ValueError, 'does not belong'):
                await relay.rewrite(json.dumps(sdp).encode())


class SelectedModuleInputs(unittest.TestCase):
    def test_module_entry_is_bound_and_directory_cannot_substitute(self):
        selected = importlib.util.spec_from_file_location('video_driver_inputs',
            pathlib.Path(__file__).parents[1] / 'check-video-paths.py')
        driver = importlib.util.module_from_spec(selected); selected.loader.exec_module(driver)
        with tempfile.TemporaryDirectory() as temporary:
            directory = pathlib.Path(temporary).resolve()
            module = directory / 'index.mjs'; module.write_text('export const fixture = 1;')
            manifest = directory / 'package.json'; manifest.write_text('{"name":"fixture"}')
            self.assertEqual(driver.playwright_inputs(module), [module, manifest])
            before = {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in driver.playwright_inputs(module)}
            module.write_text('export const fixture = 2;')
            after = {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in driver.playwright_inputs(module)}
            self.assertNotEqual(before, after)
            self.assertEqual(before[str(manifest)], after[str(manifest)])
            with self.assertRaisesRegex(ValueError, 'module entry file'):
                driver.playwright_inputs(directory)


if __name__ == '__main__': unittest.main()
