#!/usr/bin/env python3
"""Distinct-process public-interface AV proof against explicit local runtime inputs.

This checker never builds, loads private server/store internals or discovers an
ambient checkout. Assets/binaries are selected explicitly and hashed before/after.
"""
import argparse
import asyncio
import contextlib
import datetime
import importlib.util
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import traceback
import time

from lib.local_process import Server

HELPER = pathlib.Path(__file__).parent / 'video/native-processes.py'
PROCESS_HELPER = pathlib.Path(sys.modules[Server.__module__].__file__).resolve()
spec = importlib.util.spec_from_file_location('native_media_processes', HELPER)
media = importlib.util.module_from_spec(spec); spec.loader.exec_module(media)
WEBRTC_HELPER = pathlib.Path(__file__).parent / 'video/webrtc-impairment.py'
webrtc_spec = importlib.util.spec_from_file_location('webrtc_impairment', WEBRTC_HELPER)
webrtc = importlib.util.module_from_spec(webrtc_spec); webrtc_spec.loader.exec_module(webrtc)
DECODER_HELPER = pathlib.Path(media.decoder.__file__).resolve()
VISUAL_HELPER = pathlib.Path(__file__).parent / 'video/source_identity.mjs'
MARKER_PY = pathlib.Path(media.decoder.markers.__file__).resolve()
MARKER_JS = pathlib.Path(__file__).parent / 'video/marker_calibration.mjs'
AUDIO_PY = pathlib.Path(media.decoder.audio_observer.__file__).resolve()
AUDIO_JS = pathlib.Path(__file__).parent / 'video/audio_observation.mjs'
ENVELOPE_JS = pathlib.Path(__file__).parent / 'video/audio_envelope.mjs'
BROWSER_FAILURE_JS = pathlib.Path(__file__).parent / 'video/browser_failure.mjs'
MOTION_PY = pathlib.Path(media.decoder.motion_observer.__file__).resolve()
MOTION_JS = pathlib.Path(__file__).parent / 'video/motion_observation.mjs'
WORKER_JS = pathlib.Path(__file__).parent / 'video/encoded_worker_observation.mjs'
SPECTRUM_JS = pathlib.Path(__file__).parent / 'video/audio_spectrum.mjs'
FIXTURE_HELPER = pathlib.Path(__file__).parent / 'video/fixture_protocol.py'
fixture_spec = importlib.util.spec_from_file_location('media_fixture_protocol', FIXTURE_HELPER)
fixture = importlib.util.module_from_spec(fixture_spec); fixture_spec.loader.exec_module(fixture)
native_request = fixture.native_request
prepare = fixture.prepare
concurrent_file = fixture.concurrent_file


async def browser_process(args, signaling, tokens, work, expected, pids, bursts, conversation, session):
    browser = await asyncio.create_subprocess_exec(args.node, str(pathlib.Path(__file__).parent / 'video/browser-qualify.mjs'),
        stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
    pids.append(browser.pid)
    try:
        data = json.dumps({'root': str(args.source_path), 'signalingURL': signaling,
            'conversationId': conversation, 'sessionId': session,
            'tokens': tokens, 'seconds': args.seconds, 'expectedToneHz': expected,
            'sourceToneHz': [660] if len(tokens) == 1 else [440, 660],
            'sourceIds': [1] if len(tokens) == 1 else [0, 1],
            'expectedSourceIds': [0] if len(tokens) == 1 else [1, 0],
            'windowReadyPath': str(work / 'browser-window-ready.json'),
            'profileMode': 'mute' if args.mute_recovery else 'impairment' if args.impairment else 'clean',
            'muteRecovery': args.mute_recovery, 'networkBursts': bursts}).encode()
        out, err = await asyncio.wait_for(browser.communicate(data), args.seconds + 60)
        (work / 'browser.log').write_bytes(err)
        if not out: raise RuntimeError('Browser process failed before observations; see retained browser.log')
        observed = json.loads(out)
        if browser.returncode or observed['status'] != 'passed':
            (work / 'browser-result.json').write_text(json.dumps(observed, indent=2) + '\n')
            raise RuntimeError('Browser decoder profile failed; retained browser-result.json')
        return observed
    finally:
        if browser.returncode is None:
            browser.terminate()
            try: await asyncio.wait_for(browser.wait(), 5)
            except asyncio.TimeoutError: browser.kill(); await browser.wait()


async def measurement_readiness(participants, browser_ready, browser_count):
    """Start reliable load after actual receive-window readiness, not a timer."""
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        native_ready = all(media.decoder.live_keyframe_present(peer.video) for peer in participants)
        browser_starts = []
        if browser_count and browser_ready.is_file():
            try: browser_starts = json.loads(browser_ready.read_text())['starts_unix_seconds']
            except (json.JSONDecodeError, KeyError): browser_starts = []
        if native_ready and len(browser_starts) == browser_count:
            return {'observed_unix_seconds': time.time(), 'native_complete_keyframe_files': len(participants),
                'browser_window_starts_unix_seconds': browser_starts,
                'method': 'complete actual native retained keyframe plus browser independent observer window-start metadata; encoded readiness alone is not decoded acceptance'}
        await asyncio.sleep(.05)
    raise RuntimeError('Actual media receive-window readiness did not arrive within 30 seconds')


async def _qualify_path(args, route, work):
    work.mkdir(mode=0o700)
    state, identities, configs, conversation, session, signaling = prepare(work, args.server_binary)
    participants = []; sampler = None; observed = {'path': route, 'status': 'running', 'qualification_seconds': args.seconds, 'transport':
        'native=authenticated UCTP over QUIC; browser=authenticated WebRTC'}
    (work / 'result.json').write_text(json.dumps(observed, indent=2) + '\n')
    with Server(state, binary=args.server_binary, log_path=work / 'server.log') as server:
        try:
            names = [] if route == 'browser-browser' else ['reviews']
            if route == 'native-native': names += ['agent']
            async with contextlib.AsyncExitStack() as stack:
                rtc_proxy = None
                if args.impairment and route != 'native-native':
                    rtc_proxy = await stack.enter_async_context(webrtc.WebRtcImpairment(signaling,
                        server.process.pid, media.DatagramImpairment, args.seconds))
                    signaling = rtc_proxy.url
                host, p = configs['reviews']['endpoint'].rsplit(':', 1)
                impair_quic = args.impairment and route == 'native-native'
                bursts = [(args.seconds * .25, .25), (args.seconds * .6, .25)] if impair_quic else []
                proxy = await stack.enter_async_context(media.DatagramImpairment((host, int(p)),
                    drop_probability=.03 if impair_quic else 0, maximum_delay=.1 if impair_quic else 0,
                    bursts=bursts))
                for name in ['reviews', 'agent']:
                    config = state / (name + '-impaired.toml')
                    config.write_text(re.sub(r'endpoint = "[^"]+"', f'endpoint = "{proxy.address[0]}:{proxy.address[1]}"',
                        (state / (name + '.toml')).read_text()))
                for index, name in enumerate(names):
                    peer = 'agent' if name == 'reviews' else 'reviews'
                    config = state / (name + ('-impaired.toml' if name == 'reviews' else '.toml'))
                    participation_grace = 30 if route == 'native-browser' else 20 if index == 1 else 6
                    mute_flags = ['--camera-mute-at', '10', '--camera-mute-for', '2',
                        '--microphone-mute-at', '14', '--microphone-mute-for', '2'] if args.mute_recovery else []
                    participants.append(media.MediaProcess(args.video_binary, config, conversation, session,
                        identities[peer], work / name, args.seconds + participation_grace, route == 'native-native', 440 + index * 220,
                        extra=mute_flags, ffmpeg=args.ffmpeg, source_id=index))
                pids = [os.getpid(), server.process.pid] + [p.process.pid for p in participants]
                sampler = media.ResourceSampler(pids); sampler.__enter__()
                jobs = [p.wait(args.seconds + 70) for p in participants]
                if route != 'native-native':
                    tokens = [configs['agent']['credential']] if route == 'native-browser' else [configs[n]['credential'] for n in ('reviews', 'agent')]
                    jobs.append(browser_process(args, signaling, tokens, work, [440] if route == 'native-browser' else [660, 440], pids,
                        rtc_proxy.bursts() if rtc_proxy else proxy.report()['stall_bursts'], conversation, session))
                jobs.append(concurrent_file(state, conversation, args.server_binary, True,
                    measurement_readiness(participants, work / 'browser-window-ready.json',
                        0 if route == 'native-native' else 1 if route == 'native-browser' else 2)))
                running = [asyncio.create_task(job) for job in jobs]
                try:
                    results = await asyncio.gather(*running)
                except BaseException:
                    observed['resources'] = sampler.freeze_live()
                    # Preserve authenticated numeric evidence while the server is
                    # still alive, even when a participant failed validation.
                    # A failed health probe must not replace the original failure.
                    try:
                        health = native_request(state, 'reviews', {'op': 'health'}, args.server_binary)
                        observed['forwarding_diagnostics'] = media.forwarding_diagnostics(
                            health.get('readiness', {}).get('media'), route)
                    except Exception as diagnostic_error:
                        observed['forwarding_diagnostics'] = {
                            'status': 'unobserved',
                            'reason': 'Authenticated post-failure health probe failed',
                            'error_class': type(diagnostic_error).__name__}
                    observed['network'] = proxy.report()
                    if rtc_proxy: observed['webrtc_network'] = rtc_proxy.report()
                    browser_report = work / 'browser-result.json'
                    if browser_report.exists(): observed['browser'] = json.loads(browser_report.read_text())
                    observed['native_partial'] = []
                    for participant in participants:
                        if participant.report.exists() and participant.report.stat().st_size:
                            numeric = json.loads(participant.report.read_text())
                            summary = {key: value for key, value in numeric.items() if key not in ('video', 'audio')}
                            summary.update(video_samples=len(numeric['video']), audio_samples=len(numeric['audio']))
                            observed['native_partial'].append(summary)
                    (work / 'result.json').write_text(json.dumps(observed, indent=2) + '\n')
                    raise
                finally:
                    for task in running: task.cancel()
                    await asyncio.gather(*running, return_exceptions=True)
                observed['resources'] = sampler.freeze_live()
                observed['resources']['limits'] = {'sampled_peak_rss_bytes': 2 * 1024**3, 'sampled_peak_cpu_percent': 800}
                observed['resources']['within_limits'] = (observed['resources']['coverage_complete']
                    and observed['resources']['sampled_peak_rss_bytes'] <= 2 * 1024**3
                    and observed['resources']['sampled_peak_cpu_percent'] <= 800)
                observed['concurrent_reliable_file'] = results[-1]
                health = native_request(state, 'reviews', {'op': 'health'}, args.server_binary)
                observed['forwarding_diagnostics'] = media.forwarding_diagnostics(health.get('readiness', {}).get('media'), route)
                if route == 'native-native':
                    first, second = results[:2]
                    lag = ((second['unix_start_seconds'] + second['elapsed_seconds']) -
                        (first['unix_start_seconds'] + first['elapsed_seconds']))
                    observed['paired_termination'] = {'lag_seconds': lag, 'limit_seconds': 5,
                        'first_end_reason': first['end_reason'], 'second_end_reason': second['end_reason'],
                        'staggered_deadlines_seconds': [args.seconds + 6, args.seconds + 20],
                        'status': 'passed' if first['end_reason'] == 'deadline' and second['end_reason'] == 'peer-terminal'
                            and abs(lag) <= 5 else 'failed'}
                elif route == 'native-browser':
                    native_end = results[0]['unix_start_seconds'] + results[0]['elapsed_seconds']
                    lag = native_end - results[1]['ended_unix_seconds']
                    observed['paired_termination'] = {'lag_seconds': lag, 'limit_seconds': 5,
                        'native_end_reason': results[0]['end_reason'],
                        'status': 'passed' if results[0]['end_reason'] == 'peer-terminal' and abs(lag) <= 5 else 'failed'}
                observed['network'] = proxy.report()
                observed['network']['hop_role'] = 'native AV plus concurrent reliable file' if names else 'concurrent reliable file only; browser AV uses WebRTC'
                observed['network']['wire_bitrate_ceiling_bits_per_second_per_direction'] = 8_000_000
                observed['network']['within_limits'] = not observed['network']['errors'] and all(rate <= 8_000_000 for rate in
                    list(observed['network']['received_wire_bitrate_bits_per_second'].values()) +
                    list(observed['network']['maximum_one_second_received_wire_bits'].values()))
                if rtc_proxy:
                    observed['webrtc_network'] = rtc_proxy.report()
                    observed['webrtc_network']['selected_media_route'] = rtc_proxy.verify_selected_paths(results[len(participants)]['observations'])
                native = []
                observed['native'] = native
                if route != 'native-native': observed['browser'] = results[len(participants)]
                for index, (participant, result) in enumerate(zip(participants, results)):
                    video, audio, window = media.select_window(participant, result, args.seconds)
                    transitions = []
                    if args.mute_recovery:
                        sender = results[1 - index] if route == 'native-native' else None
                        transitions = ([dict(event, unix_seconds=sender['unix_start_seconds'] + event['seconds'])
                            for event in sender['mute_transitions']] if sender else results[len(participants)]['observations'][0]['mute_transitions'])
                    native.append({'window': window,
                        'window_start_unix_seconds': result['unix_start_seconds'] + window['start_emit_seconds'],
                        'window_end_unix_seconds': result['unix_start_seconds'] + window['start_emit_seconds'] + args.seconds, 'video': media.decode_video(video, args.seconds, args.ffmpeg, result, window),
                        'audio': media.decode_audio(audio, 660 if index == 0 else 440, args.ffmpeg),
                        'av_sync': media.audiovisual_markers(video, audio, result, window, args.ffmpeg,
                            expected_source_id=1 if index == 0 else 0, mute_transitions=transitions),
                        'rtp_clocks': media.clock_observation(result), 'teardown_seconds': result['teardown_seconds'],
                        'maximum_rtp_bytes': result['maximum_rtp_bytes'],
                        'maximum_encoded_frame_bytes': result['maximum_encoded_frame_bytes'],
                        'assembly': result['assembly'],
                        'sdk_media': media.native_sdk_diagnostics(result.get('sdk_media')),
                        'playout': result['playout'],
                        'maximum_codec_emission_hold_ms': result['maximum_codec_emission_hold_ms'],
                        'participant_success': result['success']})
                    if args.impairment:
                        native[-1]['recovery'] = media.decode_recovery(video, result, window,
                            rtc_proxy.bursts() if rtc_proxy else observed['network']['stall_bursts'], args.ffmpeg)
                    if args.mute_recovery:
                        content = media.decoded_mute_content(video, audio, result, window, args.ffmpeg)
                        native[-1]['mute'] = media.mute_observation(content, transitions, 660 if index == 0 else 440)
                    native[-1]['video']['motion_profile'] = media.decoder.motion_observer.motion_observation(
                        native[-1]['video']['motion_content'], 'mute' if args.mute_recovery else 'impairment' if args.impairment else 'clean',
                        transitions, rtc_proxy.bursts() if rtc_proxy else observed['network']['stall_bursts'])
                if args.mute_recovery and route != 'native-native':
                    browser_mute = []
                    for index, browser_observation in enumerate(observed['browser']['observations']):
                        if route == 'native-browser':
                            sender = results[0]
                            transitions = [dict(event, unix_seconds=sender['unix_start_seconds'] + event['seconds'])
                                for event in sender['mute_transitions']]
                        else:
                            transitions = observed['browser']['observations'][1 - index]['mute_transitions']
                        browser_observation['motion_profile'] = media.decoder.motion_observer.motion_observation(
                            browser_observation['motion_content'], 'mute', transitions)
                        browser_observation['audio_profile'] = media.decoder.audio_observer.audio_observation(
                            browser_observation['audio_content'], browser_observation['expected_audio_frequency_hz'], 'mute', transitions)
                        browser_mute.append(media.mute_observation(browser_observation['mute_content'], transitions,
                            browser_observation['expected_audio_frequency_hz']))
                    observed['browser']['mute'] = browser_mute
                windows = [(row['window_start_unix_seconds'], row['window_end_unix_seconds']) for row in native]
                windows += [(row['window_start_unix_seconds'], row['window_end_unix_seconds'])
                    for row in observed.get('browser', {}).get('observations', [])]
                file_containment = fixture.transfer_containment(observed['concurrent_reliable_file'], windows)
                file_containment['required_in_this_scenario'] = not args.impairment and not args.mute_recovery
                observed['concurrent_reliable_file']['measurement_window_containment'] = file_containment
                # Operating-profile decision stays frozen before content analysis.
                observed['postprocessing_resources'] = sampler.postprocessing_report()
                observed['resources'] = sampler.report()
                observed['resources']['limits'] = {'sampled_peak_rss_bytes': 2 * 1024**3, 'sampled_peak_cpu_percent': 800}
                observed['resources']['within_limits'] = (observed['resources']['coverage_complete']
                    and observed['resources']['sampled_peak_rss_bytes'] <= 2 * 1024**3
                    and observed['resources']['sampled_peak_cpu_percent'] <= 800)
                passed = (not file_containment['required_in_this_scenario'] or file_containment['status'] == 'passed') and observed['forwarding_diagnostics']['status'] == 'measured' and observed.get('paired_termination', {}).get('status', 'passed') == 'passed' and observed['resources']['within_limits'] and observed['network'].get('within_limits', True) and all(p['video']['changing_fps'] >= (1 if args.impairment or args.mute_recovery else 24) and (args.mute_recovery or p['audio']['identifiable_tone'])
                    and p['video']['motion_profile']['status'] == 'passed' and p['video']['width'] == 640 and p['video']['height'] == 360
                    and (args.impairment or args.mute_recovery or p['video']['clean_decoder_diagnostics'])
                    and p['audio']['duration_seconds'] >= args.seconds * .8 and p['teardown_seconds'] < 5
                    and p['participant_success'] and p['av_sync']['status'] == 'passed'
                    and p['av_sync']['source_identity']['status'] == 'passed'
                    and p['sdk_media']['status'] == 'measured'
                    and p['maximum_rtp_bytes'] <= 1180 and p['maximum_encoded_frame_bytes'] <= 256 * 1024
                    and p['assembly']['peak_pending_frames'] <= 8 and p['assembly']['peak_queued_fragments'] <= 64
                    and p['playout']['peak_video_frames'] <= 8 and p['playout']['peak_audio_frames'] <= 10
                    and p['playout']['maximum_hold_ms'] <= 200 and p['maximum_codec_emission_hold_ms'] <= 200 for p in native)
                if args.mute_recovery:
                    muted = all(row['mute']['status'] == 'passed' for row in native) and all(
                        row['status'] == 'passed' for row in observed.get('browser', {}).get('mute', [])) and all(
                        row['audio_profile']['status'] == 'passed' and row['motion_profile']['status'] == 'passed' for row in observed.get('browser', {}).get('observations', []))
                    observed['status'] = 'passed-mute-recovery' if passed and muted else 'failed'
                elif args.impairment:
                    recovered = all(p['recovery']['status'] == 'passed' for p in native)
                    if route == 'native-native':
                        recovered = recovered and all(counter.get('random_dropped', 0) > 0
                            for counter in observed['network']['directions'].values())
                    if rtc_proxy:
                        recovered = recovered and observed['webrtc_network']['selected_media_route']['status'] == 'passed' and not observed['webrtc_network']['errors'] and all(
                            not relay['errors'] for relay in observed['webrtc_network']['relays']) and any(
                            relay['configured_drop_probability'] == .03 and all(counter['received'] > 0 and counter.get('random_dropped', 0) > 0 for counter in relay['directions'].values())
                            for relay in observed['webrtc_network']['relays'])
                    observed['status'] = ('passed-impairment' if args.seconds >= 60 else 'passed-impairment-smoke') if passed and recovered else 'failed'
                else:
                    observed['status'] = ('passed-clean' if args.seconds >= 60 else 'passed-smoke') if passed else 'failed'
                (work / 'result.json').write_text(json.dumps(observed, indent=2) + '\n')
        finally:
            # Preserve completed measurements even when window selection or
            # decoding fails. The outer handler retains the failed verdict.
            try:
                (work / 'result.json').write_text(json.dumps(observed, indent=2) + '\n')
            finally:
                for participant in participants: participant.close()
                if sampler: sampler.__exit__(None, None, None)
        continuity = native_request(state, 'reviews', {'op': 'session_get', 'conversation_id': conversation,
            'session_id': session}, args.server_binary)
        if continuity['conversation_id'] != conversation: raise RuntimeError('Session conversation changed')
        observed['durable_conversation_preserved'] = True
    (work / 'result.json').write_text(json.dumps(observed, indent=2) + '\n')
    return observed


async def qualify_path(args, route, work):
    try:
        return await _qualify_path(args, route, work)
    except BaseException as error:
        observed = json.loads((work / 'result.json').read_text()) if (work / 'result.json').exists() else {'path': route}
        chain = []; cursor = error
        while cursor is not None:
            chain.append({'type': type(cursor).__name__, 'frames': [{'file': pathlib.Path(frame.filename).name,
                'line': frame.lineno, 'function': frame.name} for frame in traceback.extract_tb(cursor.__traceback__)]})
            cursor = cursor.__cause__ or cursor.__context__
        observed.update({'status': 'failed', 'error_class': type(error).__name__, 'error_chain': chain,
            'private_attempt_directory': str(work), 'error_frames': [{'file': pathlib.Path(frame.filename).name,
                'line': frame.lineno, 'function': frame.name} for frame in traceback.extract_tb(error.__traceback__)]})
        if work.exists(): (work / 'result.json').write_text(json.dumps(observed, indent=2) + '\n')
        return observed


async def explore_native_pairs(args, work):
    """Two simultaneous pairs on one server; observations do not expand support."""
    work.mkdir(mode=0o700)
    result = {'status': 'running', 'scope': 'exploratory', 'supported_profile_pairs': 1,
        'attempted_simultaneous_pairs': 2, 'seconds': args.seconds, 'participants': []}
    participants = []; sampler = None
    try:
        state, identities, configs, conversation, session, _ = prepare(work, args.server_binary)
        pairs = [(conversation, session)]
        with Server(state, binary=args.server_binary, log_path=work / 'server.log') as server:
            second_conversation = native_request(state, 'reviews', {'op': 'create', 'request_id': 'explore-create',
                'participants': [identities['agent']]}, args.server_binary)['id']
            second_session = native_request(state, 'reviews', {'op': 'session_invite', 'request_id': 'explore-invite',
                'conversation_id': second_conversation, 'participant_id': identities['agent'], 'media': ['audio', 'video']}, args.server_binary)['id']
            native_request(state, 'agent', {'op': 'session_accept', 'request_id': 'explore-accept',
                'conversation_id': second_conversation, 'session_id': second_session}, args.server_binary)
            pairs.append((second_conversation, second_session))
            host, target_port = configs['reviews']['endpoint'].rsplit(':', 1)
            async with media.DatagramImpairment((host, int(target_port)), drop_probability=0, maximum_delay=0) as proxy:
                for name in ('reviews', 'agent'):
                    (state / (name + '-impaired.toml')).write_text(re.sub(r'endpoint = "[^"]+"',
                        f'endpoint = "{proxy.address[0]}:{proxy.address[1]}"', (state / (name + '.toml')).read_text()))
                tones = [440, 660, 880, 1100]
                for pair_index, (cid, sid) in enumerate(pairs):
                    directory = work / f'pair-{pair_index + 1}'; directory.mkdir(mode=0o700)
                    for slot, name in enumerate(('reviews', 'agent')):
                        peer = 'agent' if name == 'reviews' else 'reviews'
                        config = state / (name + ('-impaired.toml' if slot == 0 else '.toml'))
                        participants.append(media.MediaProcess(args.video_binary, config, cid, sid, identities[peer],
                            directory / name, args.seconds + (6 if slot == 0 else 20), True,
                            tones[pair_index * 2 + slot], ffmpeg=args.ffmpeg, source_id=pair_index * 2 + slot))
                sampler = media.ResourceSampler([os.getpid(), server.process.pid] + [p.process.pid for p in participants])
                sampler.__enter__()
                jobs = [asyncio.create_task(p.wait(args.seconds + 70)) for p in participants]
                jobs.append(asyncio.create_task(concurrent_file(state, conversation, args.server_binary, True,
                    measurement_readiness(participants, work / 'unused-browser-ready.json', 0))))
                try:
                    completed = await asyncio.gather(*jobs)
                    result['concurrent_reliable_file'] = completed[-1]
                    result['resources'] = sampler.freeze_live()
                    for index, (participant, observation) in enumerate(zip(participants, completed)):
                        video, audio, window = media.select_window(participant, observation, args.seconds)
                        decoded = media.decode_video(video, args.seconds, args.ffmpeg, observation, window)
                        decoded['motion_profile'] = media.decoder.motion_observer.motion_observation(decoded['motion_content'])
                        result['participants'].append({'pair': index // 2 + 1, 'slot': index % 2 + 1,
                            'video': decoded,
                            'audio': media.decode_audio(audio, tones[index ^ 1], args.ffmpeg), 'window': window,
                            'success': observation['success'], 'teardown_seconds': observation['teardown_seconds'],
                            'end_reason': observation['end_reason'], 'assembly': observation['assembly'],
                            'playout': observation['playout']})
                    result['network'] = proxy.report()
                    result['status'] = 'observed-exploratory'
                    result['comparison'] = {'all_peer_moving_fps_at_least24': all(row['video']['changing_fps'] >= 24 for row in result['participants']),
                        'all_peer_motion_freshness_within2seconds': all(row['video']['motion_profile']['status'] == 'passed' for row in result['participants']),
                        'all_peer_tones_identified': all(row['audio']['identifiable_tone'] for row in result['participants']),
                        'all_teardown_under5seconds': all(row['teardown_seconds'] < 5 for row in result['participants'])}
                finally:
                    for job in jobs: job.cancel()
                    await asyncio.gather(*jobs, return_exceptions=True)
                    result['resources'] = sampler.report()
                    if hasattr(sampler, 'postprocessing_started'):
                        result['postprocessing_resources'] = sampler.postprocessing_report()
    except BaseException as error:
        result.update(status='failed', error_class=type(error).__name__, error_frames=[{
            'file': pathlib.Path(frame.filename).name, 'line': frame.lineno, 'function': frame.name}
            for frame in traceback.extract_tb(error.__traceback__)])
    finally:
        for participant in participants: participant.close()
        if sampler: sampler.__exit__(None, None, None)
        (work / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


def playwright_inputs(selected):
    """Bind the explicit module entry file; a directory is not a module proof."""
    module = pathlib.Path(selected).resolve(strict=True)
    if not module.is_file():
        raise ValueError('Select the Playwright module entry file, such as index.mjs')
    manifest = module.parent / 'package.json'
    return [module, manifest] if manifest.is_file() else [module]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runtime', required=True, type=pathlib.Path, help='Explicit installed runtime root')
    parser.add_argument('--source-path', type=pathlib.Path, help='Explicit public SDK/reference asset root; defaults to runtime')
    parser.add_argument('--server-binary', type=pathlib.Path, help='Explicit source-lane override; defaults to runtime/bin/connect')
    parser.add_argument('--video-binary', type=pathlib.Path, help='Explicit source-lane override; defaults to runtime/bin/connect-video')
    parser.add_argument('--source-revision', required=True)
    parser.add_argument('--work-root', required=True, type=pathlib.Path)
    parser.add_argument('--output', type=pathlib.Path)
    parser.add_argument('--path', action='append', choices=['native-native', 'native-browser', 'browser-browser'])
    parser.add_argument('--seconds', type=int, default=60)
    parser.add_argument('--node', default=os.environ.get('CONNECT_NODE_BINARY', 'node'))
    parser.add_argument('--ffmpeg', default=os.environ.get('CONNECT_FFMPEG_BINARY', 'ffmpeg'))
    parser.add_argument('--impairment', action='store_true', help='Owned UDP faults: configured 3%% drop probability/0–100ms added delay plus two stalls through one QUIC or WebRTC hop')
    parser.add_argument('--mute-recovery', action='store_true', help='Separate generated-source camera/microphone mute and decoded recovery scenario (at least 20 seconds)')
    parser.add_argument('--exploratory-concurrency', action='store_true', help='Observe two simultaneous native AV pairs on one server; does not extend supported single-pair profile')
    args = parser.parse_args()
    if not 1 <= args.seconds <= 120: parser.error('Duration must be 1–120 seconds')
    if args.mute_recovery and (args.seconds < 20 or args.impairment):
        parser.error('Mute recovery is a separate scenario requiring at least 20 seconds; do not combine with --impairment')
    if args.exploratory_concurrency and (args.mute_recovery or args.impairment or args.path):
        parser.error('Exploratory concurrency is a separate clean observation; do not combine scenarios or --path')
    args.runtime = args.runtime.resolve(strict=True)
    args.source_path = (args.source_path or args.runtime).resolve(strict=True)
    args.server_binary = (args.server_binary or args.runtime / 'bin/connect').resolve(strict=True)
    args.video_binary = (args.video_binary or args.runtime / 'bin/connect-video').resolve(strict=True)
    selected_ffmpeg = pathlib.Path(shutil.which(args.ffmpeg) or args.ffmpeg).resolve(strict=True)
    selected_node = pathlib.Path(shutil.which(args.node) or args.node).resolve(strict=True)
    args.ffmpeg = str(selected_ffmpeg); args.node = str(selected_node)
    # Binding includes every served public SDK/reference asset plus selected executable inputs.
    bound = [args.server_binary, args.video_binary, selected_ffmpeg, selected_node,
        pathlib.Path(sys.executable).resolve(strict=True),
        pathlib.Path(__file__), PROCESS_HELPER, HELPER, WEBRTC_HELPER, DECODER_HELPER, FIXTURE_HELPER, VISUAL_HELPER, MARKER_PY, MARKER_JS, AUDIO_PY, AUDIO_JS, ENVELOPE_JS, BROWSER_FAILURE_JS, MOTION_PY, MOTION_JS, WORKER_JS, SPECTRUM_JS,
        pathlib.Path(__file__).parent / 'video/browser-qualify.mjs']
    if os.environ.get('CONNECT_CHROME_BINARY'):
        bound.append(pathlib.Path(os.environ['CONNECT_CHROME_BINARY']).resolve(strict=True))
    if os.environ.get('CONNECT_PLAYWRIGHT_MODULE'):
        bound.extend(playwright_inputs(os.environ['CONNECT_PLAYWRIGHT_MODULE']))
    for folder in ['sdk/typescript/dist', 'examples/conversation-client']:
        directory = args.source_path / folder
        if not directory.is_dir(): parser.error('Missing selected public media assets: ' + folder)
        bound += sorted(p for p in directory.rglob('*') if p.is_file() and '__pycache__' not in p.parts
            and p.suffix not in ('.pyc', '.pyo'))
    hashes = {str(p): media.digest(p) for p in bound}
    os.umask(0o077)
    args.work_root.mkdir(parents=True, exist_ok=True)
    work = args.work_root / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    work.mkdir(mode=0o700)
    report = {'status': 'running', 'profile': 'connect-video-vp8/1', 'source_revision': args.source_revision,
        'marker_program': media.decoder.markers.PROGRAM,
        'scenario': 'exploratory-concurrency' if args.exploratory_concurrency else 'mute-recovery' if args.mute_recovery else 'impairment' if args.impairment else 'clean',
        'qualification_seconds': args.seconds,
        'selected_runtime': str(args.runtime), 'selected_assets': str(args.source_path), 'input_sha256': hashes,
        'paths': [], 'whole_gate_qualified': False,
        'limitations': ['Native content is independently decoded after participation, with exact decoded PTS/IVF/live arrival-emission mapping; native marker alignment and recovery refer to actual live emission times, not actual independent decoder completion latency. Browser content is observed at real decoded rendering/audio observation. Physical speaker/display latency and native live audio playback are not measured.'],
        'unresolved': [], 'acknowledged_unmeasured_scope': [],
        'separate_required_checks': [
            'credential revocation, restart, cancellation and ICE/QUIC failure cases outside this checker']}
    if not args.mute_recovery:
        report['separate_required_checks'].append('camera/microphone mute recovery is a separate --mute-recovery run')
    report['tool_versions'] = {'ffmpeg': subprocess.check_output([str(selected_ffmpeg), '-version'], text=True).splitlines()[0],
        'node': subprocess.check_output([str(selected_node), '--version'], text=True).strip(), 'python': sys.version}
    try:
        for route in ([] if args.exploratory_concurrency else args.path or ['native-native', 'native-browser', 'browser-browser']):
            print('RUN', route, file=sys.stderr, flush=True)
            report['paths'].append(asyncio.run(qualify_path(args, route, work / route)))
            if report['paths'][-1]['status'] == 'failed': raise RuntimeError('Clean decoded route failed')
        if args.exploratory_concurrency:
            report['exploratory'] = asyncio.run(explore_native_pairs(args, work / 'two-native-pairs'))
            if report['exploratory']['status'] == 'failed': raise RuntimeError('Exploratory two-pair observation failed; retained exact attempt')
        report['tested_inputs_stable'] = all(media.digest(p) == hashes[str(p)] for p in bound)
        if not report['tested_inputs_stable']: raise RuntimeError('Selected input changed during qualification')
        report['status'] = 'observed-exploratory' if args.exploratory_concurrency else 'passed-mute-recovery' if args.mute_recovery else ('passed-impairment' if args.seconds >= 60 else 'passed-impairment-smoke') if args.impairment else 'passed-clean' if args.seconds >= 60 else 'passed-smoke'
    except BaseException as error:
        report.update(status='failed', error_class=type(error).__name__, error_frames=[{
            'file': pathlib.Path(frame.filename).name, 'line': frame.lineno, 'function': frame.name}
            for frame in traceback.extract_tb(error.__traceback__)])
    finally:
        report['tested_inputs_stable'] = all(p.exists() and media.digest(p) == hashes[str(p)] for p in bound)
        if not report['tested_inputs_stable']: report.update(status='failed', error_class='SelectedInputChanged')
        media.annotate_diagnostic_coverage(report)
        if args.exploratory_concurrency:
            report['acknowledged_unmeasured_scope'].append('Exploratory two-pair run does not collect authenticated internal forwarding/queue diagnostics or qualify an additional supported capacity')
    (work / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))
    return 0 if report['status'] in ('passed-clean', 'passed-smoke', 'passed-impairment', 'passed-impairment-smoke', 'passed-mute-recovery', 'observed-exploratory') else 1


if __name__ == '__main__': sys.exit(main())
