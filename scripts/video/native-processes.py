"""Owned process/resource/UDP runtime and public media diagnostic coverage.

Independent post-run content observers live in decoder_observation.py. The
facade preserves the qualification API while keeping runtime ownership here.
"""
import asyncio
import hashlib
import importlib.util
import json
import math
import pathlib
import subprocess
import threading
import time


_decoder_spec = importlib.util.spec_from_file_location('native_decoder_observation',
    pathlib.Path(__file__).with_name('decoder_observation.py'))
decoder = importlib.util.module_from_spec(_decoder_spec); _decoder_spec.loader.exec_module(decoder)
decode_video = decoder.decode_video
decode_audio = decoder.decode_audio
clock_observation = decoder.clock_observation
select_window = decoder.select_window
audiovisual_markers = decoder.audiovisual_markers
decode_recovery = decoder.decode_recovery
decoded_mute_content = decoder.decoded_mute_content
mute_observation = decoder.mute_observation
map_decoded_frames = decoder.map_decoded_frames
ivf_frame_timestamps = decoder.ivf_frame_timestamps
decode_hash_frames = decoder.decode_hash_frames
decode_gray_frames = decoder.decode_gray_frames
source_identity = decoder.source_identity


def digest(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()


def forwarding_diagnostics(snapshot, route):
    """Keep sampled occupancy and handoff timing distinct from decoded proof."""
    if not isinstance(snapshot, dict):
        return {'status': 'unobserved', 'reason': 'Selected runtime did not expose authenticated media diagnostics'}
    queues = snapshot.get('queues', []); forwarding = snapshot.get('forwarding', [])
    required_age = {'native-native': ['quic-video-egress'],
        'native-browser': ['quic-video-ingress', 'quic-video-egress', 'webrtc-video-ingress', 'webrtc-video-egress', 'webrtc-video-reorder'],
        'browser-browser': ['webrtc-video-ingress', 'webrtc-video-egress', 'webrtc-video-reorder']}[route]
    by_stage = {row['stage']: row for row in queues}
    missing_age = [stage for stage in required_age if by_stage.get(stage, {}).get('max_forwarded_frame_age_ms') is None]
    expected = 'core-video-fanout' if route == 'native-native' else 'core-video-bridge'
    active = [row for row in forwarding if row.get('stage') == expected and row.get('samples', 0) > 0]
    observed_video = any(row.get('kind') == 'video' and row.get('observations', 0) > 0 for row in queues)
    bounded = bool(queues) and all(row.get('kind') in ('audio', 'video')
        and row.get('unit') in ('frame-packets', 'track-events') and row.get('observations', 0) > 0
        and 0 < row.get('capacity_packets', 0) <= (10 if row['kind'] == 'audio' else 64)
        and 0 <= row.get('sampled_high_water_packets', -1) <= row['capacity_packets']
        and (row.get('max_forwarded_frame_age_ms') is None or
            (row.get('frame_age_boundary') == 'dequeue-before-handoff-or-write'
                and 0 <= row['max_forwarded_frame_age_ms'] <= 200)) for row in queues)
    timed = bool(active) and all(row.get('percentile_method') == 'log2-bucket-upper-bound'
        and 0 <= row.get('p50_us', -1) <= row.get('p95_us', -1) and row.get('max_us', -1) >= 0 for row in active)
    quic = snapshot.get('quic')
    quic_valid = route == 'browser-browser'
    if isinstance(quic, dict):
        minimum_budget = quic.get('min_video_rtp_budget_bytes') or 0
        maximum_budget = quic.get('max_video_rtp_budget_bytes') or 0
        if route != 'browser-browser':
            quic_valid = (quic.get('samples', 0) > 0 and quic.get('accepted_datagrams', 0) > 0
                and quic.get('video_budget_observations', 0) > 0
                and 0 < minimum_budget <= maximum_budget <= 1180)
        send_buffer = quic.get('send_buffer')
        if send_buffer is not None:
            quic_valid = (quic_valid and isinstance(send_buffer, dict)
                and send_buffer.get('capacity_bytes') == 76032
                and send_buffer.get('observations', 0) > 0
                and 0 <= send_buffer.get('minimum_available_bytes', -1)
                    <= send_buffer.get('maximum_available_bytes', -1)
                    <= send_buffer.get('capacity_bytes', -1)
                and 0 <= send_buffer.get('sampled_high_water_bytes', -1) <= send_buffer.get('capacity_bytes', -1))
    valid_shape = (snapshot.get('scope') == 'server-process'
        and snapshot.get('queue_depth_observation') == 'sampled-at-enqueue-and-pump-poll'
        and snapshot.get('forwarding_boundary') == 'adapter-frame-publication-to-core-destination-queue-handoff')
    return {'status': 'measured' if valid_shape and bounded and timed and observed_video and quic_valid and not missing_age else 'failed',
        'snapshot': snapshot, 'expected_active_forwarding_stage': expected,
        'sampled_queue_bounds_satisfied': bounded, 'active_handoff_timing_observed': timed,
        'quic_video_budget_applicable': route != 'browser-browser',
        'quic_video_budget_satisfied': quic_valid if route != 'browser-browser' else None,
        'quic_observations_consistent': quic_valid,
        'quic_send_buffer_status': 'observed' if isinstance(quic, dict) and quic.get('send_buffer') is not None else 'unobserved',
        'queue_age_unobserved_stages': [row['stage'] for row in queues if row.get('max_forwarded_frame_age_ms') is None],
        'required_application_video_age_stages': required_age,
        'required_application_video_age_unobserved_stages': missing_age,
        'acknowledged_queue_age_unobserved_stages': [row['stage'] for row in queues
            if row.get('max_forwarded_frame_age_ms') is None and row['stage'] not in required_age],
        'limits': {'video_queue_capacity': 64, 'audio_queue_capacity': 10, 'observed_dequeue_age_ms': 200,
            'quic_send_buffer_capacity_bytes_when_observed': 76032},
        'notes': ['Queue depth is sampled at enqueue and pump poll; hard capacities are reported separately.',
            'Dequeue age excludes waiting inside write, network and decoder. Successful Core handoff duration includes Core queue waiting; no additional handoff-duration target was frozen.',
            'Native/native publisher ingress is a non-forwarding shadow queue; its occupancy is measured but no forwarded-age sample is expected. Raw track events, audio and inactive peer-writer ages are reported separately from active application video forwarding.',
            'Missing ingress observations, handoff timeouts and typed QUIC/packetizer outcomes remain explicit; no zero-discard target was frozen. Quinn acceptance does not establish remote delivery.']}


def native_sdk_diagnostics(snapshot):
    if not isinstance(snapshot, dict):
        return {'status': 'unobserved', 'reason': 'Selected native example did not expose its public SDK snapshot'}
    queues = {row['stage']: row for row in snapshot.get('queues', [])}
    expected = {'quic-audio-ingress': 10, 'quic-audio-egress': 10,
        'quic-video-ingress': 64, 'quic-video-egress': 64}
    observed = all(stage in queues and queues[stage].get('observations', 0) > 0
        and queues[stage].get('capacity_packets') == capacity
        and 0 <= queues[stage].get('sampled_high_water_packets', -1) <= capacity
        for stage, capacity in expected.items())
    age = all(queues.get(stage, {}).get('max_forwarded_frame_age_ms') is not None
        and 0 <= queues[stage]['max_forwarded_frame_age_ms'] <= 200
        for stage in ['quic-video-ingress', 'quic-video-egress'])
    quic = snapshot.get('quic') or {}; buffer = quic.get('send_buffer') or {}
    budget = (quic.get('video_budget_observations', 0) > 0 and quic.get('accepted_datagrams', 0) > 0
        and 0 < (quic.get('min_video_rtp_budget_bytes') or 0) <= (quic.get('max_video_rtp_budget_bytes') or 0) <= 1180)
    bytes_valid = (buffer.get('observations', 0) > 0 and buffer.get('capacity_bytes') == 76032
        and 0 <= buffer.get('minimum_available_bytes', -1) <= buffer.get('maximum_available_bytes', -1) <= 76032
        and 0 <= buffer.get('sampled_high_water_bytes', -1) <= 76032)
    shape = (snapshot.get('scope') == 'native-session-handle'
        and snapshot.get('queue_depth_observation') == 'sampled-at-sdk-send-receive-and-pump-poll'
        and snapshot.get('ingress_age_boundary') == 'client-adapter-local-receipt-to-sdk-receive'
        and snapshot.get('egress_age_boundary') == 'dequeue-before-handoff-or-write')
    return {'status': 'measured' if shape and observed and age and budget and bytes_valid else 'failed',
        'snapshot': snapshot, 'sampled_packet_queue_bounds_satisfied': observed,
        'video_queue_age_satisfied': age, 'video_datagram_budget_satisfied': budget,
        'sampled_quic_byte_buffer_satisfied': bytes_valid,
        'notes': ['Per-handle client observations are separate from authenticated server-process Health metrics.',
            'Ingress is sampled at SDK receive/poll; full enqueue and exact ingress event peak are not observed. Sender polling includes reserved and queued slots.',
            'Client incoming age begins at its adapter local receipt, excluding network and independent decoder completion time. Quinn internal evictions and queued age remain unobservable.',
            'Typed pressure/discard/send outcomes are retained without invented zero-count acceptance limits.']}


class MediaProcess:
    def __init__(self, binary, config, conversation, session, peer, work, seconds,
                 native_peer=False, tone=440, extra=(), ffmpeg='ffmpeg', source_id=0):
        self.work = pathlib.Path(work); self.work.mkdir(mode=0o700)
        self.video = self.work / 'incoming.ivf'; self.audio = self.work / 'incoming.pcmu'
        self.report = self.work / 'observation.json'
        self.log = (self.work / 'process.log').open('w')
        command = [str(binary), '--config', str(config), '--conversation', conversation,
            '--ffmpeg', str(ffmpeg),
            '--session', session, '--peer', peer, '--seconds', str(seconds), '--input', 'test-pulse',
            '--audio-input', f'pulse:{tone}', '--fixture-marker', str(source_id), '--output', str(self.video),
            '--audio-output', str(self.audio), '--report', str(self.report), *extra]
        if native_peer: command.append('--native-peer')
        self.process = subprocess.Popen(command, stdout=self.log, stderr=self.log)

    async def wait(self, timeout):
        try:
            code = await asyncio.wait_for(asyncio.to_thread(self.process.wait), timeout)
            if code: raise RuntimeError('Native participant failed; retained private process.log')
        finally:
            self.close()
        return json.loads(self.report.read_text())

    def close(self):
        if self.process.poll() is None:
            self.process.kill(); self.process.wait(timeout=5)
        if not self.log.closed: self.log.close()


class ResourceSampler:
    """Sample selected owned PIDs and their descendants, including checker overhead."""
    def __init__(self, pids):
        self.pids = pids; self.samples = []; self.stop = threading.Event(); self.sampling_errors = {}

    def __enter__(self):
        self.started = time.monotonic()
        def observe():
            while not self.stop.is_set():
                cycle_began = time.monotonic()
                try:
                    output = subprocess.check_output(['ps', '-axo', 'pid=,ppid=,rss=,%cpu='], text=True, timeout=2)
                    rows = [(int(pid), int(parent), int(rss), float(cpu)) for pid, parent, rss, cpu in
                        (line.split() for line in output.splitlines() if line.strip())]
                    selected = set(self.pids)
                    while True:
                        children = {pid for pid, parent, _, _ in rows if parent in selected}
                        if children <= selected: break
                        selected |= children
                    self.samples.append({'seconds': time.monotonic(), 'rss_bytes':
                        sum(rss * 1024 for pid, _, rss, _ in rows if pid in selected),
                        'cpu_percent': sum(cpu for pid, _, _, cpu in rows if pid in selected),
                        'processes': len([pid for pid, _, _, _ in rows if pid in selected])})
                except (OSError, ValueError, subprocess.SubprocessError) as error:
                    name = type(error).__name__
                    self.sampling_errors[name] = self.sampling_errors.get(name, 0) + 1
                self.stop.wait(max(0, .5 - (time.monotonic() - cycle_began)))
        self.worker = threading.Thread(target=observe, daemon=True); self.worker.start(); return self

    def __exit__(self, *_):
        self.stop.set(); self.worker.join(timeout=3)
        if self.worker.is_alive(): raise RuntimeError('Resource sampler did not terminate')

    def report(self):
        now = time.monotonic(); values = list(self.samples); span = now - self.started
        gaps = [b['seconds'] - a['seconds'] for a, b in zip(values, values[1:])]
        first_lag = values[0]['seconds'] - self.started if values else span
        last_age = now - values[-1]['seconds'] if values else span
        observed = values[-1]['seconds'] - values[0]['seconds'] if len(values) >= 2 else 0
        coverage = observed / span if span else 0
        maximum_gap = max(gaps + [first_lag, last_age])
        complete = len(values) >= max(2, math.floor(span / .5) * .9) and coverage >= .9 and maximum_gap <= 1.25
        return {'sampled_peak_rss_bytes': max((s['rss_bytes'] for s in values), default=None),
            'sampled_peak_processes': max((s['processes'] for s in values), default=None),
            'sampled_peak_cpu_percent': max((s['cpu_percent'] for s in values), default=None),
            'samples': len(values), 'interval_seconds': .5,
            'interval_note': 'configured cadence; actual gaps and coverage reported separately',
            'sampling_errors': {'count': sum(self.sampling_errors.values()), 'classes': dict(self.sampling_errors)},
            'measurement_span_seconds': span, 'observed_time_seconds': observed, 'observed_time_coverage': coverage,
            'first_sample_lag_seconds': first_lag, 'last_sample_age_seconds': last_age,
            'maximum_sampling_gap_seconds': maximum_gap, 'coverage_complete': complete,
            'coverage_requirements': {'minimum_time_fraction': .9, 'minimum_expected_sample_fraction': .9,
                'maximum_gap_seconds': 1.25},
            'scope': 'owned checker/server/native/browser launcher PIDs and descendants, including FFmpeg and Chrome; sampled, not exact kernel high-water',
            'metric_note': 'ps CPU is its platform process-average percentage; not a hardware instantaneous CPU counter'}


class DatagramImpairment:
    """Loopback opaque UDP proxy, deterministic 3% drop/0–100ms delay.

    This impairs transport packets (including reliable control), not a claimed RTP
    packet percentage. Both directions have independent measured packet counters.
    """
    def __init__(self, target, seed=511, drop_probability=.03, maximum_delay=.1, bursts=()):
        import random
        if target[0] != '127.0.0.1': raise ValueError('Select an owned IPv4 loopback UDP target')
        self.target = target; self.random = random.Random(seed)
        self.drop_probability = drop_probability; self.maximum_delay = maximum_delay; self.bursts = bursts
        self.counters = {name: {'received': 0, 'dropped': 0, 'forwarded': 0,
            'received_bytes': 0, 'forwarded_bytes': 0,
            'received_network_bytes': 0, 'forwarded_network_bytes': 0,
            'maximum_planned_delay_ms': 0} for name in ('to_server', 'to_client')}
        self.wire_seconds = {name: {} for name in self.counters}
        self.pending = 0; self.high_water = 0; self.handles = set()
        self.backends = {}; self.waiting = {}; self.tasks = set(); self.closed = False; self.errors = []

    def receive(self, data, name):
        counter = self.counters[name]; counter['received'] += 1
        counter['received_bytes'] += len(data)
        # Owned AF_INET loopback sockets use ordinary 20-byte IPv4 + 8-byte UDP headers.
        network_bytes = len(data) + 28
        counter['received_network_bytes'] += network_bytes
        elapsed = time.monotonic() - self.start
        bucket = int(elapsed)
        self.wire_seconds[name][bucket] = self.wire_seconds[name].get(bucket, 0) + network_bytes
        return time.monotonic()

    def schedule(self, data, name, transport, destination, received_at=None):
        if received_at is None: received_at = self.receive(data, name)
        counter = self.counters[name]; network_bytes = len(data) + 28
        elapsed = time.monotonic() - self.start
        stalled = any(at <= elapsed < at + duration for at, duration in self.bursts)
        if self.closed or stalled:
            counter['dropped'] += 1
            reason = 'closed_dropped' if self.closed else 'stall_dropped'
            counter[reason] = counter.get(reason, 0) + 1; return
        counter['random_drop_decisions'] = counter.get('random_drop_decisions', 0) + 1
        if self.random.random() < self.drop_probability:
            counter['dropped'] += 1; counter['random_dropped'] = counter.get('random_dropped', 0) + 1; return
        if self.pending >= 512:
            counter['dropped'] += 1; counter['queue_overflow'] = counter.get('queue_overflow', 0) + 1; return
        delay = self.random.uniform(0, self.maximum_delay)
        counter['maximum_planned_delay_ms'] = max(counter.get('maximum_planned_delay_ms', 0), delay * 1000)
        self.pending += 1; self.high_water = max(self.pending, self.high_water)
        def forward():
            self.pending -= 1; self.handles.discard(handle)
            try:
                transport.sendto(data, destination); counter['forwarded'] += 1
                counter['forwarded_bytes'] += len(data)
                counter['forwarded_network_bytes'] += network_bytes
                dispatched_ms = (time.monotonic() - received_at) * 1000
                counter['dispatch_delay_samples'] = counter.get('dispatch_delay_samples', 0) + 1
                counter['dispatch_delay_total_ms'] = counter.get('dispatch_delay_total_ms', 0) + dispatched_ms
                counter['maximum_observed_dispatch_delay_ms'] = max(counter.get('maximum_observed_dispatch_delay_ms', 0), dispatched_ms)
            except Exception as error:
                if len(self.errors) < 64: self.errors.append(type(error).__name__)
        handle = asyncio.get_running_loop().call_later(delay, forward); self.handles.add(handle)

    async def __aenter__(self):
        self.start = time.monotonic(); self.start_unix = time.time()
        owner = self
        async def attach(address):
            class Backend(asyncio.DatagramProtocol):
                def datagram_received(self, data, source):
                    if source == owner.target:
                        owner.schedule(data, 'to_client', owner.transport, address)
            transport, _ = await asyncio.get_running_loop().create_datagram_endpoint(Backend,
                local_addr=('127.0.0.1', 0))
            if owner.closed: transport.close(); return
            owner.backends[address] = transport
            for data, received_at in owner.waiting.pop(address):
                owner.schedule(data, 'to_server', transport, owner.target, received_at)
        def completed(task):
            owner.tasks.discard(task)
            if not task.cancelled():
                error = task.exception()
                if error is not None and len(owner.errors) < 64:
                    owner.errors.append(type(error).__name__)
        class Proxy(asyncio.DatagramProtocol):
            def connection_made(self, transport): owner.transport = transport
            def datagram_received(self, data, address):
                received_at = owner.receive(data, 'to_server')
                backend = owner.backends.get(address)
                if backend is not None:
                    owner.schedule(data, 'to_server', backend, owner.target, received_at); return
                if address not in owner.waiting:
                    if len(owner.waiting) + len(owner.backends) >= 128:
                        owner.counters['to_server']['backend_overflow'] = owner.counters['to_server'].get('backend_overflow', 0) + 1
                        owner.counters['to_server']['dropped'] += 1
                        return
                    owner.waiting[address] = []
                    task = asyncio.create_task(attach(address)); owner.tasks.add(task)
                    task.add_done_callback(completed)
                if len(owner.waiting[address]) < 64: owner.waiting[address].append((data, received_at))
                else:
                    owner.counters['to_server']['startup_queue_overflow'] = owner.counters['to_server'].get('startup_queue_overflow', 0) + 1
                    owner.counters['to_server']['dropped'] += 1
        await asyncio.get_running_loop().create_datagram_endpoint(Proxy, local_addr=('127.0.0.1', 0))
        self.address = self.transport.get_extra_info('sockname'); return self

    async def __aexit__(self, *_):
        self.closed = True
        for handle in self.handles: handle.cancel()
        for task in self.tasks: task.cancel()
        if self.tasks: await asyncio.gather(*self.tasks, return_exceptions=True)
        for backend in self.backends.values(): backend.close()
        self.transport.close()

    def report(self):
        elapsed = time.monotonic() - self.start
        return {'scope': 'owned opaque UDP packets, bidirectional; enclosing hop report identifies QUIC or WebRTC transport',
            'elapsed_seconds': elapsed, 'configured_drop_probability': self.drop_probability,
            'drop_method': 'fixed-seed Bernoulli configured probability per eligible opaque UDP packet; actual realized fraction is reported separately from stall/overflow drops',
            'configured_delay_range_ms': [0, self.maximum_delay * 1000],
            'dispatch_delay_boundary': 'owned proxy datagram receipt to sendto return, including backend setup and event-loop scheduling; no receiver arrival or network delay claim',
            'mean_observed_dispatch_delay_ms': {name: counter.get('dispatch_delay_total_ms', 0) /
                counter['dispatch_delay_samples'] if counter.get('dispatch_delay_samples') else None
                for name, counter in self.counters.items()},
            'stall_bursts': [{'start_unix_seconds': self.start_unix + at,
                'end_unix_seconds': self.start_unix + at + duration, 'duration_seconds': duration}
                for at, duration in self.bursts],
            'received_wire_bitrate_bits_per_second': {name: counter['received_network_bytes'] * 8 / elapsed
                for name, counter in self.counters.items()},
            'maximum_one_second_received_wire_bits': {name: max(buckets.values(), default=0) * 8
                for name, buckets in self.wire_seconds.items()},
            'wire_peak_method': 'nonoverlapping one-second bins anchored to proxy start; encrypted UDP payload plus ordinary IPv4/UDP headers (28 bytes per packet); loopback has no Ethernet framing',
            'measured_random_drop_fraction': {name: counter.get('random_dropped', 0) /
                max(1, counter.get('random_drop_decisions', 0)) for name, counter in self.counters.items()},
            'pending_datagrams_high_water': self.high_water, 'pending_datagrams_bound': 512,
            'backend_sockets': len(self.backends), 'backend_sockets_bound': 128,
            'startup_queue_datagrams_per_backend_bound': 64,
            'errors': self.errors,
            'directions': self.counters}


def annotate_diagnostic_coverage(report):
    """Separate missing required measurements from acknowledged public API scope."""
    report['diagnostic_coverage'] = [{'path': path['path'],
        'status': path.get('forwarding_diagnostics', {}).get('status', 'unobserved'),
        'quic_send_buffer_status': path.get('forwarding_diagnostics', {}).get('quic_send_buffer_status', 'unobserved'),
        'queue_age_unobserved_stages': path.get('forwarding_diagnostics', {}).get('queue_age_unobserved_stages', [])}
        for path in report['paths']]
    for coverage in report['diagnostic_coverage']:
        if coverage['status'] == 'unobserved':
            report['unresolved'].append(coverage['path'] + ': authenticated media forwarding and queue observations not obtained')
        diagnostics = next(path.get('forwarding_diagnostics', {}) for path in report['paths'] if path['path'] == coverage['path'])
        missing_video = diagnostics.get('required_application_video_age_unobserved_stages', [])
        acknowledged = diagnostics.get('acknowledged_queue_age_unobserved_stages', [])
        coverage['required_application_video_age_unobserved_stages'] = missing_video
        coverage['acknowledged_queue_age_unobserved_stages'] = acknowledged
        if missing_video:
            report['unresolved'].append(coverage['path'] + ': application video dequeue age unobserved at ' + ', '.join(missing_video))
        if acknowledged:
            report['acknowledged_unmeasured_scope'].append(coverage['path'] + ': audio/raw events/non-forwarding shadow or inactive peer-writer queue age not measured at ' + ', '.join(acknowledged))
        quic_buffer = ((diagnostics.get('snapshot', {}).get('quic') or {}).get('send_buffer') or {})
        if quic_buffer.get('queued_age_observed') is False:
            report['acknowledged_unmeasured_scope'].append(coverage['path'] + ': opaque Quinn internal queued age and evictions are unobservable through its public buffer-space API')
        coverage['quic_media_send_buffer_applicable'] = coverage['path'] != 'browser-browser'
        if coverage['quic_media_send_buffer_applicable'] and coverage['quic_send_buffer_status'] == 'unobserved':
            report['unresolved'].append(coverage['path'] + ': QUIC send-buffer byte occupancy unobserved')
        if coverage['path'] != 'browser-browser':
            route = next(path for path in report['paths'] if path['path'] == coverage['path'])
            sdk_statuses = [participant.get('sdk_media', {}).get('status', 'unobserved')
                for participant in route.get('native', [])]
            coverage['native_sdk_packet_queue_high_water_status'] = 'measured' if sdk_statuses and all(status == 'measured' for status in sdk_statuses) else 'unobserved'
            if coverage['native_sdk_packet_queue_high_water_status'] != 'measured':
                report['unresolved'].append(coverage['path'] + ': native SDK incoming/outgoing audio/video packet queue high water/age/buffer proof missing or failed; assembler/playout metrics do not substitute')
