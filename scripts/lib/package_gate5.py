"""Gate 5 installed package checks; unresolved coverage is a blocking result.

Compilation and pinned Python environment preparation are separate caller phases.
Behavior selects installed executables and SDK assets. Connector examples compile
from installed API sources; native storage fixtures compile from exact archived
source and separately launch the selected installed CLI. Unmeasured acceptance
requirements remain blocking even when a component lane passes.
"""
import json
import math
import pathlib
import re
import sys

from .package_inventory import digest, inventory_gate5
from .qualification_process import run_phase
from .media_failure_proof import verify_external_failures
from .qualification_report import environment, check_test_results
from .setup_provider_fixture import CHECKS as PROVIDER_SETUP_CHECKS

CONNECTED_APP_CHECKS = (
    'normal example excludes test-support crash controls',
    'outgoing artifact bytes, owner, conversation and draft digest',
    'concurrent worker cannot own original ledger',
    'unbound, wrong-conversation and unauthorized decisions cannot select a record',
    'both directions retain their original conversation mapping',
    'app checkpoint and original business effect identities persisted',
    'original mutation and business IDs survive server and worker restart',
    'same reviewer mutation replays exact accepted receipt',
    'late decision and semantic duplicates preserve original effects',
    'one public response for review_recorded', 'one public response for draft_received',
    'unauthorized contact receives no business response',
    'selected normal binary hashes unchanged', 'selected driver and tools unchanged',
)


def connected_app_inputs(release, source):
    paths = [source / name for name in ('scripts/check-connected-app.py',
        'scripts/lib/local_process.py', 'scripts/lib/qualification_report.py')]
    paths += [pathlib.Path(sys.executable), release / 'bin/connect', release / 'bin/connect-example']
    return {str(path.resolve(strict=True)): digest(path) for path in paths}


def verify_connected_app(observed, release, source, revision):
    expected = connected_app_inputs(release, source)
    binaries = {name: str((release / 'bin' / name).resolve(strict=True)) for name in ('connect', 'connect-example')}
    if (not isinstance(observed, dict) or observed.get('status') != 'passed'
            or observed.get('profile') != 'connected-app-public-cli/1'
            or observed.get('source_revision') != revision
            or observed.get('selected_binaries') != binaries
            or observed.get('binary_sha256') != {name: expected[path] for name, path in binaries.items()}
            or observed.get('driver_sha256') != digest(source / 'scripts/check-connected-app.py')
            or observed.get('input_sha256') != expected or observed.get('input_sha256_after') != expected
            or observed.get('tested_inputs_stable') is not True
            or observed.get('checks') != list(CONNECTED_APP_CHECKS)
            or observed.get('not_run') != ['test-support lost-ack crash injection',
                'actual-model inference', 'whole-gate qualification']
            or observed.get('remaining_owned_processes') != []
            or 'cleanup_error' in observed or observed.get('cleanup_errors')):
        raise ValueError('Installed connected-app proof omitted required behavior or changed selected inputs')
    cleanup = observed.get('cleanup')
    if (not isinstance(cleanup, list) or len(cleanup) != 3
            or any(not isinstance(row, dict) or type(row.get('pid')) is not int or row['pid'] <= 0
                   or type(row.get('exit_code')) is not int for row in cleanup)
            or len({row['pid'] for row in cleanup}) != 3
            or sum(row['exit_code'] == 0 for row in cleanup) != 2):
        raise ValueError('Installed connected-app proof omitted the three expected worker exits')
    return observed


def session_inputs(release, source, node):
    paths = [release / 'bin/connect', pathlib.Path(node), pathlib.Path(sys.executable),
        source / 'scripts/check-installed-sessions.py', source / 'scripts/lib/local_process.py',
        source / 'scripts/lib/qualification_report.py', source / 'sdk/tests/session_probe.py',
        source / 'scripts/lib/qualification_process.py', source / 'scripts/lib/phase_supervisor.py',
        source / 'sdk/tests/session_probe.mjs']
    for folder in ('sdk/python/connect_sdk', 'sdk/typescript/dist'):
        paths += [path for path in (release / folder).rglob('*') if path.is_file()
            and path.suffix not in ('.pyc', '.pyo') and '__pycache__' not in path.parts]
    return {str(path.resolve(strict=True)): digest(path) for path in paths}


def verify_session_controls(observed, release, source, node, revision):
    expected = session_inputs(release, source, node)
    if (not isinstance(observed, dict) or observed.get('status') != 'passed'
            or observed.get('profile') != 'installed-session-controls/1'
            or observed.get('source_revision') != revision
            or observed.get('selected_binary') != str((release / 'bin/connect').resolve(strict=True))
            or observed.get('selected_assets') != str(release.resolve(strict=True))
            or observed.get('input_sha256') != expected or observed.get('input_sha256_after') != expected
            or observed.get('tested_inputs_stable') is not True
            or observed.get('interfaces') != ['CLI', 'Python native helper', 'Python HTTP', 'TypeScript native helper', 'TypeScript HTTP']
            or any(type(observed.get(name)) is not int or observed[name] != count for name, count in
                [('cli_seed_records', 19), ('original_scope_records', 35), ('sdk_origin_records', 16), ('narrowed_scope_denials', 26)])
            or observed.get('current_authorization_enforced_on_replay') is not True
            or observed.get('credential_diagnostics_redacted') is not True):
        raise ValueError('Installed Session controls omitted selected inputs, SDK origins or current-authority replay')
    phases = observed.get('sdk_phases')
    expected_names = [f'session-{language}-{index}' for index, language in
        enumerate(('python', 'typescript', 'python', 'typescript', 'python', 'typescript'), 1)]
    if (not isinstance(phases, list) or len(phases) != 6
            or any(not isinstance(phase, dict) for phase in phases)
            or [phase.get('name') for phase in phases] != expected_names
            or any(phase.get('status') != 'passed' or phase.get('leftover_group_members') != []
                or phase.get('group_cleanup') != 'signalled' or 'error' in phase or 'cleanup_error' in phase for phase in phases)):
        raise ValueError('Installed Session controls lack six owned SDK phases with complete cleanup')
    return observed


def verify_reference_continuity(observed):
    ids = ('conversation_id', 'session_id', 'source_message_id', 'decision_message_id',
        'decision_actor_id', 'followup_message_id', 'artifact_id')
    if (not isinstance(observed, dict) or observed.get('status') != 'passed'
            or any(not isinstance(observed.get(name), str) or not observed[name] for name in ids)
            or len({observed[name] for name in ('source_message_id', 'decision_message_id', 'followup_message_id')}) != 3
            or not isinstance(observed.get('artifact_sha256'), str)
            or re.fullmatch('[0-9a-f]{64}', observed['artifact_sha256']) is None
            or any(type(observed.get(name)) is not int or observed[name] <= 0 for name in ('artifact_size', 'explicit_omitted_sources'))
            or type(observed.get('foreign_conversation_sources')) is not int or observed['foreign_conversation_sources'] != 0
            or any(observed.get(name) is not True for name in ('artifact_bytes_reverified_after_restart', 'exact_decision_replay', 'one_followup'))
            or observed.get('transcript_status') != 'No transcript or derived summary was produced; AV transport supplies no ASR or model vision.'):
        raise ValueError('Installed reference continuity lacks attributed decision, artifact or replay proof')


def native_cleanup_inputs(release, source, ffmpeg):
    """Reconstruct the checker's full selected input inventory independently."""
    script = source / 'scripts'
    paths = [script / 'check-native-video-cleanup.py', script / 'video/fixture_protocol.py',
        script / 'video/native-processes.py', script / 'video/decoder_observation.py',
        script / 'video/marker_calibration.py', script / 'lib/local_process.py',
        script / 'lib/qualification_process.py', script / 'lib/phase_supervisor.py',
        script / 'lib/native_process_owner.py',
        script / 'tests/test_native_video_cleanup.py', release / 'bin/connect',
        release / 'bin/connect-video', pathlib.Path(ffmpeg), pathlib.Path(sys.executable), pathlib.Path('/bin/ps')]
    for directory in [script / 'video'] + [source / name for name in ('examples/native-video', 'crates', 'contracts')]:
        paths.extend(path for path in directory.rglob('*') if path.is_file()
            and not any(part in ('target', '__pycache__', '.local', '.deps') for part in path.relative_to(directory).parts)
            and path.suffix not in ('.pyc', '.pyo'))
    for name in ('Cargo.toml', 'Cargo.lock', 'dependencies/source-lock.json', 'manifest.json'):
        path = source / name
        if path.is_file():
            paths.append(path)
    return {str(path.resolve(strict=True)): digest(path) for path in paths}


def verify_native_cleanup(observed, release, source, ffmpeg, revision, work_root):
    """Require both actual example faults and both same-Session reattachments."""
    def fail(reason):
        raise ValueError('Installed native example cleanup proof: ' + reason)
    def finite_time(value):
        return type(value) in (int, float) and math.isfinite(value) and 0 <= value <= 5
    def hash_value(value):
        return isinstance(value, str) and re.fullmatch('[0-9a-f]{64}', value) is not None
    def field(value, name):
        if not isinstance(value, dict):
            fail('missing structured observation')
        return value.get(name)

    release = release.resolve(strict=True)
    source = source.resolve(strict=True)
    ffmpeg = pathlib.Path(ffmpeg).resolve(strict=True)
    expected = native_cleanup_inputs(release, source, ffmpeg)
    if (not isinstance(observed, dict) or observed.get('status') != 'passed'
            or observed.get('checker_profile') != 'connect-native-video-cleanup/1'
            or observed.get('source_revision') != revision
            or observed.get('tested_inputs_stable') is not True
            or observed.get('not_run') != [] or 'active_case' in observed
            or observed.get('selected_runtime') != str(release)
            or observed.get('selected_source') != str(source)
            or observed.get('selected_server_binary') != str(release / 'bin/connect')
            or observed.get('selected_video_binary') != str(release / 'bin/connect-video')
            or observed.get('selected_ffmpeg') != str(ffmpeg)
            or observed.get('input_sha256') != expected
            or observed.get('input_sha256_after') != expected
            or observed.get('selected_binary_sha256') != {'server': digest(release / 'bin/connect'),
                'video': digest(release / 'bin/connect-video'), 'ffmpeg': digest(ffmpeg)}):
        fail('selected binaries/source/tools/revision or before/after inputs changed')
    cases = observed.get('cases')
    if (not isinstance(cases, list) or len(cases) != 2
            or {field(row, 'case') for row in cases} != {'interrupt', 'capture-refusal'}):
        fail('missing or duplicated fault cases')
    private = observed.get('private_attempt_directory')
    if not isinstance(private, str):
        fail('missing private attempt artifacts')
    attempt = pathlib.Path(private).resolve(strict=True)
    if not attempt.is_relative_to(pathlib.Path(work_root).resolve(strict=True)):
        fail('attempt artifacts do not belong to the selected qualification phase')
    for case in cases:
        name = case['case']
        if case.get('status') != 'passed':
            fail('fault case did not pass')
        context = case.get('continuity')
        conversation = field(context, 'conversation_id')
        session = field(context, 'session_id')
        initial = field(context, 'history_message_ids')
        if (not isinstance(conversation, str) or not conversation or not isinstance(session, str) or not session
                or not isinstance(initial, list) or len(initial) != 1 or not isinstance(initial[0], str) or not initial[0]):
            fail('missing original Conversation/Session/history identity')
        followup = case.get('continued_communication')
        ids = field(followup, 'history_message_ids')
        if (not isinstance(ids, list) or len(ids) != 2 or ids[0] != initial[0]
                or not isinstance(ids[1], str) or not ids[1] or ids[1] == ids[0]):
            fail('ordinary communication did not continue in the same Conversation')
        for key, messages in [('continuity', initial), ('continued_communication', ids), ('after_reattachment', ids)]:
            row = case.get(key)
            if (field(row, 'conversation_id') != conversation or field(row, 'session_id') != session
                    or field(row, 'session_state') != 'accepted' or field(row, 'history_preserved') is not True
                    or type(field(row, 'terminal_session_events')) is not int
                    or field(row, 'terminal_session_events') != 0 or field(row, 'history_message_ids') != messages):
                fail('accepted Session or exact Conversation history was not preserved')
        for key, interrupted in [('fault', 0), ('fresh_reattachment', 1)]:
            row = case.get(key)
            if (field(row, 'status') != 'passed' or field(row, 'conversation_id') != conversation
                    or field(row, 'session_id') != session
                    or field(row, 'fault') != (name if key == 'fault' else 'interrupt')
                    or field(row, 'recovery_signals') != []
                    or field(row.get('readiness'), 'complete_incoming_keyframes') != 2):
                fail('missing actual duplex keyframes/reattachment or forced group cleanup')
            cleanup = row.get('cleanup')
            if (field(cleanup, 'both_exited_within5seconds') is not True
                    or field(cleanup, 'remaining_owned_processes') != []
                    or field(cleanup, 'forced_cleanup_required') is not False):
                fail('owned native/FFmpeg descendants survived or cleanup was forced')
            peers = cleanup.get('participants')
            if not isinstance(peers, list) or len(peers) != 2 or {field(peer, 'slot') for peer in peers} != {0, 1}:
                fail('missing or duplicated participant exits')
            for peer in peers:
                slot = peer['slot']
                failed = name == 'capture-refusal' and key == 'fault' and slot == 0
                reason = 'child-failure' if failed else 'interrupt' if slot == interrupted else 'peer-terminal'
                if name == 'capture-refusal' and key == 'fault' and slot == 1:
                    reason = 'peer-terminal'
                code = peer.get('exit_code')
                if (type(slot) is not int or type(code) is not int or (code == 0) == failed
                        or peer.get('success') is not (not failed) or peer.get('end_reason') != reason
                        or not finite_time(peer.get('trigger_to_exit_seconds'))
                        or not finite_time(peer.get('teardown_seconds'))):
                    fail('participant exit/failure reason or five-second deadline was not observed')
                folder = attempt / name / key.replace('fresh_reattachment', 'fresh-reattachment') / ('reviews' if slot == 0 else 'agent')
                for label, filename in [('report_sha256', 'observation.json'), ('incoming_ivf_sha256', 'incoming.ivf')]:
                    artifact = folder / filename
                    if (not hash_value(peer.get(label)) or not artifact.is_file() or artifact.is_symlink()
                            or digest(artifact) != peer[label]):
                        fail('private participant observation/IVF artifact changed or is missing')
                    if label == 'report_sha256':
                        retained = json.loads(artifact.read_text())
                        if any(retained.get(field) != peer.get(field)
                                for field in ('end_reason', 'success', 'teardown_seconds')):
                            fail('participant summary disagrees with its retained native report')
        refusal = case['fault']
        if name == 'capture-refusal':
            shim = attempt / name / 'fault/ffmpeg-refuse-audio.py'
            before = refusal.get('shim_sha256_before')
            if (not hash_value(before) or refusal.get('shim_sha256_after') != before
                    or not shim.is_file() or shim.is_symlink() or digest(shim) != before):
                fail('capture refusal shim changed or is missing')
            events = refusal.get('shim_events')
            if not isinstance(events, list) or any(not isinstance(event, dict) for event in events):
                fail('capture refusal events are missing')
            blocked = [event for event in events if event.get('kind') == 'blocked-avfoundation']
            refused = [event for event in events if event.get('kind') == 'capture-refusal']
            forwards = [event for event in events if event.get('kind') == 'forward-generated-codec']
            if (len(blocked) != 1 or len(refused) != 1 or not forwards
                    or any(event.get('hardware_invoked') is not False for event in events)
                    or blocked[0].get('audio_only') is not True or refused[0].get('audio_only') is not True
                    or any(event.get('kind') not in ('blocked-avfoundation', 'capture-refusal', 'forward-generated-codec') for event in events)):
                fail('audio avfoundation was not refused before hardware with real generated codec forwards')
    return observed


def verify_decoded_routes(observed, expected_status, seconds):
    """Require every route's completed scenario, not only an aggregate label."""
    def duration(value):
        return type(value) in (int, float) and math.isfinite(value) and value >= seconds

    routes = observed.get('paths')
    if (not duration(observed.get('qualification_seconds'))
            or not isinstance(routes, list) or len(routes) != 3
            or any(not isinstance(route, dict) for route in routes)
            or {route.get('path') for route in routes}
            != {'native-native', 'native-browser', 'browser-browser'}
            or any(route.get('status') != expected_status
                   or not duration(route.get('qualification_seconds')) for route in routes)):
        raise ValueError('Installed decoded paths did not complete every candidate route/input scenario')


def verify_provider_setup(observed):
    """Installed setup must include every supplied-metadata/loopback case."""
    if (not isinstance(observed, dict) or observed.get('status') != 'passed'
            or observed.get('checks') != [{'name': name, 'status': 'passed'} for name in PROVIDER_SETUP_CHECKS]
            or observed.get('provider_http_methods') != ['GET']
            or type(observed.get('provider_http_count')) is not int
            or observed['provider_http_count'] != 2
            or any(type(observed.get(name)) is not int or observed[name] != 0
                   for name in ('resource_mutations', 'real_account_requests'))
            or observed.get('preview_and_readiness_inputs_unchanged') is not True):
        raise ValueError('Installed provider setup proof omitted a required read-only loopback/metadata case')


def qualify(release, source, manifest, directory, independent_python=None, media_tools=None,
            baselines=None):
    inventory_gate5(release, source)
    directory.mkdir(mode=0o700, parents=True, exist_ok=False)
    env = environment()
    (directory / 'provider-state').mkdir(mode=0o700)
    env.update(PRISM_TEST_STATE_DIR=str(directory / 'provider-state'),
               PYTHONPATH=str(release / 'sdk/python'),
               CONNECT_TEST_RUNTIME_ROOT=str(release),
               CONNECT_EXAMPLE_BINARY=str(release / 'bin/connect-example'),
               CONNECT_BINARY=str(release / 'bin/connect'),
               CONNECT_TEST_BINARY=str(release / 'bin/connect'),
               CONNECT_VIDEO_BINARY=str(release / 'bin/connect-video'),
               CONNECT_SOURCE_REVISION=manifest['source_revision'],
               CARGO_TARGET_DIR=str(directory / 'target'))
    report = {'status': 'running', 'source_revision': manifest['source_revision'],
              'installed_binary_sha256': digest(release / 'bin/connect'),
              'installed_video_sha256': digest(release / 'bin/connect-video'),
              'steps': [], 'whole_gate_qualified': False, 'unresolved': []}
    def save():
        (directory / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    def phase(name, command, timeout, cwd):
        result = run_phase(name, command, timeout, directory, cwd, env)
        report['steps'].append(result)
        save()
        if result['status'] != 'passed':
            raise RuntimeError('Installed Gate 5 phase failed: ' + name)
        return result
    save()
    try:
        for name in ('connector-template', 'connector-contribution'):
            arguments = ['--offline', '--locked', '--manifest-path', str(release / 'examples' / name / 'Cargo.toml')]
            phase(name + '-cold-tests', ['cargo', 'test', '--no-run'] + arguments, 2400, release)
            phase(name + '-cold-binary', ['cargo', 'build', '--bins'] + arguments, 2400, release)
            result = phase(name + '-tests', ['cargo', 'test'] + arguments, 120, release)
            check_test_results(result, directory)
            if result.get('rust_tests') != {'passed': 4, 'failed': 0, 'ignored': 0}:
                raise ValueError('Installed connector example did not execute its four required checks')
            phase(name + '-contract', [sys.executable, str(source / 'scripts/check-connector-contract.py'),
                  '--manifest-path', str(release / 'examples' / name / 'Cargo.toml'),
                  '--output', str(directory / (name + '-contract.json')),
                  '--public-root', str(release), '--target-dir', str(directory / 'target'),
                  '--work-root', str(directory / (name + '-state'))], 120, release)
        if independent_python is None:
            report['unresolved'].append({'check': 'installed-independent-control',
                'reason': 'Select --independent-python from the pinned requirements environment; no ambient or implicit dependency installation'})
        else:
            vectors = phase('independent-client-vectors', [str(independent_python), '-m', 'unittest',
                  'discover', '-s', str(release / 'examples/independent-uctp-client'), '-p', 'test_client.py'], 120, release)
            import re
            text = (directory / vectors['log']).read_text()
            if re.findall(r'^Ran (\d+) tests? in ', text, re.M) != ['6'] or '\nOK\n' not in text or 'skipped=' in text:
                raise ValueError('Independent client vectors did not execute all six checks')
            vectors['python_tests'] = {'passed': 6, 'failed': 0, 'ignored': 0}
            output = directory / 'independent-control.json'
            phase('installed-independent-control', [str(independent_python),
                  str(source / 'scripts/check-independent-uctp.py'), '--binary', str(release / 'bin/connect'),
                  '--client-root', str(release / 'examples/independent-uctp-client'),
                  '--source-revision', manifest['source_revision'], '--work-root', str(directory / 'control-state'),
                  '--output', str(output)], 600, release)
            observed = json.loads(output.read_text())
            if (observed.get('status') != 'passed-control-only' or not observed.get('tested_inputs_stable')
                    or observed.get('server_sha256') != report['installed_binary_sha256']
                    or observed.get('client_sha256') != digest(release / 'examples/independent-uctp-client/client.py')
                    or observed.get('source_revision') != manifest['source_revision']):
                raise ValueError('Independent installed proof did not bind the candidate inputs')
            report['independent_control'] = observed
        output = directory / 'public-setup.json'
        phase('installed-public-setup', [sys.executable, str(source / 'scripts/check-gate5-setup.py'),
            '--binary', str(release / 'bin/connect'), '--source-revision', manifest['source_revision'],
            '--work-root', str(directory / 'setup-state'), '--output', str(output)], 180, release)
        observed = json.loads(output.read_text())
        if (observed.get('status') != 'passed' or not observed.get('tested_inputs_stable')
                or observed.get('binary_sha256') != report['installed_binary_sha256']
                or observed.get('candidate_schema') != manifest['schema']
                or observed.get('source_revision') != manifest['source_revision']):
            raise ValueError('Installed first-run proof did not bind candidate inputs')
        verify_provider_setup(observed.get('provider_setup'))
        report['public_setup'] = observed
        output = directory / 'connected-app.json'
        phase('installed-connected-app', [sys.executable, str(source / 'scripts/check-connected-app.py'),
            '--server-binary', str(release / 'bin/connect'),
            '--example-binary', str(release / 'bin/connect-example'),
            '--source-revision', manifest['source_revision'],
            '--work-root', str(directory / 'connected-app-state'), '--output', str(output)], 180, release)
        report['connected_app'] = verify_connected_app(json.loads(output.read_text()),
            release, source, manifest['source_revision'])
        if not media_tools or not media_tools.get('node'):
            report['unresolved'].append({'check': 'installed-session-controls',
                'reason': 'Explicit selected Node required for packaged TypeScript native/HTTP Session probes'})
        else:
            output = directory / 'session-controls.json'
            phase('installed-session-controls', [sys.executable, str(source / 'scripts/check-installed-sessions.py'),
                '--binary', str(release / 'bin/connect'), '--assets', str(release),
                '--node', str(media_tools['node']), '--source-revision', manifest['source_revision'],
                '--work-root', str(directory / 'session-control-state'), '--output', str(output)], 180, release)
            report['session_controls'] = verify_session_controls(json.loads(output.read_text()),
                release, source, media_tools['node'], manifest['source_revision'])
        # Native Keychain library behavior and installed CLI operations have
        # separate scope. A source-built test executable never becomes an
        # installed server substitute via CARGO_BIN_EXE_connect.
        cargo = ['cargo', 'test', '--offline', '--locked', '--manifest-path', str(source / 'Cargo.toml'),
                 '-p', 'connect-server', '--all-features', '--test', 'secrets_native']
        phase('native-storage-fixture-cold-build', [*cargo, '--no-run'], 2400, source)
        result = phase('native-storage-installed-cli', [*cargo, 'isolated_native_keychain',
            '--', '--ignored', '--nocapture'], 120, source)
        check_test_results(result, directory)
        if result.get('rust_tests') != {'passed': 2, 'failed': 0, 'ignored': 0}:
            raise ValueError('Native parent and child storage fixtures did not both execute')
        failure_tests = ['cargo', 'test', '--offline', '--locked', '--manifest-path',
            str(source / 'Cargo.toml'), '-p', 'connect-server', '--all-features',
            '--test', 'gate5_video_failures']
        phase('source-media-failure-fixture-build', [*failure_tests, '--no-run'], 2400, source)
        result = phase('source-media-failure-fixture', failure_tests, 120, source)
        check_test_results(result, directory)
        if result.get('rust_tests') != {'passed': 7, 'failed': 0, 'ignored': 0}:
            raise ValueError('All seven exact-source media failure checks must execute')
        report['source_media_failures'] = {
            'scope': 'exact archived source SDK and embedded server; separate from installed runtime checks',
            'passed': 7, 'failed': 0, 'ignored': 0,
            'source_revision': manifest['source_revision'],
        }
        external_failures = ['cargo', 'test', '--offline', '--locked', '--manifest-path',
            str(source / 'Cargo.toml'), '-p', 'connect-server', '--all-features',
            '--test', 'gate5_installed_video_failures']
        phase('external-media-failure-fixture-build', [*external_failures, '--no-run'], 2400, source)
        result = phase('installed-media-failures', [*external_failures, '--', '--ignored',
            '--nocapture', '--test-threads=1'], 120, source)
        check_test_results(result, directory)
        report['installed_media_failures'] = verify_external_failures(result, directory,
            release / 'bin/connect', source, manifest['source_revision'])
        phase('native-example-cold-build', ['cargo', 'build', '--offline', '--locked',
            '--manifest-path', str(source / 'Cargo.toml'), '-p', 'connect-native-video',
            '--bin', 'connect-video'], 2400, source)
        report['native_example_source_build'] = {
            'scope': 'matching archived workspace source outside the author checkout; supplied runtime example manifest inherits that workspace',
            'source_revision': manifest['source_revision'], 'status': 'passed'}
        if not media_tools or not media_tools.get('ffmpeg'):
            report['unresolved'].append({'check': 'installed-native-example-cleanup',
                'reason': 'Explicit selected FFmpeg required for actual native example interrupt/refusal and both same-Session reattachments'})
        else:
            output = directory / 'native-example-cleanup.json'
            work_root = directory / 'native-example-cleanup-state'
            phase('installed-native-example-cleanup', [sys.executable,
                str(source / 'scripts/check-native-video-cleanup.py'), '--runtime', str(release),
                '--source-path', str(source), '--source-revision', manifest['source_revision'],
                '--ffmpeg', str(media_tools['ffmpeg']), '--work-root', str(work_root),
                '--output', str(output)], 300, release)
            report['native_example_cleanup'] = verify_native_cleanup(json.loads(output.read_text()),
                release, source, media_tools['ffmpeg'], manifest['source_revision'], work_root)
        if baselines is None:
            report['unresolved'].append({'check': 'installed-historical-recovery',
                'reason': 'Explicit verified schema1 and schema16 baseline executables/manifests required'})
        else:
            output = directory / 'historical-recovery.json'
            command = [sys.executable, str(source / 'scripts/check-gate5-recovery.py'),
                '--binary', str(release / 'bin/connect'), '--catalog', str(source / 'dependencies/recovery-baselines.json'),
                '--python-sdk', str(release / 'sdk/python'), '--work-root', str(directory / 'recovery-state'),
                '--output', str(output)]
            for flag, path in baselines.items():
                command += ['--' + flag, str(path)]
            phase('installed-historical-recovery', command, 660, release)
            observed = json.loads(output.read_text())
            if (observed.get('status') != 'passed'
                    or observed.get('candidate_schema') != manifest['schema']
                    or observed.get('binary_sha256') != report['installed_binary_sha256']):
                raise ValueError('Installed recovery proof did not bind candidate executable')
            report['historical_recovery'] = observed
        if not media_tools or set(media_tools) != {'node', 'ffmpeg', 'chrome', 'playwright'}:
            report['unresolved'].append({'check': 'installed-decoded-video',
                'reason': 'Explicit Node, FFmpeg, Chrome and Playwright inputs required; no ambient browser fallback'})
        else:
            env.update(CONNECT_CHROME_BINARY=str(media_tools['chrome']),
                CONNECT_PLAYWRIGHT_MODULE=str(media_tools['playwright']))
            reference_output = directory / 'reference-client.json'
            env['CONNECT_REFERENCE_OUTPUT'] = str(reference_output)
            phase('installed-reference-client', [str(media_tools['node']),
                str(source / 'scripts/check-reference-client.mjs')], 180, release)
            reference = json.loads(reference_output.read_text())
            if (reference.get('result') != 'passed' or not reference.get('tested_inputs_stable')
                    or reference.get('source_revision') != manifest['source_revision']
                    or reference.get('runtime_root') != str(release)
                    or reference.get('binary_sha256') != report['installed_binary_sha256']):
                raise ValueError('Installed reference proof did not bind candidate inputs')
            video = reference.get('video', {})
            failures = video.get('failures', [])
            if (video.get('status') != 'passed' or len(failures) != 2
                    or {row.get('failure') for row in failures} != {'capture-ended', 'ice-failed'}
                    or any(type(row.get('cleanupMs')) not in (int, float)
                           or not math.isfinite(row['cleanupMs']) or not 0 <= row['cleanupMs'] < 5000
                           for row in failures)):
                raise ValueError('Installed browser failure cleanup proof is incomplete')
            verify_reference_continuity(video.get('continuity'))
            report['reference_client'] = reference
            for label, flag, seconds, expected_status in (
                ('decoded-video', None, 60, 'passed-clean'),
                ('decoded-video-impaired', '--impairment', 60, 'passed-impairment'),
                ('decoded-video-mute-recovery', '--mute-recovery', 20, 'passed-mute-recovery'),
            ):
                output = directory / (label + '.json')
                command = [sys.executable, str(source / 'scripts/check-video-paths.py'),
                    '--runtime', str(release), '--source-revision', manifest['source_revision'],
                    '--work-root', str(directory / (label + '-state')), '--output', str(output),
                    '--node', str(media_tools['node']), '--ffmpeg', str(media_tools['ffmpeg']),
                    '--seconds', str(seconds)]
                if flag:
                    command.append(flag)
                phase('installed-' + label, command, 600, release)
                observed = json.loads(output.read_text())
                verify_decoded_routes(observed, expected_status, seconds)
                bound = observed.get('input_sha256', {})
                if (observed.get('status') != expected_status or not observed.get('tested_inputs_stable')
                        or observed.get('source_revision') != manifest['source_revision']
                        or observed.get('selected_runtime') != str(release)
                        or observed.get('selected_assets') != str(release)
                        or bound.get(str(release / 'bin/connect')) != report['installed_binary_sha256']
                        or bound.get(str(release / 'bin/connect-video')) != report['installed_video_sha256']):
                    raise ValueError('Installed decoded paths did not bind every candidate route/input')
                known_checks = {
                    'credential revocation, restart, cancellation and ICE/QUIC failure cases outside this checker',
                    'camera/microphone mute recovery is a separate --mute-recovery run',
                }
                if any(check not in known_checks for check in observed.get('separate_required_checks', [])):
                    raise ValueError('Installed media reports an unaccounted acceptance obligation')
                report[label.replace('-', '_')] = observed
                report['unresolved'] += [{'check': 'installed-video-acceptance', 'reason': requirement}
                    for requirement in observed.get('unresolved', [])]
        report['joined_media_proof'] = {
            'native_authority_lifecycle': 'installed_media_failures',
            'native_example_interrupt_capture_child_and_reattachment': 'native_example_cleanup',
            'browser_capture_and_ice_cleanup': 'reference_client.video.failures',
            'decoded_route_delivery': 'decoded_video',
            'decoded_route_loss_recovery': 'decoded_video_impaired',
            'camera_microphone_mute_recovery': 'decoded_video_mute_recovery',
            'scope': 'separate component obligations stay identifiable; no single decoded run substitutes for failures',
        }
        report['status'] = 'incomplete' if report['unresolved'] else 'passed'
    except BaseException as error:
        report.update(status='failed', error_class=type(error).__name__)
        raise
    finally:
        save()
    return report
