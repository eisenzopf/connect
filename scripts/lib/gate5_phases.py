"""Named executable Gate 5 lanes. Existing child deadlines stay immutable."""
import sys
import uuid

from .gate4_phases import HOST_TESTS, phases as gate4_phases
from .local_process import ROOT
from .media_failure_proof import EXTERNAL_FAILURE_TESTS

LANES = ('source', 'independent-client', 'installed', 'scripted-host', 'actual-model', 'operating', 'native-keychain', 'native-example')
OPERATING = {'native-task-capacity', 'reference-runner-capacity', 'mcp-operating-load',
             'grant-lifecycle-load', 'task-artifact-retention', 'voice-setup-retirement'}
EXTERNAL_MEDIA_TESTS = {
    'native_to_native_decodes_duplex_moving_video_and_distinct_audio_for_sixty_seconds': 'gate5_video_decode',
    'browser_to_browser_decodes_duplex_moving_video_and_audio_for_sixty_seconds': 'gate5_browser_video',
    'native_process_and_browser_decode_duplex_av_through_public_interfaces': 'gate5_native_browser_video',
}
UPSTREAM_MEDIA_TEST_COUNTS = {
    'upstream-video-datagram-budget': 3,
    'workspace-quic-sustained-transport': 5,
    'upstream-webrtc-preallocation': 1,
    'upstream-webrtc-session-selection': 1,
    'upstream-media-diagnostics': 7,
    'upstream-video-packetizer': 9,
    'upstream-peer-media-ticket': 3,
    'native-media-diagnostics': 1,
    'upstream-webrtc-video-boundaries': 9,
    'upstream-webrtc-av-sync-identity': 1,
    'upstream-dtls-flight-retransmission': 2,
    'upstream-receiver-report-accounting': 16,
    'upstream-bbr-model-bounds': 9,
    'external-runtime-media-failures': 7,
    'upstream-terminal-session-retirement': 5,
    'upstream-legacy-revocation-guard': 1,
    'upstream-declared-ssrc-feedback': 2,
    'upstream-audio-reorder': 5,
    'upstream-repair-clock-interceptors': 134,
    'upstream-repair-clock-rtc': 210,
    'upstream-source-clock-reports': 56,
    'upstream-source-clock-traits': 2,
    'upstream-source-clock-routes': 5,
    'upstream-source-clock-webrtc': 22,
    'upstream-source-clock-activation': 3,
    'upstream-vp8-repair-profile': 7,
    'upstream-outbound-rtx-wire': 4,
    'upstream-track-ready-publication': 2,
    'upstream-track-ready-simulcast': 1,
}


def phases(lanes, codex_binary=None, candidate=None, runtime=None, qualification_report=None,
           allow_model_inference=False, *, independent_python=None, media_tools=None,
           source_revision=None, native_ffmpeg=None):
    """Returns actual commands plus explicit unresolved lane prerequisites.

    This is a coverage driver, not an acceptance declaration. Retained operating
    checks qualify their existing task/audio profiles, not unimplemented video.
    """
    steps = []
    unresolved = []
    cargo = ['cargo', 'test', '--locked', '-p', 'connect-server', '--all-features']
    for lane in lanes:
        selected = []
        if lane == 'source':
            selected = gate4_phases(source_only=True)
            # Preserve historical executable coverage and original deadlines;
            # Gate5 requires public catalog fetches rather than private history.
            selected = [(name, command + ['--public-source'] if name in
                         ('schema-six-build', 'schema-ten-build') else command, timeout)
                        for name, command, timeout in selected]
            selected.insert(1, ('gate5-driver-tests', [sys.executable, '-m', 'unittest', 'discover',
                              '-s', 'scripts/tests', '-p', 'test_gate5*.py'], 120))
            selected.insert(2, ('grant-boundary-tests', [sys.executable, '-m', 'unittest', 'discover',
                               '-s', 'scripts/tests', '-p', 'test_grant_boundary.py'], 120))
            selected.insert(3, ('video-measurement-tests', [sys.executable, '-m', 'unittest', 'discover',
                               '-s', 'scripts/tests', '-p', 'test_video_measurements.py'], 120))
            selected.insert(4, ('package-proof-tests', [sys.executable, '-m', 'unittest', 'discover',
                               '-s', 'scripts/tests', '-p', 'test_package_inventory.py'], 120))
            selected.insert(5, ('prepared-archive-tests', [sys.executable, '-m', 'unittest', 'discover',
                               '-s', 'scripts/tests', '-p', 'test_prepared_archive.py'], 120))
            selected.insert(6, ('connected-app-checker-tests', [sys.executable, '-m', 'unittest', 'discover',
                               '-s', 'scripts/tests', '-p', 'test_connected_app_checker.py'], 120))
            # Run the delivered app commands immediately after the ordinary
            # binaries are built. Preserve every attempt across driver reruns.
            app_evidence = ROOT / '.local/gate5/connected-app-public' / uuid.uuid4().hex
            at = next(i for i, row in enumerate(selected) if row[0] == 'current-binaries') + 1
            selected.insert(at, ('connected-app-public-journey', [sys.executable,
                'scripts/check-connected-app.py', '--source-revision', str(source_revision),
                '--server-binary', str(ROOT / 'target/debug/connect'),
                '--example-binary', str(ROOT / 'target/debug/connect-example'),
                '--work-root', str(app_evidence / 'work'),
                '--output', str(app_evidence / 'result.json')], 120))
            upstream = ['cargo', 'test', '--locked', '--manifest-path']
            webrtc = '.deps/rvoip/crates/webrtc/rvoip-webrtc/Cargo.toml'
            selected += [
                ('upstream-declared-ssrc-feedback', [*upstream,
                    '.deps/rvoip/crates/webrtc/rvoip-rtc/Cargo.toml',
                    '--lib', 'declared_feedback_tests'], 1200),
                ('upstream-terminal-session-retirement', [*upstream,
                    '.deps/rvoip/crates/uctp/rvoip-uctp/Cargo.toml',
                    '--lib', 'terminal_tests'], 1200),
                ('upstream-legacy-revocation-guard', [*upstream,
                    '.deps/rvoip/crates/uctp/rvoip-uctp/Cargo.toml',
                    '--lib', 'resource_guard_exits_when_a_bound_session_is_revoked'], 1200),
                ('native-media-diagnostics', [*cargo, '--test', 'gate5_native_diagnostics'], 1200),
                ('external-runtime-media-failures', [*cargo, '--test', 'gate5_installed_video_failures',
                    '--', '--ignored', '--nocapture', '--test-threads=1'], 1200),
                ('typescript-public-media-tests', ['node', '--test', '--test-reporter=tap',
                    *[str(path.relative_to(ROOT)) for path in sorted((ROOT / 'sdk/tests').glob('*.test.mjs'))]], 300),
                ('workspace-quic-sustained-transport', [*cargo, '--test', 'gate5_quic_transport',
                    '--', '--nocapture', '--test-threads=1'], 1200),
                ('upstream-video-datagram-budget', [*upstream,
                    '.deps/rvoip/crates/uctp/rvoip-quic/Cargo.toml', '--test', 'video_budget'], 1200),
                ('upstream-webrtc-preallocation', [*upstream, webrtc,
                    '--features', 'signaling-ws,client', '--test', 'selected_session_admission'], 1200),
                ('upstream-webrtc-session-selection', [*upstream, webrtc,
                    '--features', 'signaling-ws,client', '--lib', 'selected_av_offer'], 1200),
                ('upstream-bbr-model-bounds', [*upstream,
                    '.deps/quinn-proto/Cargo.toml', '--lib', 'congestion::bbr'], 1200),
                ('upstream-receiver-report-accounting', [*upstream,
                    '.deps/rtc-interceptor/Cargo.toml', '--lib', 'report::receiver_stream'], 1200),
                ('upstream-repair-clock-interceptors', [*upstream,
                    '.deps/rvoip/Cargo.toml', '-p', 'rtc-interceptor', '--lib'], 1200),
                ('upstream-repair-clock-rtc', [*upstream,
                    '.deps/rvoip/crates/webrtc/rvoip-rtc/Cargo.toml', '--lib'], 1200),
                ('upstream-source-clock-reports', [*upstream,
                    '.deps/rvoip/Cargo.toml', '-p', 'rtc-interceptor', '--lib', 'report::'], 1200),
                ('upstream-source-clock-traits', [*upstream,
                    '.deps/rvoip/crates/foundation/rvoip-core-traits/Cargo.toml', '--lib', 'source_clock'], 1200),
                ('upstream-source-clock-routes', [*upstream,
                    '.deps/rvoip/crates/foundation/rvoip-core/Cargo.toml', '--lib', 'source_clock'], 1200),
                ('upstream-source-clock-webrtc', [*upstream, webrtc,
                    '--features', 'signaling-ws,client', '--lib', 'source_clock'], 1200),
                ('upstream-source-clock-activation', [*upstream,
                    '.deps/rvoip/Cargo.toml', '-p', 'rvoip-rtc', '--lib', 'source_clock_activation'], 1200),
                ('upstream-vp8-repair-profile', [*upstream, webrtc,
                    '--features', 'signaling-ws,client', '--lib', 'vp8'], 1200),
                ('upstream-outbound-rtx-wire', [*upstream,
                    '.deps/rvoip/Cargo.toml', '-p', 'rvoip-webrtc-stack', '--lib', 'outbound_rtx'], 1200),
                ('upstream-track-ready-publication', [*upstream,
                    '.deps/rvoip/Cargo.toml', '-p', 'rvoip-webrtc-stack', '--lib', 'track_ready_'], 1200),
                ('upstream-track-ready-simulcast', [*upstream,
                    '.deps/rvoip/Cargo.toml', '-p', 'rvoip-webrtc-stack',
                    '--test', 'simulcast_rtc_to_webrtc_interop'], 1200),
                ('upstream-dtls-flight-retransmission', [*upstream,
                    '.deps/rtc-dtls/Cargo.toml', '--lib', 'connect_retransmission_tests'], 1200),
                ('upstream-webrtc-av-sync-identity', [*upstream, webrtc,
                    '--features', 'signaling-ws,client', '--lib',
                    'local_audio_video_sdp_shares_one_private_per_peer_sync_identity'], 1200),
                ('upstream-webrtc-video-boundaries', [*upstream, webrtc,
                    '--features', 'signaling-ws,client', '--lib', 'media::video_'], 1200),
                ('upstream-audio-reorder', [*upstream, webrtc,
                    '--features', 'signaling-ws,client', '--lib', 'media::audio_reorder'], 1200),
                ('upstream-media-diagnostics', [*upstream,
                    '.deps/rvoip/crates/foundation/rvoip-core-traits/Cargo.toml',
                    '--lib', 'media_diagnostics'], 1200),
                ('upstream-video-packetizer', [*upstream,
                    '.deps/rvoip/crates/foundation/rvoip-core/Cargo.toml',
                    '--lib', 'video_packetizer'], 1200),
                ('upstream-peer-media-ticket', [*upstream,
                    '.deps/rvoip/crates/foundation/rvoip-core-traits/Cargo.toml',
                    '--lib', 'peer_media::tests'], 1200),
            ]
            selected += [('connector-template-cold-build', ['cargo', 'build', '--locked', '--manifest-path',
                         'examples/connector-template/Cargo.toml', '--target-dir', '.local/gate5/connector-template-target'], 1200),
                         ('connector-template-contract', [sys.executable, 'scripts/check-connector-contract.py'], 300)]
            selected += [
                ('connector-contribution-cold-build', ['cargo', 'build', '--locked', '--manifest-path',
                    'examples/connector-contribution/Cargo.toml', '--target-dir', '.local/gate5/connector-contribution-target'], 1200),
                ('connector-contribution-contract', [sys.executable, 'scripts/check-connector-contract.py',
                    '--manifest-path', 'examples/connector-contribution/Cargo.toml',
                    '--output', '.local/gate5/connector-contribution-contract.json',
                    '--target-dir', '.local/gate5/connector-contribution-target'], 300),
                ('schema-one-public-build', [sys.executable, 'scripts/build-recovery-baseline.py',
                    '--schema', '1', '--public-source', '--retire-build-cache'], 7500),
                ('schema-sixteen-public-build', [sys.executable, 'scripts/build-recovery-baseline.py',
                    '--schema', '16', '--public-source', '--retire-build-cache'], 7500),
                ('gate5-historical-recovery', [sys.executable, 'scripts/check-gate5-recovery.py'], 660),
                ('gate5-public-setup', [sys.executable, 'scripts/check-gate5-setup.py',
                    '--binary', str(ROOT / 'target/debug/connect'),
                    '--work-root', '.local/gate5/setup-qualification',
                    '--output', '.local/gate5/setup-qualification.json'], 180),
            ]
        elif lane == 'independent-client':
            selected = [
                ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 600),
                ('independent-runtime', [sys.executable, 'scripts/prepare-independent-uctp.py'], 300),
                ('independent-vector-tests', [str(ROOT / '.local/independent-uctp/venv/bin/python'),
                                             '-m', 'unittest', 'discover', '-s', 'examples/independent-uctp-client',
                                             '-p', 'test_*.py'], 120),
                ('independent-cold-build', ['cargo', 'build', '--locked', '-p', 'connect-server',
                                            '--bin', 'connect'], 2400),
                ('independent-quic-conformance', [str(ROOT / '.local/independent-uctp/venv/bin/python'),
                                                'scripts/check-independent-uctp.py'], 120),
            ]
        elif lane == 'installed':
            if candidate is None or codex_binary is None or independent_python is None:
                unresolved.append({'lane': lane, 'reason': 'requires matching candidate, explicit installed Codex binary and pinned independent Python'})
            else:
                selected = [('installed-candidate', [sys.executable, 'scripts/check-package.py',
                    str(candidate), '--codex-binary', str(codex_binary), '--profile', 'gate5-local',
                    '--independent-python', str(independent_python)], 14400)]
                if media_tools and set(media_tools) == {'node', 'ffmpeg', 'chrome', 'playwright'}:
                    flags = {'node': 'node-binary', 'ffmpeg': 'ffmpeg-binary',
                             'chrome': 'chrome-binary', 'playwright': 'playwright-module'}
                    selected[0][1].extend(arg for key, flag in flags.items()
                                         for arg in ('--' + flag, str(media_tools[key])))
                else:
                    unresolved.append({'lane': lane, 'reason': 'installed video requires explicitly selected Node/FFmpeg/Chrome/Playwright inputs'})
        elif lane == 'scripted-host':
            if codex_binary is None:
                unresolved.append({'lane': lane, 'reason': 'requires explicit installed Codex binary'})
            else:
                selectors = [arg for name in HOST_TESTS for arg in ('--test', name)]
                selected = [('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 600),
                            ('host-cold-compile', [*cargo, *selectors, '--no-run'], 2400),
                            ('installed-host', [*cargo, *selectors, '--', '--ignored', '--test-threads=1'], 1800)]
        elif lane == 'actual-model':
            if not (allow_model_inference and runtime and qualification_report and codex_binary):
                unresolved.append({'lane': lane, 'reason': 'requires explicit inference selection, installed runtime, matching qualification report and Codex binary'})
            else:
                selected = [('actual-model-journey', [sys.executable, 'scripts/live/agent-journey.py', str(runtime),
                            '--qualification-report', str(qualification_report), '--codex-binary', str(codex_binary),
                            '--allow-model-inference', '--profile', 'gate5-local'], 1200)]
        elif lane == 'native-example':
            if native_ffmpeg is None:
                unresolved.append({'lane': lane, 'reason': 'Select an explicit --ffmpeg-binary for generated example interruption/refusal; no hardware is acquired'})
            else:
                selected = [
                    ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 600),
                    ('native-example-cold-build', ['cargo', 'build', '--locked', '-p', 'connect-server',
                        '-p', 'connect-native-video', '--bin', 'connect', '--bin', 'connect-video'], 2400),
                    ('native-example-cleanup-guards', [sys.executable, '-m', 'unittest', 'discover',
                        '-s', 'scripts/tests', '-p', 'test_native_video_cleanup.py'], 120),
                    ('native-example-cleanup', [sys.executable, 'scripts/check-native-video-cleanup.py',
                        '--runtime', str(ROOT), '--server-binary', str(ROOT / 'target/debug/connect'),
                        '--video-binary', str(ROOT / 'target/debug/connect-video'),
                        '--ffmpeg', str(native_ffmpeg), '--source-revision', str(source_revision),
                        '--work-root', '.local/gate5/native-example-cleanup',
                        '--output', '.local/gate5/native-example-cleanup.json'], 300),
                ]
        elif lane == 'native-keychain':
            selected = [('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 600),
                        ('keychain-cold-compile', [*cargo, '--test', 'secrets_native', '--no-run'], 2400),
                        ('isolated-native-keychain', [*cargo, '--test', 'secrets_native', 'isolated_native_keychain',
                                                     '--', '--ignored', '--nocapture'], 120)]
        elif lane == 'operating':
            selected = [('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 600),
                        ('operating-cold-build', ['cargo', 'build', '--locked', '-p', 'connect-server',
                         '-p', 'connect-review-loop', '--bin', 'connect', '--bin', 'connect-example'], 2400)]
            selected += [s for s in gate4_phases(source_only=True) if s[0] in OPERATING]
            if not (runtime and source_revision and media_tools
                    and set(media_tools) == {'node', 'ffmpeg', 'chrome', 'playwright'}):
                unresolved.append({'lane': lane, 'reason': 'decoded-video operating proof needs selected installed runtime, revision and explicit media tools'})
            else:
                selectors = [argument for name in EXTERNAL_MEDIA_TESTS.values()
                             for argument in ('--test', name)]
                selected += [('media-fixture-cold-build', [*cargo, *selectors, '--no-run'], 2400),
                    ('external-decoder-fixtures', [*cargo, *selectors,
                     '--', '--ignored', '--test-threads=1', '--nocapture'], 600)]
                selected += [('reference-client-browser', [str(media_tools['node']),
                    'scripts/check-reference-client.mjs'], 180)]
                selected += [('browser-rtx-codec-offer', [str(media_tools['node']),
                    'sdk/tests/browser_rtx_offer.mjs', str(runtime / 'sdk/typescript/dist'),
                    str(ROOT / '.local/gate5/browser-rtx-offer' / uuid.uuid4().hex / 'result.json')], 120)]
                media = [sys.executable, 'scripts/check-video-paths.py', '--runtime', str(runtime),
                    '--source-revision', source_revision, '--work-root', '.local/gate5/media-operating',
                    '--node', str(media_tools['node']), '--ffmpeg', str(media_tools['ffmpeg'])]
                selected += [('decoded-video-clean', [*media, '--output', '.local/gate5/media-clean.json'], 600),
                    ('decoded-video-impairment', [*media, '--impairment',
                     '--output', '.local/gate5/media-impairment.json'], 600),
                    ('decoded-video-mute-recovery', [*media, '--mute-recovery', '--seconds', '20',
                     '--output', '.local/gate5/media-mute-recovery.json'], 600),
                    ('decoded-video-exploratory-capacity', [*media, '--exploratory-concurrency',
                     '--output', '.local/gate5/media-exploratory-capacity.json'], 600)]
                unresolved.append({'lane': lane, 'reason': 'complete media failure acceptance remains required; clean and impaired decoded routes do not substitute for it'})
        else:
            raise ValueError('Unknown lane: ' + lane)
        steps += [{'lane': lane, 'name': lane + '--' + name, 'command': command,
                   'timeout_seconds': seconds} for name, command, seconds in selected]
    unresolved += [{'lane': lane, 'reason': 'required lane not selected'} for lane in LANES if lane not in lanes]
    return steps, unresolved
