#!/usr/bin/env python3
"""Public native example interruption/capture-refusal and same-Session reattachment.

No build, private Store/Core, device access, or profile throughput qualification.
Only explicitly selected executables and owned phase process groups are used.
Every attempt, including failure, retains its private logs and input identities.
"""
import argparse
import datetime
import hashlib
import importlib.util
import json
import os
import pathlib
import shutil
import signal
import subprocess
import sys
import time
import traceback

from lib.local_process import Server
from lib.native_process_owner import OwnedChild

SCRIPT = pathlib.Path(__file__).resolve()
FIXTURE = SCRIPT.parent / 'video/fixture_protocol.py'
NATIVE = SCRIPT.parent / 'video/native-processes.py'


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


fixture = load('native_cleanup_fixture', FIXTURE)
media = load('native_cleanup_runtime', NATIVE)


def digest(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def owned_rows(peers):
    """Observe owned groups, including reparented children; never signal ps PIDs."""
    groups = {peer.process.pid for peer in peers}
    output = subprocess.check_output(['/bin/ps', '-axo', 'pid=,ppid=,pgid=,stat=,comm='],
        text=True, timeout=2)
    rows = []
    for line in output.splitlines():
        fields = line.strip().split(None, 4)
        if len(fields) != 5:
            continue
        pid, parent, group = map(int, fields[:3])
        if group in groups and pid != group and not fields[3].startswith('Z'):
            rows.append({'pid': pid, 'parent_pid': parent, 'owned_group': group,
                'executable': fields[4]})
    return rows


class Peer:
    def __init__(self, args, state, conversation, session, identities, name, work,
                 ffmpeg=None, audio=None):
        self.work = work
        work.mkdir(mode=0o700)
        self.video = work / 'incoming.ivf'
        self.report = work / 'observation.json'
        self.log = (work / 'process.log').open('w')
        other = 'agent' if name == 'reviews' else 'reviews'
        command = [str(args.video_binary), '--config', str(state / (name + '.toml')),
            '--conversation', conversation, '--session', session, '--peer', identities[other],
            '--ffmpeg', str(ffmpeg or args.ffmpeg), '--seconds', '120', '--input', 'test-pulse',
            '--audio-input', audio or ('pulse:440' if name == 'reviews' else 'pulse:660'),
            '--fixture-marker', '0' if name == 'reviews' else '1', '--native-peer',
            '--output', str(self.video), '--audio-output', str(work / 'incoming.pcmu'),
            '--report', str(self.report)]
        # The codec children inherit this explicitly owned phase group.
        self.process = OwnedChild(command, stdout=self.log, stderr=self.log)
        self.exit_at = None

    def close_log(self):
        self.log.close()


def cleanup_owned(peers):
    """Cleanup is recovery only; any forced cleanup makes the case a failure."""
    recovery = []
    try:
        remaining = owned_rows(peers)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        # Observation failure cannot abandon live owned work. All leader
        # anchors remain unreaped, so bounded group cleanup is still safe.
        remaining = None
        recovery.append({'observation_error_class': type(error).__name__})
    for peer in peers:
        if remaining is None or any(row['owned_group'] == peer.process.pid for row in remaining):
            # Group identity originates solely in our own unreaped Child. Never
            # signal a PID discovered through ps, or an unanchored/reaped group.
            try:
                peer.process.kill_group()
                recovery.append({'owned_group': peer.process.pid, 'signal': 'SIGKILL'})
            except ProcessLookupError:
                pass
    # All possible group signaling finishes before releasing ANY leader anchor.
    for peer in peers:
        peer.process.wait(timeout=5)
        peer.close_log()
    return recovery


def ready(peers, work):
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        rows = owned_rows(peers)
        if all(media.decoder.live_keyframe_present(peer.video) for peer in peers):
            observed = {'observed_unix_seconds': time.time(), 'complete_incoming_keyframes': 2,
                'method': 'complete actual VP8 keyframes in both retained incoming IVF files; no decoded performance claim',
                'owned_processes': rows}
            write_json(work / 'readiness.json', observed)
            return observed
        if any(peer.process.poll() is not None for peer in peers):
            raise RuntimeError('Native participant exited before actual duplex keyframe readiness')
        time.sleep(.05)
    raise TimeoutError('Actual native incoming keyframes did not arrive within 30 seconds')


def exits(peers, began, work, expected_failure=False, interrupted_slot=0):
    deadline = began + 5
    observations = []
    while time.monotonic() < deadline:
        for peer in peers:
            if peer.exit_at is None and peer.process.poll() is not None:
                peer.exit_at = time.monotonic()
        if all(peer.exit_at is not None for peer in peers):
            break
        time.sleep(.01)
    # Save partial exit state before any recovery can change the evidence.
    progress = [{'slot': index, 'exit_code': peer.process.poll(),
        'native_pid': getattr(peer.process, 'native_pid', None),
        'owned_group': getattr(peer.process, 'pid', None),
        'trigger_to_exit_seconds': None if peer.exit_at is None else peer.exit_at - began}
        for index, peer in enumerate(peers)]
    remaining = owned_rows(peers)
    write_json(work / 'exit-state.json', {'participants': progress,
        'remaining_owned_processes': remaining})
    if any(peer.exit_at is None or peer.exit_at - began > 5 for peer in peers):
        raise TimeoutError('Both native participants must exit within 5 seconds of trigger')
    if remaining:
        raise RuntimeError('Owned native/FFmpeg process group has surviving descendants')
    for index, peer in enumerate(peers):
        observed = json.loads(peer.report.read_text())
        row = {**progress[index], 'end_reason': observed['end_reason'],
            'success': observed['success'], 'teardown_seconds': observed['teardown_seconds'],
            'report_sha256': digest(peer.report), 'incoming_ivf_sha256': digest(peer.video)}
        observations.append(row)
        if type(row['exit_code']) is not int:
            raise RuntimeError('Actual native exit status is unavailable')
        if not 0 <= row['teardown_seconds'] <= 5:
            raise RuntimeError('Native reported teardown exceeded 5 seconds')
        if expected_failure and index == 0:
            if row['exit_code'] == 0 or row['success'] or row['end_reason'] != 'child-failure':
                raise RuntimeError('Capture-refusing participant did not report actual child failure')
        else:
            expected = 'peer-terminal' if expected_failure or index != interrupted_slot else 'interrupt'
            if row['exit_code'] != 0 or not row['success'] or row['end_reason'] != expected:
                raise RuntimeError('Native participant did not finish with expected terminal reason')
    return {'participants': observations, 'remaining_owned_processes': remaining,
        'both_exited_within5seconds': True, 'forced_cleanup_required': False}


def refusal_shim(args, work):
    """Intercept explicit audio hardware syntax before it can reach real FFmpeg."""
    events = work / 'shim-events'
    events.mkdir(mode=0o700)
    gate = work / 'release-refusal'
    shim = work / 'ffmpeg-refuse-audio.py'
    body = '''import json, os, pathlib, sys, time
real = REAL
work = pathlib.Path(WORK)
args = sys.argv[1:]
def event(kind, **extra):
    value = dict(kind=kind, pid=os.getpid(), group=os.getpgrp(), at_unix_seconds=time.time(), **extra)
    path = work / 'shim-events' / (str(os.getpid()) + '-' + kind + '.json')
    path.write_text(json.dumps(value) + '\\n')
if 'avfoundation' in args:
    # No hardware input is forwarded, regardless of any unexpected syntax.
    audio = '-vn' in args and '-c:a' in args and 'pcm_mulaw' in args
    event('blocked-avfoundation', audio_only=audio, hardware_invoked=False)
    if not audio: sys.exit(74)
    deadline = time.monotonic() + 30
    while not (work / 'release-refusal').exists():
        if time.monotonic() >= deadline: sys.exit(75)
        time.sleep(.01)
    event('capture-refusal', audio_only=True, hardware_invoked=False)
    sys.exit(73)
if 'lavfi' not in args:
    event('unexpected-input-refused', hardware_invoked=False)
    sys.exit(76)
event('forward-generated-codec', hardware_invoked=False)
os.execv(real, [real, *args])
'''.replace('REAL', repr(str(args.ffmpeg))).replace('WORK', repr(str(work)))
    shim.write_text('#!' + str(pathlib.Path(sys.executable).resolve()) + '\n' + body)
    shim.chmod(0o700)
    return shim, gate, events


def pair(args, state, identities, conversation, session, work, mode='interrupt', interrupt_slot=0):
    work.mkdir(mode=0o700)
    peers = []
    result = {'status': 'running', 'fault': mode,
        'conversation_id': conversation, 'session_id': session}
    shim = None
    try:
        gate = events = None
        if mode == 'capture-refusal':
            shim, gate, events = refusal_shim(args, work)
            result['shim_sha256_before'] = digest(shim)
        for name in ('reviews', 'agent'):
            peers.append(Peer(args, state, conversation, session, identities, name, work / name,
                ffmpeg=shim if name == 'reviews' and shim else None,
                audio='microphone:0' if name == 'reviews' and shim else None))
        result['readiness'] = ready(peers, work)
        began = time.monotonic()
        result['trigger_unix_seconds'] = time.time()
        if gate:
            # Refusal occurs after actual attachment, never actual hardware access.
            gate.touch(mode=0o600)
        else:
            peers[interrupt_slot].process.send_signal(signal.SIGINT)
        result['cleanup'] = exits(peers, began, work,
            expected_failure=mode == 'capture-refusal', interrupted_slot=interrupt_slot)
        if shim:
            evidence = [json.loads(path.read_text()) for path in sorted(events.glob('*.json'))]
            result['shim_events'] = evidence
            blocked = [event for event in evidence if event['kind'] == 'blocked-avfoundation']
            refused = [event for event in evidence if event['kind'] == 'capture-refusal']
            forwards = [event for event in evidence if event['kind'] == 'forward-generated-codec']
            if (len(blocked) != 1 or len(refused) != 1 or not forwards
                    or any(event['hardware_invoked'] for event in evidence)
                    or not blocked[0]['audio_only']):
                raise RuntimeError('Explicit audio capture refusal boundary was not established')
            result['shim_sha256_after'] = digest(shim)
            if result['shim_sha256_after'] != result['shim_sha256_before']:
                raise RuntimeError('Capture shim changed during qualification')
        result['status'] = 'passed'
    except BaseException as error:
        result.update(status='failed', error_class=type(error).__name__,
            error_frames=[{'file': pathlib.Path(frame.filename).name, 'line': frame.lineno,
                'function': frame.name} for frame in traceback.extract_tb(error.__traceback__)])
        raise
    finally:
        result['recovery_signals'] = cleanup_owned(peers)
        if result['recovery_signals']:
            result['status'] = 'failed'
        write_json(work / 'result.json', result)
    if result['status'] != 'passed':
        raise RuntimeError('Owned process group required forced recovery cleanup')
    return result


def continuity(state, conversation, session, args, message_ids):
    deadline = time.monotonic() + 5
    while True:
        durable = fixture.native_request(state, 'reviews', {'op': 'session_get',
            'conversation_id': conversation, 'session_id': session}, args.server_binary)
        if durable['state'] == 'accepted':
            break
        if durable['state'] not in ('accepted', 'ready') or time.monotonic() >= deadline:
            raise RuntimeError('Transient peer cleanup failed to preserve accepted durable Session')
        time.sleep(.05)
    history = fixture.native_request(state, 'agent', {'op': 'history',
        'conversation_id': conversation, 'limit': 100}, args.server_binary)
    if durable['conversation_id'] != conversation or [m['id'] for m in history['messages']] != message_ids:
        raise RuntimeError('Same-Conversation history changed during transient media cleanup')
    inbox = fixture.native_request(state, 'reviews', {'op': 'inbox', 'limit': 100, 'wait_ms': 0}, args.server_binary)
    terminal = [event for event in inbox['events'] if (event.get('session') or {}).get('id') == session
        and event['kind'] in ('session_ended', 'session_cancelled', 'session_rejected')]
    if terminal:
        raise RuntimeError('Transient media cleanup incorrectly ended durable Session')
    return {'conversation_id': conversation, 'session_id': session, 'session_state': durable['state'],
        'history_message_ids': message_ids, 'history_preserved': True, 'terminal_session_events': 0}


def scenario(args, work, mode):
    work.mkdir(mode=0o700)
    state, identities, _, conversation, session, _ = fixture.prepare(work, args.server_binary)
    result = {'status': 'running', 'case': mode}
    with Server(state, binary=args.server_binary, log_path=work / 'server.log'):
        first = fixture.native_request(state, 'reviews', {'op': 'send', 'request_id': 'before-cleanup',
            'conversation_id': conversation, "message": {'to': 'all', 'content_type': 'text/plain', 'body': 'History before native cleanup', 'body_encoding': 'utf8', 'label': 'connect-messages', 'attachments': [], 'in_reply_to_msg': None, 'msg_id': None}}, args.server_binary)
        result['fault'] = pair(args, state, identities, conversation, session, work / 'fault', mode)
        result['continuity'] = continuity(state, conversation, session, args, [first['message_id']])
        followup = fixture.native_request(state, 'agent', {'op': 'send', 'request_id': 'after-cleanup',
            'conversation_id': conversation, "message": {'to': 'all', 'content_type': 'text/plain', 'body': 'Conversation continues after native cleanup', 'body_encoding': 'utf8', 'label': 'connect-messages', 'attachments': [], 'in_reply_to_msg': None, 'msg_id': None}}, args.server_binary)
        ids = [first['message_id'], followup['message_id']]
        result['continued_communication'] = continuity(state, conversation, session, args, ids)
        result['fresh_reattachment'] = pair(args, state, identities, conversation, session,
            work / 'fresh-reattachment', interrupt_slot=1)
        result['after_reattachment'] = continuity(state, conversation, session, args, ids)
    result['status'] = 'passed'
    write_json(work / 'result.json', result)
    return result


def inputs(args):
    selected = [SCRIPT, FIXTURE, NATIVE, pathlib.Path(sys.modules[Server.__module__].__file__),
        pathlib.Path(media.decoder.__file__), pathlib.Path(media.decoder.markers.__file__),
        args.server_binary, args.video_binary, args.ffmpeg, pathlib.Path(sys.executable).resolve(), pathlib.Path('/bin/ps'),
        SCRIPT.parent / 'tests/test_native_video_cleanup.py',
        SCRIPT.parent / 'lib/qualification_process.py', SCRIPT.parent / 'lib/phase_supervisor.py',
        SCRIPT.parent / 'lib/native_process_owner.py']
    # Bind the complete qualification helper family, including identity and
    # coded/CDF marker observers even though this checker only consumes the
    # bounded live-keyframe readiness observer. Binding is not execution proof.
    selected.extend(path for path in (SCRIPT.parent / 'video').rglob('*') if path.is_file()
        and '__pycache__' not in path.parts and path.suffix not in ('.pyc', '.pyo'))
    example = args.source_path / 'examples/native-video'
    if not example.is_dir():
        raise ValueError('Select source/assets containing the public examples/native-video')
    for folder in ('examples/native-video', 'crates', 'contracts'):
        directory = args.source_path / folder
        selected.extend(path for path in directory.rglob('*') if path.is_file()
            and not any(part in ('target', '__pycache__', '.local', '.deps') for part in path.relative_to(directory).parts)
            and path.suffix not in ('.pyc', '.pyo'))
    for name in ('Cargo.toml', 'Cargo.lock', 'dependencies/source-lock.json', 'manifest.json'):
        path = args.source_path / name
        if path.is_file():
            selected.append(path)
    return {str(path.resolve()): digest(path) for path in sorted(set(selected))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runtime', required=True, type=pathlib.Path)
    parser.add_argument('--source-path', type=pathlib.Path, help='Explicit selected source/assets; defaults to runtime')
    parser.add_argument('--server-binary', type=pathlib.Path)
    parser.add_argument('--video-binary', type=pathlib.Path)
    parser.add_argument('--ffmpeg', default=os.environ.get('CONNECT_FFMPEG_BINARY', 'ffmpeg'))
    parser.add_argument('--source-revision', required=True)
    parser.add_argument('--work-root', required=True, type=pathlib.Path)
    parser.add_argument('--output', type=pathlib.Path)
    args = parser.parse_args()
    args.runtime = args.runtime.resolve(strict=True)
    args.source_path = (args.source_path or args.runtime).resolve(strict=True)
    args.server_binary = (args.server_binary or args.runtime / 'bin/connect').resolve(strict=True)
    args.video_binary = (args.video_binary or args.runtime / 'bin/connect-video').resolve(strict=True)
    args.ffmpeg = pathlib.Path(shutil.which(args.ffmpeg) or args.ffmpeg).resolve(strict=True)
    args.work_root = args.work_root.resolve()
    if '.local' not in args.work_root.parts or (args.output and '.local' not in args.output.resolve().parts):
        parser.error('Private attempts and output must be under explicitly selected ignored .local/')
    os.umask(0o077)
    args.work_root.mkdir(parents=True, exist_ok=True)
    work = args.work_root / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    work.mkdir(mode=0o700)
    report = {'status': 'running', 'checker_profile': 'connect-native-video-cleanup/1',
        'source_revision': args.source_revision, 'cases': [],
        'scope': 'local public CLI/public native SDK example; selected generated video/audio; no physical hardware or video profile rate/bitrate claims',
        'whole_gate_qualified': False, 'private_attempt_directory': str(work),
        'selected_runtime': str(args.runtime), 'selected_source': str(args.source_path),
        'selected_server_binary': str(args.server_binary), 'selected_video_binary': str(args.video_binary),
        'selected_ffmpeg': str(args.ffmpeg),
        'not_run': ['interrupt', 'capture-refusal']}
    bound = None
    try:
        bound = inputs(args)
        report['input_sha256'] = bound
        report['selected_binary_sha256'] = {'server': bound[str(args.server_binary)],
            'video': bound[str(args.video_binary)], 'ffmpeg': bound[str(args.ffmpeg)]}
        report['tool_versions'] = {'python': sys.version, 'ffmpeg': subprocess.check_output(
            [str(args.ffmpeg), '-version'], text=True, timeout=5).splitlines()[0]}
        write_json(work / 'result.json', report)
        for mode in ('interrupt', 'capture-refusal'):
            report['not_run'].remove(mode)
            report['active_case'] = mode
            report['cases'].append(scenario(args, work / mode, mode))
        report.pop('active_case', None)
        report['status'] = 'passed'
    except BaseException as error:
        report.update(status='failed', error_class=type(error).__name__,
            error_frames=[{'file': pathlib.Path(frame.filename).name, 'line': frame.lineno,
                'function': frame.name} for frame in traceback.extract_tb(error.__traceback__)])
    finally:
        try:
            after = inputs(args) if bound is not None else None
        except (OSError, ValueError) as error:
            after = None
            report['input_recheck_error_class'] = type(error).__name__
        report['input_sha256_after'] = after
        report['tested_inputs_stable'] = bound is not None and bound == after
        if not report['tested_inputs_stable']:
            report['status'] = 'failed'
        write_json(work / 'result.json', report)
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            write_json(args.output, report)
    print(json.dumps(report, indent=2))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    sys.exit(main())
