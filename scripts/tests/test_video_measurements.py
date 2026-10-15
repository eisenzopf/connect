"""Decoder/audio marker qualification must reject static content and bad skew."""
import array
import asyncio
import contextlib
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
    def test_decoded_audio_envelope_runs_all_thirteen_adversarial_guards(self):
        script = pathlib.Path(__file__).with_name('audio_envelope.test.mjs')
        result = subprocess.run([os.environ.get('CONNECT_NODE_BINARY', 'node'),
            '--test', '--test-reporter=tap', str(script)], capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for count in ('# tests 13', '# pass 13', '# fail 0', '# cancelled 0', '# skipped 0'):
            self.assertIn(count + '\n', result.stdout)

    def test_executable_digest_uses_bounded_reads_and_preserves_exact_sha256(self):
        import builtins
        reads = []
        test = self

        class BoundedReader:
            def __init__(self, path, mode):
                self.stream = builtins.open(path, mode)

            def __enter__(self):
                return self

            def __exit__(self, *_):
                self.stream.close()

            def read(self, size=-1):
                test.assertGreater(size, 0, 'Whole-file reads inflate measured checker RSS')
                test.assertLessEqual(size, 1024 * 1024, 'Hash buffer must stay bounded')
                reads.append(size)
                return self.stream.read(size)

        with tempfile.TemporaryDirectory() as directory:
            for content in (b'', bytes(range(256)) * 8192 + b'nonaligned-tail'):
                path = pathlib.Path(directory) / 'executable-input'
                path.write_bytes(content)
                reads.clear()
                with patch.object(pathlib.Path, 'read_bytes', side_effect=AssertionError('Whole-file allocation forbidden')), \
                        patch.object(pathlib.Path, 'open', lambda path, mode: BoundedReader(path, mode)):
                    self.assertEqual(MEDIA.digest(path), hashlib.sha256(content).hexdigest())
                self.assertEqual(len(reads), math.ceil(len(content) / (1024 * 1024)) + 1)

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
            'queues': [{'stage': stage, 'kind': 'video', 'unit': 'frame-packets',
                'observations': 200, 'capacity_packets': 64, 'sampled_high_water_packets': 8,
                'max_forwarded_frame_age_ms': 100, 'frame_age_boundary': 'dequeue-before-handoff-or-write'}
                for stage in ['quic-video-egress', 'quic-video-reorder']],
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
            for stage, capacity in [('quic-audio-ingress', 10), ('quic-audio-egress', 10), ('quic-video-ingress', 64), ('quic-video-egress', 64), ('quic-video-reorder', 64)]]
        client['quic']['send_buffer'] = {'observations': 2, 'capacity_bytes': 76032,
            'minimum_available_bytes': 100, 'maximum_available_bytes': 76032, 'sampled_high_water_bytes': 75932}
        self.assertEqual(MEDIA.native_sdk_diagnostics(client)['status'], 'measured')
        client['queues'].pop()
        self.assertEqual(MEDIA.native_sdk_diagnostics(client)['status'], 'failed')

    def test_quic_reorder_requires_observed_bounded_age_instead_of_inactive_queue_acknowledgment(self):
        for route in ['native-native', 'native-browser']:
            snapshot = self.diagnostic_fixture()
            if route == 'native-browser':
                snapshot['forwarding'][0]['stage'] = 'core-video-bridge'
                snapshot['queues'] += [dict(snapshot['queues'][0], stage=stage) for stage in
                    ['quic-video-ingress', 'webrtc-video-ingress', 'webrtc-video-egress', 'webrtc-video-reorder']]
            self.assertEqual(MEDIA.forwarding_diagnostics(snapshot, route)['status'], 'measured')
            snapshot['queues'][1]['max_forwarded_frame_age_ms'] = None
            missing = MEDIA.forwarding_diagnostics(snapshot, route)
            self.assertEqual(missing['status'], 'failed')
            self.assertEqual(missing['required_application_video_age_unobserved_stages'], ['quic-video-reorder'])
            self.assertNotIn('quic-video-reorder', missing['acknowledged_queue_age_unobserved_stages'])
            snapshot['queues'][1]['max_forwarded_frame_age_ms'] = 201
            self.assertEqual(MEDIA.forwarding_diagnostics(snapshot, route)['status'], 'failed')
            snapshot['queues'][1].update(max_forwarded_frame_age_ms=100, sampled_high_water_packets=65)
            self.assertEqual(MEDIA.forwarding_diagnostics(snapshot, route)['status'], 'failed')

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

    def test_independent_postprocessing_cpu_cannot_overwrite_frozen_live_operating_samples(self):
        sampler = MEDIA.ResourceSampler([]); sampler.started = 0
        row = lambda at, cpu: {'seconds': at, 'rss_bytes': 10, 'cpu_percent': cpu, 'processes': 1}
        sampler.samples = [row(index / 2, 200) for index in range(20)]
        with patch.object(MEDIA.time, 'monotonic', return_value=10):
            live = sampler.freeze_live()
        self.assertTrue(live['coverage_complete'])
        sampler.samples += [row(10 + index / 2, 1200) for index in range(4)]
        with patch.object(MEDIA.time, 'monotonic', return_value=12):
            after = sampler.postprocessing_report()
            self.assertIs(sampler.report(), live)
        self.assertEqual(live['sampled_peak_cpu_percent'], 200)
        self.assertEqual(live['samples'], 20)
        self.assertEqual(after['sampled_peak_cpu_percent'], 1200)
        self.assertEqual(after['samples'], 4)
        self.assertIn('before independent post-run', live['scope'])
        self.assertIn('separate independent post-run', after['scope'])

    def test_postprocessing_without_a_prior_live_measurement_freeze_fails_closed(self):
        sampler = MEDIA.ResourceSampler([]); sampler.started = 0; sampler.samples = []
        with self.assertRaisesRegex(RuntimeError, 'must be frozen before'):
            sampler.postprocessing_report()

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

    def test_native_black_identity_requires_actual_camera_setters_and_exact_decoded_time(self):
        frame = bytearray([100] * 64)
        for index, value in zip([0, 1, 8, 9], [40, 40, 200, 200]): frame[index] = value
        pixels = frame + bytes(64) + frame
        events = [{'kind': 'video', 'muted': True, 'unix_seconds': 10},
            {'kind': 'video', 'muted': False, 'unix_seconds': 12}]
        observed = MEDIA.source_identity(pixels, 0, frame_times=[9, 12.2, 13], mute_transitions=events)
        self.assertEqual(observed['status'], 'passed')
        self.assertEqual(observed['admitted_muted_black_frames'], 1)
        for when in [9.999, 12.201, 15]:
            observed = MEDIA.source_identity(pixels, 0, frame_times=[9, when, 16], mute_transitions=events)
            self.assertEqual(observed['status'], 'failed')
            self.assertEqual(observed['unexcepted_black_frames'], 1)
        for wrong in [1, 2, 3]:
            self.assertEqual(MEDIA.source_identity(pixels, wrong, frame_times=[9, 11, 13], mute_transitions=events)['status'], 'failed')
        for absent in [[], events[:1], [dict(events[0], muted=False), events[1]]]:
            self.assertEqual(MEDIA.source_identity(pixels, 0, frame_times=[9, 11, 13], mute_transitions=absent)['status'], 'failed')
        with self.assertRaises(ValueError): MEDIA.source_identity(pixels, 0, frame_times=[9, 11])

    def test_failed_marker_calibration_preserves_bounded_numeric_edges_without_changing_verdict(self):
        video = [index * .13 for index in range(20)]; audio = [index * .17 for index in range(30)]
        observed = MEDIA.decoder.markers.ordinal_sync(video, audio, 20)
        self.assertEqual(observed['status'], 'failed')
        self.assertEqual(observed['video_edge_seconds'], video)
        self.assertEqual(observed['audio_edge_seconds'], audio)
        excessive = MEDIA.decoder.markers.ordinal_sync(list(range(513)), audio, 20)
        self.assertEqual(excessive['status'], 'failed')
        self.assertEqual(excessive['video_edge_seconds'], [])

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

    def test_actual_dark_boundary_marker_rejects_blended_unknown_without_lowering_identity_threshold(self):
        # Actual failed decoded cells near a dark moving background blended the
        # fourth high cell down to154. Strict classification must still reject
        # these values; the sampler must read the marker interior instead.
        contaminated = bytearray([80] * 64)
        for at, value in zip([0, 1, 8, 9], [189, 55, 55, 154]): contaminated[at] = value
        self.assertEqual(MEDIA.source_identity(contaminated, 1)['status'], 'failed')
        centres = bytearray([80] * 64)
        for at, value in zip([0, 1, 8, 9], [200, 40, 40, 200]): centres[at] = value
        self.assertEqual(MEDIA.source_identity(centres, 1)['status'], 'passed')
        for wrong in [0, 2, 3]: self.assertEqual(MEDIA.source_identity(centres, wrong)['status'], 'failed')
        for value in [100, 154, 231]:
            unclassified = centres.copy(); unclassified[9] = value
            self.assertEqual(MEDIA.source_identity(unclassified, 1)['status'], 'failed')

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

    def test_audio_burst_recovery_waits_for_delayed_attenuation_and_retains_samples(self):
        observe = MEDIA.decoder.audio_observer.audio_observation
        content = self.audio_content_fixture()
        bursts = [{'start_unix_seconds': 5, 'end_unix_seconds': 5.25},
            {'start_unix_seconds': 12, 'end_unix_seconds': 12.25}]
        # The retained failed run's two actual post-burst timing/RMS patterns;
        # shifted into a synthetic window, without claiming runtime rescue.
        firsts = [5.25 + .003963947296142578, 12.25 + .001463890075683594]
        lows = [5.25 + .10386395454406738, 12.25 + .10076403617858887]
        returns = [5.25 + .2010641098022461, 12.25 + .20176410675048828]
        for index, first, low, resumed, rms in zip([53, 123], firsts, lows, returns, [.00053704287, .000322908426]):
            content['samples'][index]['unix_seconds'] = first
            content['samples'][index + 1].update(unix_seconds=low, rms=rms, frequency_hz=0)
            content['samples'][index + 2]['unix_seconds'] = resumed
        result = observe(content, 660, 'impairment', bursts=bursts)
        self.assertEqual(result['status'], 'passed')
        self.assertEqual(result['observed_bad_samples'], 2)
        self.assertEqual(result['excepted_attenuation_samples'], 2)
        self.assertEqual(result['bad_unexcepted_samples'], 0)
        for check, first, low, stable in zip(result['observed_exceptions'], firsts, lows, returns):
            self.assertEqual(check['first_identified_after_burst_unix_seconds'], first)
            self.assertEqual(check['last_attenuation_unix_seconds'], low)
            self.assertEqual(check['observed_identified_recovery_unix_seconds'], stable)
            self.assertGreaterEqual(check['identified_continuation_witness_unix_seconds'], stable + .2)
            self.assertEqual(check['attenuated_samples'][0]['unix_seconds'], low)

    def stable_audio_recovery_cases(self):
        import copy
        base = self.audio_content_fixture()
        bursts = [{'start_unix_seconds': 5, 'end_unix_seconds': 5.2}]
        rows = []
        def case(name, changes=(), expected='failed', mode='impairment', selected=bursts):
            content = copy.deepcopy(base)
            for at, rms, hz in changes: content['samples'][round(at * 10)].update(rms=rms, frequency_hz=hz)
            row = {'name': name, 'content': content, 'bursts': selected, 'mode': mode, 'expected': expected}
            rows.append(row); return row
        case('delayed-attenuation', [(5.4, 0, 0)], 'passed')
        case('wrong-tone-during-burst', [(5.1, .05, 440)])
        case('wrong-tone-after-first-positive', [(5.4, .05, 440)])
        case('wrong-tone-before-stable-return', [(5.4, 0, 0), (5.5, .05, 440)])
        case('late-silence', [(7.3, 0, 0)])
        case('late-wrong-tone', [(7.3, .05, 440)])
        case('outside-burst', [(4.9, 0, 0)])
        case('no-burst', [(5.4, 0, 0)], selected=[])
        case('clean-silence', [(5.4, 0, 0)], mode='clean')
        case('negative-rms', [(5.4, -.01, 0)])
        case('missing-return', [(at / 10, 0, 0) for at in range(52, 73)])
        case('deadline-return', [(at / 10, 0, 0) for at in range(52, 72)], 'passed')
        case('overlapping-budgets', selected=bursts + [{'start_unix_seconds': 6, 'end_unix_seconds': 6.2}])
        row = case('truncated-aftermath'); row['content']['window_end_unix_seconds'] = 6
        row = case('coverage-gap'); del row['content']['samples'][53:57]
        row = case('unordered'); row['content']['samples'][54]['unix_seconds'] = 5.3
        row = case('overflow'); row['content']['samples'] *= 11
        return rows

    def test_audio_stable_recovery_preserves_identity_deadline_and_coverage_failures(self):
        observe = MEDIA.decoder.audio_observer.audio_observation
        for row in self.stable_audio_recovery_cases():
            with self.subTest(row['name']):
                result = observe(row['content'], 660, row['mode'], bursts=row['bursts'])
                self.assertEqual(result['status'], row['expected'])
        for field in ['rms', 'frequency_hz']:
            content = self.audio_content_fixture(); content['samples'][54][field] = math.nan
            self.assertEqual(observe(content, 660, 'impairment', bursts=[
                {'start_unix_seconds': 5, 'end_unix_seconds': 5.2}])['status'], 'failed')

    def test_audio_stable_recovery_matches_browser_for_all_adversarial_cases(self):
        observe = MEDIA.decoder.audio_observer.audio_observation
        rows = self.stable_audio_recovery_cases()
        module = (pathlib.Path(__file__).parents[1] / 'video/audio_observation.mjs').resolve().as_uri()
        code = """import{audioObservation}from MODULE;let text='';for await(const chunk of process.stdin)text+=chunk;
            console.log(JSON.stringify(JSON.parse(text).map(row=>audioObservation(row.content,660,row.mode,[],row.bursts))));""".replace('MODULE', json.dumps(module))
        result = subprocess.run([os.environ.get('CONNECT_NODE_BINARY', 'node'), '--input-type=module', '-e', code],
            input=json.dumps(rows), text=True, capture_output=True, check=True)
        actual = json.loads(result.stdout)
        self.assertEqual(len(actual), len(rows))
        for row, browser in zip(rows, actual):
            with self.subTest(row['name']):
                host = observe(row['content'], 660, row['mode'], bursts=row['bursts'])
                self.assertEqual(browser, host)
                self.assertEqual(browser['status'], row['expected'])

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

    def test_mute_rgb_hash_mode_preserves_full_resolution_pts_and_native_hash_default(self):
        metadata = self.frame_metadata([0, 3000], ['first', 'last'])
        completed = subprocess.CompletedProcess([], 0, stdout=metadata, stderr='')
        with patch.object(MEDIA.decoder.subprocess, 'run', return_value=completed) as run:
            for pixel_format in (None, 'rgb24'):
                result, diagnostic_bytes = MEDIA.decode_hash_frames(
                    pathlib.Path('nonexistent-fixture.ivf'), pixel_format=pixel_format)
                self.assertEqual(result, metadata)
                self.assertEqual(diagnostic_bytes, 0)
                command = run.call_args.args[0]
                self.assertIn('-copyts', command)
                self.assertEqual(command[command.index('-enc_time_base') + 1], '1:90000')
                self.assertEqual(command[command.index('-fps_mode') + 1], 'passthrough')
                self.assertNotIn('-vf', command)
                self.assertNotIn('-s', command)
                if pixel_format is None:
                    self.assertNotIn('-pix_fmt', command)
                else:
                    self.assertEqual(command[command.index('-pix_fmt') + 1], 'rgb24')
        with self.assertRaises(ValueError):
            MEDIA.decode_hash_frames(pathlib.Path('nonexistent-fixture.ivf'), pixel_format='gray')

    def test_mute_rgb_content_rejects_one_visible_pixel_and_changed_timestamp_binding(self):
        # Generated full-resolution decoded pixels; no private captured media.
        # The retained actual FFmpeg counterexample is separate component proof.
        area = 640 * 360
        black = bytes(area * 3)
        visible = bytearray(black); visible[3 * (area // 2)] = 1
        rgb = [black, black, black]
        residual_yuv = bytes(area) + bytes([127]) * (area // 4) + bytes([129]) * (area // 4)
        neutral_yuv = bytes(area) + bytes([128]) * (area // 2)
        self.assertNotEqual(hashlib.md5(residual_yuv).digest(), hashlib.md5(neutral_yuv).digest())
        stamps = [0, 9000, 108000]
        rows = [{'arrival_seconds': when, 'emit_seconds': when, 'timestamp_rtp': stamp,
                 'timestamp_ivf': stamp, 'keyframe': index > 0}
                for index, (stamp, when) in enumerate(zip(stamps, [6.9, 7.0, 8.1]))]
        report = {'unix_start_seconds': 0, 'video': rows, 'audio': []}
        window = {'start_arrival_seconds': 0, 'seconds': 20}
        def decoded_metadata(path, ffmpeg, *, pixel_format):
            self.assertEqual(pixel_format, 'rgb24')
            # framemd5 RGB24 reports 640*360*3 bytes, not YUV420p's1.5 bytes/pixel.
            result = self.frame_metadata(stamps, [hashlib.md5(frame).hexdigest() for frame in rgb])
            return result.replace(',64,', f',{area * 3},'), 0
        with tempfile.TemporaryDirectory() as directory:
            target = self.write_ivf(directory, stamps)
            audio = pathlib.Path(directory) / 'not-created.pcmu'
            with patch.object(MEDIA.decoder, 'decode_hash_frames', side_effect=decoded_metadata), \
                    patch.object(MEDIA.decoder.subprocess, 'run',
                                 return_value=subprocess.CompletedProcess([], 0, stdout=b'', stderr=b'')):
                observed = MEDIA.decoded_mute_content(target, audio, report, window)
                self.assertEqual(observed['changing_times'], [])
                self.assertEqual(observed['decoder_mapping']['mapped_decoded_frames'], 3)
                self.assertEqual(observed['decoder_mapping']['pixel_hash_format'], 'rgb24')
                content, transitions = self.mute_fixture()
                self.assertEqual(MEDIA.mute_observation(content, transitions, 440)['status'], 'passed')
                rgb[1] = bytes(visible)
                changed = MEDIA.decoded_mute_content(target, audio, report, window)
                self.assertEqual(changed['changing_times'], [7.0, 8.1])
                content['changing_times'] = sorted(set(content['changing_times'] + changed['changing_times']))
                verdict = MEDIA.mute_observation(content, transitions, 440)
                self.assertEqual(verdict['observations'][0]['changing_frames_while_muted'], 1)
                self.assertEqual(verdict['status'], 'failed')
                # A changed IVF source timestamp cannot keep the old decoded/journal mapping.
                self.write_ivf(directory, [0, 9001, 108000])
                with self.assertRaisesRegex(RuntimeError, 'timestamps differ'):
                    MEDIA.decoded_mute_content(target, audio, report, window)

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

    def test_mute_freshness_is_separate_from_clean60_fps_and_rejects_frozen_stale_content(self):
        content, transitions = self.mute_fixture()
        content['changing_times'] = [index / 10 for index in range(1, 200) if not 6 <= index / 10 < 8]
        observed = MEDIA.mute_observation(content, transitions, 440)
        self.assertEqual(observed['status'], 'passed')
        self.assertLess(observed['unmuted_changing_video_fps'], 24)
        self.assertEqual(observed['limits']['clean_60_seconds_changing_video_fps'], 24)
        for excluded in [(10.5, 11.9), (13, 16), (0, 20)]:
            copied = dict(content, changing_times=[at for at in content['changing_times'] if not excluded[0] <= at < excluded[1]])
            self.assertEqual(MEDIA.mute_observation(copied, transitions, 440)['status'], 'failed')
        bad = dict(content, audio_samples=[dict(row, frequency_hz=660) for row in content['audio_samples']])
        self.assertEqual(MEDIA.mute_observation(bad, transitions, 440)['status'], 'failed')


class DatagramProxy(unittest.IsolatedAsyncioTestCase):
    async def test_signaling_proxy_preserves_authentication_and_negotiated_protocol(self):
        for value in ['', 'rvoip.webrtc.v1', 'rvoip.webrtc.v1, token.ok\r\nInjected: yes',
                      'rvoip.webrtc.v1, token.ok, token.other']:
            with self.assertRaises(ValueError): RTC.signaling_protocols({'sec-websocket-protocol': value})
        seen = asyncio.get_running_loop().create_future()
        async def selected_server(reader, writer):
            request = await reader.readuntil(b'\r\n\r\n')
            seen.set_result(b'Sec-WebSocket-Protocol: rvoip.webrtc.v1, token.private-fixture\r\n' in request)
            writer.write(b'HTTP/1.1 101 Switching Protocols\r\nSec-WebSocket-Protocol: rvoip.webrtc.v1\r\n\r\n')
            await writer.drain()
            await reader.read()
            writer.close(); await writer.wait_closed()
        server = await asyncio.start_server(selected_server, '127.0.0.1', 0)
        try:
            port = server.sockets[0].getsockname()[1]
            async with RTC.WebRtcImpairment(f'ws://127.0.0.1:{port}', 100, MEDIA.DatagramImpairment, 4) as proxy:
                reader, writer = await asyncio.open_connection('127.0.0.1', proxy.server.sockets[0].getsockname()[1])
                writer.write(b'GET / HTTP/1.1\r\nSec-WebSocket-Key: Zml4dHVyZQ==\r\n'
                    b'Sec-WebSocket-Protocol: rvoip.webrtc.v1, token.private-fixture\r\n\r\n')
                await writer.drain()
                response = await asyncio.wait_for(reader.readuntil(b'\r\n\r\n'), 2)
                self.assertTrue(await seen)
                self.assertIn(b'Sec-WebSocket-Protocol: rvoip.webrtc.v1\r\n', response)
                self.assertNotIn(b'private-fixture', response)
                writer.close(); await writer.wait_closed()
                self.assertNotIn('private-fixture', json.dumps(proxy.report()))
        finally:
            server.close(); await server.wait_closed()

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


class PeerFailureTelemetryTests(unittest.IsolatedAsyncioTestCase):
    async def test_packet_diagnostics_preserve_seeded_loss_and_never_export_payload(self):
        from time import monotonic, time
        class Sink:
            def __init__(self): self.payloads = []
            def sendto(self, data, destination): self.payloads.append(data)
        observed_type = RTC.diagnostic_datagram_class(MEDIA.DatagramImpairment)
        plain = MEDIA.DatagramImpairment(('127.0.0.1', 9), seed=515, maximum_delay=0)
        observed = observed_type(('127.0.0.1', 9), seed=515, maximum_delay=0)
        for relay in (plain, observed):
            relay.start = monotonic(); relay.start_unix = time()
        sinks = [Sink(), Sink()]
        secret = b'private-fixture-credential-and-payload'
        packets = [b'\x00\x01\x00\x00\x21\x12\xa4\x42' + b'\0' * 12 + secret,
                   b'\x16\xfe\xfd' + b'\0' * 10 + secret,
                   b'\x80\x60' + b'\0' * 10 + secret, b'other-' + secret]
        self.assertEqual([RTC.packet_class(p) for p in packets], ['stun', 'dtls', 'rtp_or_rtcp', 'other'])
        self.assertEqual(RTC.packet_class(b'\x16'), 'other')
        for index in range(240):
            for relay, sink in zip((plain, observed), sinks):
                relay.schedule(packets[index % 4], 'to_server', sink, ('127.0.0.1', 9))
        await asyncio.sleep(.01)
        self.assertEqual(sinks[0].payloads, sinks[1].payloads)
        for key in ('received', 'dropped', 'random_dropped', 'forwarded', 'random_drop_decisions'):
            self.assertEqual(plain.counters['to_server'].get(key), observed.counters['to_server'].get(key))
        report = observed.report()
        self.assertEqual(sum(row['received'] for row in report['packet_classes']['to_server'].values()), 240)
        self.assertEqual(sum(row['random_dropped'] for row in report['packet_classes']['to_server'].values()),
                         observed.counters['to_server']['random_dropped'])
        self.assertNotIn(secret.decode(), json.dumps(report))
        self.assertEqual(observed.pending, 0)

    async def test_peer_failure_report_is_bounded_and_excludes_descriptions_and_error_text(self):
        helper = pathlib.Path(__file__).parents[1] / 'video/browser_failure.mjs'
        code = r'''
import assert from 'node:assert/strict';
const {installPeerDiagnostics,browserFailure}=await import(process.argv[1]);
class Peer extends EventTarget {
 constructor(){super();this.connectionState='new';this.iceConnectionState='new';this.iceGatheringState='new';this.signalingState='stable';this.localDescription=null;this.remoteDescription=null;}
 getTransceivers(){return [];}
}
globalThis.RTCPeerConnection=Peer;
installPeerDiagnostics();
const peers=Array.from({length:5},()=>new RTCPeerConnection());
peers[0].localDescription={sdp:'private-secret-sdp'};
peers[0].remoteDescription={sdp:'private-secret-credential'};
peers[0].connectionState='connecting';peers[0].iceConnectionState='checking';
for(let i=0;i<140;i++)peers[0].dispatchEvent(new Event('connectionstatechange'));
const result=await browserFailure([{evaluate:async callback=>callback()}],new Error('private-secret-error'));
assert.equal(result.status,'failed');
const proof=result.peer_diagnostics[0];assert.equal(proof.omitted_peers,1);assert.equal(proof.peers.length,4);
assert.equal(proof.peers[0].events.length,128);assert.equal(proof.peers[0].omitted_events,13);
assert.equal(proof.peers[0].current.ice,'checking');assert.equal(proof.peers[0].current.remote_description_set,true);
assert(!JSON.stringify(result).includes('private-secret'));
console.log('bounded peer-state privacy guard passed');
'''
        result = subprocess.run([os.environ.get('CONNECT_NODE_BINARY', 'node'), '--input-type=module',
            '-e', code, helper.resolve().as_uri()], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('bounded peer-state privacy guard passed', result.stdout)




class FailureEvidencePreservationTests(unittest.TestCase):

    def test_processing_failure_preserves_measurements_and_cleanup(self):
        import types
        spec = importlib.util.spec_from_file_location('failure_preservation_checker', pathlib.Path(__file__).parents[1] / 'check-video-paths.py')
        CHECK = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(CHECK)
        for fail_at in ('first-window', 'second-window', 'second-decode', 'participant', 'participant-and-health'):
            with self.subTest(fail_at=fail_at), tempfile.TemporaryDirectory() as directory, contextlib.ExitStack() as stack:
                work = pathlib.Path(directory) / 'attempt'
                closed = []
                exited = []
                calls = []
                args = types.SimpleNamespace(server_binary=pathlib.Path('server'), video_binary=pathlib.Path('video'), seconds=60, impairment=False, mute_recovery=False, ffmpeg='ffmpeg')
                results = [{'unix_start_seconds': 100, 'elapsed_seconds': 60, 'end_reason': 'deadline' if n == 0 else 'peer-terminal', 'teardown_seconds': 0.2, 'maximum_rtp_bytes': 1180, 'maximum_encoded_frame_bytes': 99, 'assembly': {}, 'playout': {}, 'maximum_codec_emission_hold_ms': 1, 'success': True} for n in range(2)]

                def prepare(work, binary):
                    state = work / 'state'
                    state.mkdir()
                    for name in ('reviews', 'agent'):
                        (state / (name + '.toml')).write_text('endpoint = "127.0.0.1:1"\n')
                    return (state, {'reviews': 'r', 'agent': 'a'}, {n: {'endpoint': '127.0.0.1:1'} for n in ('reviews', 'agent')}, 'c', 's', 'ws://test')

                class Server:
                    process = types.SimpleNamespace(pid=1)

                    def __init__(self, *a, **k):
                        pass

                    def __enter__(self):
                        return self

                    def __exit__(self, *a):
                        exited.append('server')

                class Proxy:
                    address = ('127.0.0.1', 2)

                    def __init__(self, *a, **k):
                        pass

                    async def __aenter__(self):
                        return self

                    async def __aexit__(self, *a):
                        exited.append('proxy')

                    def report(self):
                        return {'errors': [], 'stall_bursts': [], 'received_wire_bitrate_bits_per_second': {'to': 123}, 'maximum_one_second_received_wire_bits': {'to': 456}}

                class Participant:
                    process = types.SimpleNamespace(pid=2)

                    def __init__(self, *a, **k):
                        self.index = len(calls)
                        calls.append(self)
                        self.report = work / ('absent-report-' + str(self.index))

                    async def wait(self, *a):
                        if fail_at.startswith('participant'):
                            raise ValueError('private participant error')
                        return results[self.index]

                    def close(self):
                        closed.append(self.index)

                class Sampler:

                    def __init__(self, *a):
                        pass

                    def __enter__(self):
                        return self

                    def __exit__(self, *a):
                        exited.append('sampler')

                    def freeze_live(self):
                        return {'coverage_complete': True, 'sampled_peak_rss_bytes': 1234, 'sampled_peak_cpu_percent': 12}

                async def readiness(*a):
                    return {}

                async def transfer(*a):
                    await a[-1]
                    return {'status': 'passed', 'sha256': 'f' * 64, 'bytes': 1048576}

                def window(p, result, seconds):
                    if p.index == (0 if fail_at == 'first-window' else 1) and fail_at != 'second-decode':
                        raise RuntimeError('private error must not be copied')
                    return (str(p.index), 'audio', {'start_emit_seconds': 0})

                def decode(video, *a):
                    if video == '1' and fail_at == 'second-decode':
                        raise ValueError('private decode detail')
                    return {'motion_content': [], 'changing_fps': 25}
                def health_request(*a):
                    if fail_at == 'participant-and-health':
                        raise TimeoutError('private health error')
                    return {'readiness': {'media': {'sentinel': 17}}}

                replacements = [(CHECK, 'prepare', prepare), (CHECK, 'Server', Server), (CHECK, 'native_request', health_request), (CHECK, 'concurrent_file', transfer), (CHECK, 'measurement_readiness', readiness), (CHECK.media, 'MediaProcess', Participant), (CHECK.media, 'ResourceSampler', Sampler), (CHECK.media, 'DatagramImpairment', Proxy), (CHECK.media, 'forwarding_diagnostics', lambda *a: {'status': 'measured', 'sentinel': 17}), (CHECK.media, 'select_window', window), (CHECK.media, 'decode_video', decode), (CHECK.media, 'decode_audio', lambda *a: {}), (CHECK.media, 'audiovisual_markers', lambda *a, **k: {'status': 'passed'}), (CHECK.media, 'clock_observation', lambda *a: {}), (CHECK.media, 'native_sdk_diagnostics', lambda *a: {}), (CHECK.media.decoder.motion_observer, 'motion_observation', lambda *a: {})]
                for obj, name, value in replacements:
                    stack.enter_context(patch.object(obj, name, value))
                observed = asyncio.run(CHECK.qualify_path(args, 'native-native', work))
                self.assertEqual(observed['status'], 'failed')
                self.assertEqual(observed['resources']['sampled_peak_rss_bytes'], 1234)
                if fail_at.startswith('participant'):
                    self.assertEqual(observed['error_class'], 'ValueError')
                    self.assertEqual(observed['native_partial'], [])
                    self.assertNotIn('private participant', json.dumps(observed))
                    self.assertNotIn('private health', json.dumps(observed))
                    if fail_at == 'participant-and-health':
                        self.assertEqual(observed['forwarding_diagnostics']['status'], 'unobserved')
                        self.assertEqual(observed['forwarding_diagnostics']['error_class'], 'TimeoutError')
                    else:
                        self.assertEqual(observed['forwarding_diagnostics']['sentinel'], 17)
                    self.assertEqual(closed, [0, 1])
                    self.assertIn('server', exited)
                    self.assertIn('proxy', exited)
                    self.assertIn('sampler', exited)
                    self.assertEqual(json.loads((work / 'result.json').read_text()), observed)
                    continue
                self.assertEqual(observed['concurrent_reliable_file']['bytes'], 1048576)
                self.assertEqual(observed['forwarding_diagnostics']['sentinel'], 17)
                self.assertEqual(observed['network']['received_wire_bitrate_bits_per_second'], {'to': 123})
                self.assertEqual(observed['paired_termination']['status'], 'passed')
                self.assertEqual(len(observed['native']), 0 if fail_at == 'first-window' else 1)
                self.assertNotIn('durable_conversation_preserved', observed)
                self.assertEqual(closed, [0, 1])
                self.assertIn('sampler', exited)
                self.assertIn('server', exited)
                self.assertIn('proxy', exited)
                self.assertEqual(json.loads((work / 'result.json').read_text()), observed)
                self.assertNotIn('private error', json.dumps(observed))
                self.assertNotIn('private decode', json.dumps(observed))


class AcceptancePolicyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location('acceptance_policy_tests', pathlib.Path(__file__).parents[1] / 'video/acceptance_policy.py')
        cls.policy = importlib.util.module_from_spec(spec); spec.loader.exec_module(cls.policy)

    def sync(self, offset=0, count=70):
        edges = [frame / 30 for frame in MEDIA.decoder.markers.marker_frames()][:count]
        return MEDIA.decoder.markers.ordinal_sync(edges, [at + offset for at in edges], 60)

    def test_strict_clean_and_mute_remain_strict_but_impairment_preserves_degraded_measurement(self):
        import copy
        good = self.sync()
        poor = self.sync(.3)
        original = copy.deepcopy(poor)
        for scenario in ('clean', 'mute'):
            self.assertEqual(self.policy.audio_video_acceptance(good, scenario, 60)['status'], 'passed')
            self.assertEqual(self.policy.audio_video_acceptance(poor, scenario, 60)['status'], 'failed')
        accepted = self.policy.audio_video_acceptance(poor, 'impairment', 60)
        self.assertEqual((accepted['status'], accepted['quality_status'], accepted['reason']),
                         ('passed', 'failed', 'impaired-quality-advisory'))
        self.assertEqual(poor, original)

    def test_sparse_and_contradictory_valid_cadence_are_advisory_only_under_impairment(self):
        sparse = self.sync(count=30)
        contradictory = MEDIA.decoder.markers.ordinal_sync([.1, .2, .3, .4, .5, .6, .7], [], 60)
        for sync in (sparse, contradictory):
            self.assertEqual(sync['status'], 'failed')
            self.assertEqual(self.policy.audio_video_acceptance(sync, 'impairment', 60)['status'], 'passed')
            self.assertEqual(self.policy.audio_video_acceptance(sync, 'clean', 60)['status'], 'failed')

    def test_malformed_edges_and_count_overflow_cannot_be_advisory(self):
        import copy
        good = self.sync()
        for values, count in (([1, float('nan')], 2), ([1, float('inf')], 2), ([2, 1], 2),
                              ([1, 1], 2), ([True, 2], 2), ([1, 2], 1), ([1], True),
                              (list(range(513)), 513), ('not edges', 9)):
            sync = copy.deepcopy(good)
            sync.update(video_edge_seconds=values, video_markers=count)
            self.assertEqual(self.policy.audio_video_acceptance(sync, 'impairment', 60)['status'], 'failed')

    def test_fabricated_summary_pair_and_calibration_metadata_fail_closed(self):
        import copy
        good = self.sync()
        mutations = [lambda s: s.update(paired=99), lambda s: s.update(median_abs_skew_ms=70),
                     lambda s: s.update(status='failed'), lambda s: s.update(marker_pairs=[]),
                     lambda s: s['marker_pairs'][0].update(ordinal=999),
                     lambda s: s['video_calibration']['ordinals'].__setitem__(0, 999),
                     lambda s: s['limits'].update(p95_abs_skew_ms=999)]
        for mutate in mutations:
            sync = copy.deepcopy(good); mutate(sync)
            self.assertEqual(self.policy.audio_video_acceptance(sync, 'impairment', 60)['reason'], 'inconsistent-observation')

    def test_invalid_envelope_cannot_override_good_or_degraded_skew(self):
        for offset in (0, .3):
            for invalid in ({'status': 'observer-invalid'}, {'overflow': True}, {'invalid_input': True},
                            {'invalid': [{'reason': 'unobserved active envelope'}]}):
                sync = self.sync(offset)
                sync['audio_envelope'] = dict(status='observer-valid', overflow=False, invalid_input=False, invalid=[])
                sync['audio_envelope'].update(invalid)
                self.assertEqual(self.policy.audio_video_acceptance(sync, 'impairment', 60)['reason'], 'invalid-observer')
        sync = self.sync(); sync['observer_failure'] = 'invalid mapping'
        self.assertEqual(self.policy.audio_video_acceptance(sync, 'impairment', 60)['status'], 'failed')

    def test_unknown_scenarios_duration_and_unmapped_native_reports_fail_closed(self):
        for scenario in ('', 'mute-recovery', 'exploratory', None, 'IMPAIRMENT'):
            self.assertEqual(self.policy.audio_video_acceptance(self.sync(), scenario, 60)['reason'], 'invalid-scenario')
        for seconds in (0, 121, float('nan'), float('inf'), True, '60'):
            self.assertEqual(self.policy.audio_video_acceptance(self.sync(), 'impairment', seconds)['reason'], 'invalid-duration')
        for sync in (None, {}, {'status': 'unmapped', 'reason': 'decoder correspondence failed'}):
            self.assertEqual(self.policy.audio_video_acceptance(sync, 'impairment', 60)['status'], 'failed')


    def test_shared_acceptance_cases_match_actual_javascript_and_preserve_raw_evidence(self):
        import copy
        fixture_path = pathlib.Path(__file__).with_name('av_acceptance_cases.json')
        fixture = json.loads(fixture_path.read_text())
        outputs = []
        for row in fixture['cases']:
            variant = fixture['variants'][row['variant']]
            sync = MEDIA.decoder.markers.ordinal_sync(variant['video_edges'], variant['audio_edges'], row['seconds'])
            for dotted, value in row['changes']:
                keys = dotted.split('.'); target = sync
                for key in keys[:-1]: target = target[int(key)] if isinstance(target, list) else target[key]
                target[int(keys[-1]) if isinstance(target, list) else keys[-1]] = copy.deepcopy(value)
            original = copy.deepcopy(sync)
            actual = self.policy.audio_video_acceptance(sync, row['scenario'], row['seconds'])
            self.assertEqual(actual, row['expected'], row['name'])
            self.assertEqual(sync, original, row['name'])
            outputs.append(actual)
        self.assertEqual(len(outputs), 30)
        module = (pathlib.Path(__file__).parents[1] / 'video/acceptance_policy.mjs').resolve().as_uri()
        marker = (pathlib.Path(__file__).parents[1] / 'video/marker_calibration.mjs').resolve().as_uri()
        script = ("import fs from 'node:fs';import {audioVideoAcceptance} from " + json.dumps(module)
                  + ";import {ordinalSync} from " + json.dumps(marker) + ";"
                  + "const f=JSON.parse(fs.readFileSync(0,'utf8'));console.log(JSON.stringify(f.cases.map(r=>{"
                  + "const v=f.variants[r.variant],s=ordinalSync(v.video_edges,v.audio_edges,r.seconds);"
                  + "for(const[p,x]of r.changes){const k=p.split('.');let t=s;for(const a of k.slice(0,-1))t=t[a];t[k.at(-1)]=x;}"
                  + "return audioVideoAcceptance(s,r.scenario,r.seconds);})));" )
        result = subprocess.run([os.environ.get('CONNECT_NODE_BINARY', 'node'), '--input-type=module', '-e', script],
                                input=json.dumps(fixture), capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout), outputs)

    def test_javascript_acceptance_executes_all_thirty_four_adversarial_guards(self):
        script = pathlib.Path(__file__).with_name('acceptance_policy.test.mjs')
        result = subprocess.run([os.environ.get('CONNECT_NODE_BINARY', 'node'), '--test', '--test-reporter=tap', str(script)],
                                capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for count in ('# tests 34', '# pass 34', '# fail 0', '# cancelled 0', '# skipped 0'):
            self.assertIn(count + '\n', result.stdout)


    def test_actual_native_verdict_keeps_non_av_hard_gates_during_degradation(self):
        import ast
        import copy
        import types
        tree = ast.parse((pathlib.Path(__file__).parents[1] / 'check-video-paths.py').read_text())
        function = next(n for n in tree.body if isinstance(n, ast.AsyncFunctionDef) and n.name == '_qualify_path')
        expressions = [n.value for n in ast.walk(function) if isinstance(n, ast.Assign)
                       and any(isinstance(t, ast.Name) and t.id == 'passed' for t in n.targets)]
        self.assertEqual(len(expressions), 1)
        expression = compile(ast.Expression(expressions[0]), '<actual native route verdict>', 'eval')
        sync = self.sync(.3)
        sync['source_identity'] = {'status': 'passed'}
        participant = {'video': {'changing_fps': 5, 'motion_profile': {'status': 'passed'},
                                 'width': 640, 'height': 360, 'clean_decoder_diagnostics': False},
                       'audio': {'identifiable_tone': True, 'duration_seconds': 60},
                       'teardown_seconds': .1, 'participant_success': True, 'av_sync': sync,
                       'av_acceptance': self.policy.audio_video_acceptance(sync, 'impairment', 60),
                       'sdk_media': {'status': 'measured'}, 'maximum_rtp_bytes': 1180,
                       'maximum_encoded_frame_bytes': 1024, 'assembly': {'peak_pending_frames': 1, 'peak_queued_fragments': 1},
                       'playout': {'peak_video_frames': 1, 'peak_audio_frames': 1, 'maximum_hold_ms': 10},
                       'maximum_codec_emission_hold_ms': 10}
        observed = {'forwarding_diagnostics': {'status': 'measured'}, 'paired_termination': {'status': 'passed'},
                    'resources': {'within_limits': True}, 'network': {'within_limits': True}}
        def verdict(peer, route):
            return eval(expression, {'args': types.SimpleNamespace(impairment=True, mute_recovery=False, seconds=60),
                                     'file_containment': {'required_in_this_scenario': False},
                                     'native': [peer], 'observed': route})
        self.assertTrue(verdict(participant, observed))
        for path, value in [('video.changing_fps', .5), ('video.motion_profile.status', 'failed'),
                            ('audio.identifiable_tone', False), ('audio.duration_seconds', 1),
                            ('av_sync.source_identity.status', 'failed'), ('participant_success', False),
                            ('teardown_seconds', 6), ('sdk_media.status', 'missing'), ('maximum_rtp_bytes', 1181),
                            ('assembly.peak_queued_fragments', 65), ('playout.maximum_hold_ms', 201)]:
            peer = copy.deepcopy(participant); keys = path.split('.'); target = peer
            for key in keys[:-1]: target = target[key]
            target[keys[-1]] = value
            self.assertFalse(verdict(peer, observed), path)
        for key in ('resources', 'network'):
            route = copy.deepcopy(observed); route[key]['within_limits'] = False
            self.assertFalse(verdict(participant, route), key)
        route = copy.deepcopy(observed); route['paired_termination']['status'] = 'failed'
        self.assertFalse(verdict(participant, route))


if __name__ == '__main__': unittest.main()
