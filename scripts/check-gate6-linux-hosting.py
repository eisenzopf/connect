#!/usr/bin/env python3
"""Native Ubuntu x86 configuration/HTTPS/RTCP checks; no account/live lanes.

Raw compiler/test output and synthetic state stay private. Only source hashes,
tool versions, exit/result counts and artifact hashes enter uploadable evidence.
This is not installation, remote media or whole Gate 6 qualification.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import signal
import shutil
import math
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / ".local/gate6-linux-ci"
EVIDENCE = BASE / "evidence"
PRIVATE = BASE / "private-logs"
sys.path.insert(0, str(ROOT / "scripts/lib"))
from source_build_cache import MANIFEST, snapshot


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def output(name, value):
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a") as stream:
            stream.write(f"{name}={value}\n")


def sources():
    files = {name: {k: v for k, v in value.items() if k != "mtime_ns"} for name, value in snapshot(ROOT).items()}
    for path in sorted((ROOT / ".deps").rglob("*")):
        if ".git" in path.parts or path.is_symlink() or not path.is_file():
            continue
        files[str(path.relative_to(ROOT))] = {"sha256": digest(path), "mode": path.stat().st_mode & 0o777}
    return files


def preflight():
    assert platform.system() == "Linux" and platform.machine() == "x86_64", "Native Linux x86_64 profile required"
    cpuinfo = Path("/proc/cpuinfo").read_text()
    assert re.search(r"^vendor_id\s*:", cpuinfo, re.M), "An x86 kernel is required; an emulated amd64 userland on an ARM kernel is insufficient"
    release = platform.freedesktop_os_release()
    assert release.get("ID") == "ubuntu" and release.get("VERSION_ID") == "24.04", "Ubuntu24.04 profile required"
    rust = subprocess.check_output(["rustc", "-Vv"], text=True)
    assert "rustc 1.97.1 " in rust and "host: x86_64-unknown-linux-gnu" in rust
    native = {
        "rust": rust,
        "clang": subprocess.check_output(["clang", "--version"], text=True),
        "packages": subprocess.check_output(["dpkg-query", "-W", "libopus-dev", "libssl-dev", "libc6-dev", "clang", "g++", "binutils", "pkg-config", "cmake"], text=True),
    }
    key = hashlib.sha256(json.dumps(native, sort_keys=True).encode()).hexdigest()
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    (EVIDENCE / "preflight.json").write_text(json.dumps({"os": release, "platform": platform.platform(),
        "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "runner_image_version": os.environ.get("ImageVersion"), "native_key": key, "toolchain": native,
        "scope": "Native Ubuntu24.04x86 CI fixtures only. No named remote/provider, model account, media/performance, installation or full acceptance qualification."}, indent=2) + "\n")
    output("native_key", key)


def audio_minimum_diagnostic(value):
    assert isinstance(value,dict) and set(value)=={'probe','source_index','receiver_index','allowed_indices','sent_packets','received_packets','non_silent_packets','timestamp_step_min','timestamp_step_max'}
    assert value['probe'] in {'replacement-preparation-remote','replacement-preparation-source','replacement-baseline-remote','replacement-baseline-source','replacement-first-committed-remote','replacement-old-authority-revoked-remote','replacement','consultation','other'}
    assert all(type(value[k]) is int and 0 <= value[k] <= 32 for k in ('source_index','receiver_index'))
    assert all(type(value[k]) is int and 0 <= value[k] <= 100000 for k in ('sent_packets','received_packets','non_silent_packets'))
    assert value['non_silent_packets'] <= 10 and value['non_silent_packets'] <= value['received_packets']
    assert isinstance(value['allowed_indices'],list) and len(value['allowed_indices']) <= 32 and all(type(v) is int and 0 <= v <= 32 for v in value['allowed_indices'])
    assert value['receiver_index'] in value['allowed_indices']
    low,high=value['timestamp_step_min'],value['timestamp_step_max']
    assert (low is None and high is None) or (type(low) is int and type(high) is int and 0 <= low <= high <= 4294967295)
    return value


def audio_quiescence_diagnostic(value):
    assert isinstance(value,dict) and set(value)=={'probe','source_index','sent_packets','minimum_tail_ms','quiet_ms','maximum_tail_ms'}
    assert value['probe'] in {'replacement-preparation-remote','replacement-preparation-source','replacement-baseline-remote','replacement-baseline-source','replacement-first-committed-remote','replacement-old-authority-revoked-remote','replacement','consultation','other'}
    assert type(value['source_index']) is int and 0 <= value['source_index'] <= 32
    assert type(value['sent_packets']) is int and 0 <= value['sent_packets'] <= 100000
    assert all(type(value[k]) is int and value[k]==expected for k,expected in (('minimum_tail_ms',300),('quiet_ms',200),('maximum_tail_ms',2000)))
    return value



NODE_CHECKPOINTS = {'https-snapshot','https-send','https-journal','https-receipts','https-revocation','media-packets','media-av','media-data-setup','media-data-text','media-data-json','media-data-binary','events-bootstrap', 'events-original', 'events-resume', 'events-current-authority', 'create', 'capabilities', 'session', 'native-offer', 'peer-offer',
    'session-finish', 'native-movement-refusals', 'native-quality', 'native-gates', 'native-codecs', 'native-selection', 'native-end', 'native-recovery', 'native-end-http-recovery', 'native-end-status', 'native-end-wait', 'session-end-wait', 'session-end-refresh', 'message-receipts', 'raw-envelope', 'receipt-read','current-revocation'}


def node_progress_diagnostic(value):
    assert isinstance(value,dict) and set(value)=={'algorithm','checkpoint','seconds','deadline_seconds'}
    assert value['algorithm'] in {'EdDSA','ES256'}
    assert type(value['checkpoint']) is str and value['checkpoint'] in NODE_CHECKPOINTS
    assert type(value['seconds']) in (int,float) and 0 <= value['seconds'] <= 90
    assert type(value['deadline_seconds']) is int and value['deadline_seconds']==15
    return value


def node_failure_diagnostic(failure):
    assert set(failure) == {'phase', 'checkpoint', 'kind', 'certainty', 'command', 'observedState', 'observedKind', 'observedCertainty', 'observedHttpStatus', 'line', 'column'}
    assert failure['phase'] == 'failed'
    assert failure['checkpoint'] in NODE_CHECKPOINTS
    assert re.fullmatch(r'[a-z-]{1,64}', failure['kind'])
    assert failure['certainty'] is None or re.fullmatch(r'[a-z-]{1,32}', failure['certainty'])
    assert all(failure[k] is None or type(failure[k]) is int and 0 < failure[k] < 100000 for k in ('line', 'column'))
    assert failure['command'] is None or failure['command'] in {'conversation.create','message.send','operation.cancel','operation.reconcile','session.invite','session.accept','session.update','session.end','session.reject','session.cancel','session.handoff','connection.replace','connection.offer','connection.update','connection.end','stream.subscribe','stream.unsubscribe','operation.get','conversation.get','message.history','conversation.participant.add','conversation.participant.update','conversation.participant.remove'}
    assert failure['observedState'] is None or failure['observedState'] in {'accepted','preparing','ready','committing','completed','cancelled','failed','interrupted','unknown'}
    assert failure['observedKind'] is None or re.fullmatch(r'[a-z-]{1,64}',failure['observedKind'])
    assert failure['observedCertainty'] is None or re.fullmatch(r'[a-z-]{1,32}',failure['observedCertainty'])
    assert failure['observedHttpStatus'] is None or type(failure['observedHttpStatus']) is int and 100 <= failure['observedHttpStatus'] <= 599
    return failure

def python_movement_phase_diagnostic(value):
    assert set(value)=={'algorithm','phase','seconds','deadline_seconds'}
    assert value['algorithm'] in {'EdDSA','ES256'}
    assert value['phase'] in {'registered','accepted','completed','stopped','closed'}
    assert type(value['seconds']) in (int,float) and 0<=value['seconds']<=90
    assert type(value['deadline_seconds']) is int and value['deadline_seconds']==15
    return value


def python_movement_failure_diagnostic(value):
    assert set(value)=={'phase','checkpoint','kind','certainty','line','error_type'}
    assert value['phase']=='failed'
    assert value['checkpoint'] in {'register','recover','begin','completed','stopped','close','invalid'}
    assert re.fullmatch(r'[a-z-]{1,64}',value['kind'])
    assert value['certainty'] is None or re.fullmatch(r'[a-z-]{1,32}',value['certainty'])
    assert value['line'] is None or type(value['line']) is int and 0<value['line']<100000
    assert value['error_type'] in {None,'AssertionError','AttributeError','KeyError','TypeError','ValueError','RuntimeError','OSError','TimeoutError'}
    return value


def python_native_failure_diagnostic(value):
    assert set(value)=={'phase','checkpoint','kind','certainty','source_line','error_type'}
    assert value['phase']=='failed'
    assert value['checkpoint'] in {'initial','participants','sessions','connections','session-decisions',
                                   'message-events','operation-decisions','receipts','restart','revocation',
                                   'data-invite','data-accept','data-offer','data-subscribe','data-send','data-receive',
                                   'data-recovery','data-replay','data-return','data-end','media-av'}
    assert re.fullmatch(r'[a-z-]{1,64}',value['kind'])
    assert value['certainty'] is None or re.fullmatch(r'[a-z-]{1,32}',value['certainty'])
    assert value['source_line'] is None or type(value['source_line']) is int and 0<value['source_line']<100000
    assert value['error_type'] in {'SdkError','AssertionError','UnboundLocalError','AttributeError','KeyError','TypeError',
                                   'ValueError','RuntimeError','OSError','TimeoutError'}
    return value


def python_administrative_failure_diagnostic(value):
    assert set(value)=={'phase','checkpoint','kind','certainty','line','error_type'}
    assert value['phase']=='failed'
    assert value['checkpoint'] in {'ready','apply','read','read-refused','refuse','held','held-cancel','close','invalid'}
    assert re.fullmatch(r'[a-z-]{1,64}',value['kind'])
    assert value['certainty'] is None or re.fullmatch(r'[a-z-]{1,32}',value['certainty'])
    assert value['line'] is None or type(value['line']) is int and 0<value['line']<100000
    assert value['error_type'] in {'SdkError','AssertionError','UnboundLocalError','AttributeError','KeyError','TypeError',
                                   'ValueError','RuntimeError','OSError','TimeoutError'}
    return value


def python_consultation_failure_diagnostic(value):
    assert set(value)=={'phase','checkpoint','kind','certainty','line','error_type'}
    assert value['phase']=='failed'
    assert value['checkpoint'] in {'submit','return-submit','return-wait','return-recover','completed','revoked','close','invalid'}
    assert re.fullmatch(r'[a-z-]{1,64}',value['kind'])
    assert value['certainty'] is None or re.fullmatch(r'[a-z-]{1,32}',value['certainty'])
    assert value['line'] is None or type(value['line']) is int and 0<value['line']<100000
    assert value['error_type'] in {'SdkError','AssertionError','AttributeError','KeyError','TypeError',
                                   'ValueError','RuntimeError','OSError','TimeoutError'}
    return value


def language_av_diagnostic(values, frequencies, channels):
    assert type(values) is list and len(values)==2
    fields={'identified_audio_hz','decoded_audio_samples','audio_rms','source_energy_ratio','identified_video_high_channels','decoded_video_frames','decoded_rgb_bytes','decoder_children_joined'}
    for value,frequency,high in zip(values,frequencies,channels):
        assert type(value) is dict and set(value)==fields
        assert type(value['identified_audio_hz']) in (int,float) and value['identified_audio_hz']==frequency
        assert type(value['identified_video_high_channels']) is list and value['identified_video_high_channels']==high and all(type(v) is bool for v in value['identified_video_high_channels'])
        for name,expected in [('decoded_audio_samples',4800),('decoded_video_frames',6),('decoded_rgb_bytes',640*360*3*6),('decoder_children_joined',2)]:
            assert type(value[name]) is int and value[name]==expected
        assert type(value['audio_rms']) in (int,float) and math.isfinite(value['audio_rms']) and 500<value['audio_rms']<=32768
        assert type(value['source_energy_ratio']) in (int,float) and math.isfinite(value['source_energy_ratio']) and value['source_energy_ratio']>25
    return values


def rtc_operation_failure_diagnostic(value):
    assert type(value) is dict and set(value)=={'checkpoint','state','problem_code','result_present','cancel_requested','effects'}
    assert value['checkpoint'] in {'initial-offer','restart-0','restart-1','other'}
    assert value['state'] in {'accepted','preparing','ready','committing','completed','cancelled','failed','interrupted','unknown'}
    assert value['problem_code'] in {'none','other','forbidden','conflict','reconciliation-required','authority-busy','storage','unsupported-media-profile','webrtc-failed'}
    assert type(value['result_present']) is bool and type(value['cancel_requested']) is bool
    assert type(value['effects']) is list and len(value['effects'])<=4
    for effect in value['effects']:
        assert type(effect) is dict and set(effect)=={'state','attempt'}
        assert effect['state'] in {'prepared','submitting','confirmed','failed','cancelled','unknown'}
        assert type(effect['attempt']) is int and 0<=effect['attempt']<=64
    return value


def ipc_data_revocation_failure_diagnostic(value):
    assert type(value) is dict and set(value)=={'kind','certainty','original_operation_matches'}
    assert value['kind'] in {'authentication','transport-uncertain','closed','deadline','forbidden','backpressure','protocol','unsupported','other','cancelled','validation','configuration','trust','conflict','application','receipt-unavailable','resync-required','cleanup-incomplete'}
    assert value['certainty'] in {'refused','not-submitted','uncertain','read-only','confirmed','other'}
    assert type(value['original_operation_matches']) is bool
    return value


def rtc_ice_worker_failure_diagnostic(value):
    assert type(value) is dict and set(value)=={'phase','code','expected_generation','submitted','receipt_captured','stop_cancelled','physical_cancelled','deadline_elapsed'}
    assert value['phase'] in {'current-authority','original-slot','original-peer','original-identity','rtc-observation','current-slot','physical-current','negotiate','complete','observe','retire','reply-wait'}
    assert value['code'] in {'none','other','forbidden','conflict','reconciliation-required','authority-busy','storage'}
    assert value['expected_generation'] is None or type(value['expected_generation']) is int and 0<=value['expected_generation']<=9007199254740991
    assert all(type(value[key]) is bool for key in ['submitted','receipt_captured','stop_cancelled','physical_cancelled','deadline_elapsed'])
    return value


def rtc_admission_observations(text):
    messages = {'RTC actual admission boundary refused':'actual-boundary',
        'RTC original resource observation refused':'resource-observation',
        'RTC original Session observation refused':'session-observation',
        'RTC initial authority observation refused':'initial-authority',
        'RTC initial receipt persistence refused':'initial-receipt',
        'RTC original media admission refused':'initial-admission',
        'RTC idle observation retired original media':'idle-retirement'}
    codes = {'forbidden','conflict','timeout','closed','authority-busy','storage','storage-unavailable'}
    flags = {'original_revision','physical_closed','runtime_stopped','peer_closed',
        'bindings_match','connection_present','participant_present'}
    observed = []
    for line in text.splitlines():
        line = re.sub(r'\x1b\[[0-9;]*m', '', line)
        phase = next((value for message,value in messages.items() if message in line), None)
        if phase is None:
            continue
        code = re.search(r'\bcode=([a-z-]+)\b', line)
        code = code[1] if code and code[1] in codes else 'other'
        if re.search(r'\berror=Authority\b', line):
            code = 'authority'
        elif re.search(r'\berror=Transition\(Conflict\)', line):
            code = 'transition-conflict'
        observed.append(dict(phase=phase, code=code, flags={name:match[1]=='true' for name in sorted(flags)
            if (match := re.search(r'\b' + name + r'=(true|false)\b', line))}))
        if len(observed) == 16:
            break
    return observed


def consultation_return_observations(text):
    """Keep only declared return boundaries; paths, IDs and errors stay private."""
    stages = {'return-origin', 'return-handle', 'return-original-observation',
        'return-scene', 'return-read', 'return-revision', 'return-plan',
        'return-planned', 'return-prepare', 'return-core'}
    codes = {'forbidden', 'conflict', 'authority-busy', 'storage',
        'storage-unavailable', 'reconciliation-required', 'not-ready'}
    result = []
    for line in text.splitlines():
        line = re.sub(r'\x1b\[[0-9;]*m', '', line)
        if 'connect_service::canonical_consultation' not in line:
            continue
        stage = re.search(r'\bstage="?(return-[a-z-]+)\b', line)
        if not stage or stage[1] not in stages:
            continue
        code = re.search(r'\bcode=([a-z-]+)\b', line)
        row = dict(stage=stage[1], code=code[1] if code and code[1] in codes else 'other')
        current = re.search(r'\bcurrent=(true|false)\b', line)
        if current:
            row['current'] = current[1] == 'true'
        for field in ('source_count', 'receiver_count'):
            count = re.search(r'\b' + field + r'=([0-9]+)\b', line)
            if count and 0 <= int(count[1]) <= 64:
                row[field] = int(count[1])
        result.append(row)
        if len(result) == 32:
            break
    return result


def run(node_sdk=False, python_sdk=False):
    preflight()
    os.environ['CONNECT_GATE6_ICE_DIAGNOSTICS'] = '1'
    PRIVATE.mkdir(parents=True, exist_ok=True, mode=0o700)
    temporary = BASE / "private-tmp"
    temporary.mkdir(exist_ok=True, mode=0o700)
    os.environ["TMPDIR"] = str(temporary)
    codec_fixtures = None
    if node_sdk or python_sdk:
        sys.path.insert(0,str(ROOT/'sdk/tests'))
        from prepare_codec_fixtures import prepare as prepare_codecs
        ffmpeg = shutil.which('ffmpeg')
        assert ffmpeg, 'Explicit SDK media lane requires FFmpeg with libvpx'
        fixture_dir = PRIVATE/'codec-fixtures'
        codec_fixtures = prepare_codecs(fixture_dir,Path(ffmpeg))
        os.environ['CONNECT_GATE6_AV_FIXTURES'] = str(fixture_dir)
        os.environ['CONNECT_GATE6_FFMPEG'] = codec_fixtures['ffmpeg']
    node_package = None
    if node_sdk:
        sys.path.insert(0, str(ROOT / 'sdk/tests'))
        from prepare_native_consumer import prepare
        node_package = prepare(PRIVATE / 'node-native', ROOT / 'dist/gate-6/node-linux-runtime')
        os.environ['CONNECT_GATE6_NODE_HTTPS_CONSUMER'] = str(ROOT / node_package['https_consumer'])
        os.environ['CONNECT_GATE6_NODE_ADMIN_CONSUMER'] = str(ROOT / node_package['administrative_consumer'])
        os.environ['CONNECT_GATE6_NODE_CONSUMER'] = str(ROOT / node_package['consumer'])
        os.environ['CONNECT_GATE6_NODE_CONSULT_CONSUMER'] = str(ROOT / node_package['consultation_consumer'])
        os.environ['CONNECT_GATE6_NODE_MOVEMENT_CONSUMER'] = str(ROOT / node_package['movement_consumer'])
    python_package = None
    if python_sdk:
        sys.path.insert(0, str(ROOT / 'sdk/tests'))
        from prepare_python_native_consumer import prepare as prepare_python
        python_package = prepare_python(PRIVATE / 'python-native')
        os.environ['CONNECT_GATE6_PYTHON'] = str(ROOT / python_package['python'])
        os.environ['CONNECT_GATE6_PYTHON_CONSUMER'] = str(ROOT / python_package['consumer'])
        os.environ['CONNECT_GATE6_PYTHON_MOVEMENT_CONSUMER'] = str(ROOT / python_package['movement_consumer'])
        os.environ['CONNECT_GATE6_PYTHON_ADMIN_CONSUMER'] = str(ROOT / python_package['administrative_consumer'])
        os.environ['CONNECT_GATE6_PYTHON_CONSULT_CONSUMER'] = str(ROOT / python_package['consultation_consumer'])
    before = sources()
    (EVIDENCE / "source-inputs.json").write_text(json.dumps(before, indent=2) + "\n")
    report = {"passed": False, "started_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "steps": [], "runner_sha256": digest(__file__), "source_count": len(before),
              "source_inputs_sha256": digest(EVIDENCE / "source-inputs.json"),
              "scope": "Current snapshot and peer-authority Store regressions, bounded configuration/security, ICE/DTLS owned-close, negotiated RTCP UDP/adjacent allocator, local synthetic SIP readiness/startup/drain and actual HTTPS surface/CLI fixtures. No actual TURN allocation, remote SIP/NAT, browser/performance or installation qualification."}
    if codec_fixtures is not None:
        report['codec_fixtures'] = {k:v for k,v in codec_fixtures.items() if k!='ffmpeg'}
    if node_package is not None:
        # Preserve the exact installed bytes/runtime even if a later scene fails.
        report['installed_node_package'] = node_package
    if python_package is not None:
        report['installed_python_package'] = python_package

    def execute(name, command, timeout, *, defer_failure=False, environment=None):
        log = PRIVATE / (name + ".log")
        started = time.monotonic()
        with log.open("wb") as stream:
            child = subprocess.Popen(command, cwd=ROOT, stdout=stream, stderr=subprocess.STDOUT, start_new_session=True,
                                     env=None if environment is None else dict(os.environ, **environment))
            timed = False
            forced = False
            try:
                child.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                timed = True
                os.killpg(child.pid, signal.SIGTERM)
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    forced = True
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait(timeout=10)
        text = log.read_text(errors="replace")
        summary = [line for line in text.splitlines() if line.startswith("test result:")]
        step = {"name": name, "exit_code": child.returncode, "timed_out": timed, "forced_kill": forced,
                "seconds": time.monotonic() - started, "log_sha256": digest(log), "test_results": summary}
        step['rtc_admission_observations'] = rtc_admission_observations(text) if child.returncode else []
        step['consultation_return_observations'] = consultation_return_observations(text) if child.returncode else []
        step['rtc_admission_trace_enabled'] = []
        for line in text.splitlines():
            if 'rtc-admission-tracing-ready ' in line:
                value = json.loads(line.split('rtc-admission-tracing-ready ',1)[1])
                assert set(value) == {'enabled'} and type(value['enabled']) is bool
                step['rtc_admission_trace_enabled'].append(value['enabled'])
        assert len(step['rtc_admission_trace_enabled']) <= 2
        step["panic_locations"] = [
            {"file": location[0], "line": int(location[1]), "column": int(location[2])}
            for location in re.findall(r"thread '[^']+'(?: \([0-9]+\))? panicked at ([^\n]+?):([0-9]+):([0-9]+):", text, re.M)
        ][:16]
        for key, prefix, validate in (
            ('rtc_operation_failures', 'rtc-operation-failure ', rtc_operation_failure_diagnostic),
            ('ipc_data_revocation_failures', 'ipc-data-revocation-failure ', ipc_data_revocation_failure_diagnostic),
            ('ipc_data_status_revocation_failures', 'ipc-data-status-revocation-failure ', ipc_data_revocation_failure_diagnostic),
            ('rtc_ice_worker_failures', 'rtc-ice-worker-failure ', rtc_ice_worker_failure_diagnostic),
        ):
            values = [validate(json.loads(line[len(prefix):])) for line in text.splitlines() if line.startswith(prefix)]
            assert len(values)<=16, 'Bounded native failure diagnostics exceeded'
            step[key] = values
        step["local_schema_mismatches"] = []
        for line in text.splitlines():
            if line.startswith('sdk-local-schema-mismatch '):
                mismatch = json.loads(line.split(' ', 1)[1])
                assert set(mismatch) == {'id', 'expected', 'observed'}
                assert isinstance(mismatch['id'], str) and re.fullmatch(r'[A-Za-z0-9_.-]{1,160}', mismatch['id'])
                assert type(mismatch['expected']) is bool and type(mismatch['observed']) is bool
                step['local_schema_mismatches'].append(mismatch)
        step['native_audio_boundary_failures'] = []
        for line in text.splitlines():
            if line.startswith('native-audio-boundary-failure '):
                diagnostic = json.loads(line.split(' ',1)[1])
                assert set(diagnostic) == {'probe','source_index','receiver_index','allowed_indices','sent_packets','received_packets','non_silent_packets'}
                assert diagnostic['probe'] in {'replacement-preparation-remote','replacement-preparation-source','replacement-baseline-remote','replacement-baseline-source','replacement-first-committed-remote','replacement-old-authority-revoked-remote','replacement','consultation','other'}
                assert all(type(diagnostic[k]) is int and 0 <= diagnostic[k] <= 100000 for k in ('source_index','receiver_index','sent_packets','received_packets','non_silent_packets'))
                assert isinstance(diagnostic['allowed_indices'],list) and len(diagnostic['allowed_indices']) <= 32 and all(type(v) is int and 0 <= v <= 32 for v in diagnostic['allowed_indices'])
                step['native_audio_boundary_failures'].append(diagnostic)
        step['native_audio_minimum_failures'] = []
        for line in text.splitlines():
            if line.startswith('native-audio-minimum-failure '):
                step['native_audio_minimum_failures'].append(audio_minimum_diagnostic(json.loads(line.split(' ',1)[1])))
        step['native_audio_quiescence_failures'] = []
        for line in text.splitlines():
            if line.startswith('native-audio-quiescence-failure '):
                step['native_audio_quiescence_failures'].append(audio_quiescence_diagnostic(json.loads(line.split(' ',1)[1])))
        step['sdk_current_read_failures'] = []
        for line in text.splitlines():
            if line.startswith('sdk-current-read-failure '):
                failure = json.loads(line.split(' ',1)[1])
                assert set(failure) == {'stage','kind','certainty'} and failure['stage'] in {'revoked-return-read','revoked-consultation-status'}
                assert isinstance(failure['kind'],str) and re.fullmatch(r'[a-z-]{1,64}',failure['kind'])
                assert isinstance(failure['certainty'],str) and re.fullmatch(r'[a-z-]{1,32}',failure['certainty'])
                step['sdk_current_read_failures'].append(failure)
        step["node_completed_phases"] = []
        for line in text.splitlines():
            if line.startswith('installed-typed-node-sdk-phase '):
                phase = json.loads(line.split(' ', 1)[1])
                assert set(phase) == {'algorithm','phase','passed','seconds','deadline_seconds'}
                assert phase['algorithm'] in {'EdDSA','ES256'} and phase['passed'] is True
                assert phase['phase'] in {'controls-ready','native-complete','initial','recovered','receipts-observed','read-observed','completed'}
                assert type(phase['seconds']) in (int,float) and 0 <= phase['seconds'] <= 90 and phase['deadline_seconds'] == 15
                step['node_completed_phases'].append(phase)
        step["node_failures"] = []
        step['node_progress'] = []
        for line in text.splitlines():
            if line.startswith('installed-typed-node-sdk-progress '):
                assert len(step['node_progress'])<128
                step['node_progress'].append(node_progress_diagnostic(json.loads(line.split(' ',1)[1])))
            if line.startswith('installed-typed-node-sdk-failure '):
                failure = json.loads(line.split(' ', 1)[1])
                step['node_failures'].append(node_failure_diagnostic(failure))
        step['node_consultation_phases'] = []
        step['node_consultation_failures'] = []
        for line in text.splitlines():
            if line.startswith('installed-node-consultation-phase '):
                phase = json.loads(line.split(' ', 1)[1])
                assert set(phase) == {'algorithm', 'phase', 'seconds', 'deadline_seconds'}
                assert phase['algorithm'] in {'EdDSA', 'ES256'}
                assert phase['phase'] in {'submitted', 'completed', 'returned', 'revoked', 'closed'}
                assert type(phase['seconds']) in (int, float) and 0 <= phase['seconds'] <= 90 and phase['deadline_seconds'] == 15
                step['node_consultation_phases'].append(phase)
            if line.startswith('installed-node-consultation-failure '):
                failure = json.loads(line.split(' ', 1)[1])
                assert set(failure) == {'phase', 'checkpoint', 'kind', 'certainty', 'observedKind', 'observedCertainty', 'line', 'column'} and failure['phase'] == 'failed'
                assert failure['checkpoint'] in {'submit', 'return-submit', 'return-wait', 'return-recover', 'completed', 'revoked', 'close', 'invalid'}
                assert re.fullmatch(r'[a-z-]{1,64}', failure['kind'])
                assert failure['certainty'] is None or re.fullmatch(r'[a-z-]{1,32}', failure['certainty'])
                assert failure['observedKind'] is None or re.fullmatch(r'[a-z-]{1,64}', failure['observedKind'])
                assert failure['observedCertainty'] is None or re.fullmatch(r'[a-z-]{1,32}', failure['observedCertainty'])
                assert all(failure[k] is None or type(failure[k]) is int and 0 < failure[k] < 100000 for k in ('line', 'column'))
                step['node_consultation_failures'].append(failure)
        step['node_movement_phases'] = []
        step['node_movement_failures'] = []
        for line in text.splitlines():
            if line.startswith('installed-node-movement-phase '):
                phase = json.loads(line.split(' ', 1)[1])
                assert set(phase) == {'algorithm', 'phase', 'seconds', 'deadline_seconds'}
                assert phase['algorithm'] in {'EdDSA', 'ES256'}
                assert phase['phase'] in {'registered', 'accepted', 'completed', 'stopped', 'closed'}
                assert type(phase['seconds']) in (int, float) and 0 <= phase['seconds'] <= 90 and phase['deadline_seconds'] == 15
                step['node_movement_phases'].append(phase)
            if line.startswith('installed-node-movement-failure '):
                failure = json.loads(line.split(' ', 1)[1])
                assert set(failure) == {'phase', 'checkpoint', 'kind', 'certainty', 'line', 'column'} and failure['phase'] == 'failed'
                assert failure['checkpoint'] in {'register', 'recover', 'begin', 'completed', 'stopped', 'close', 'invalid'}
                assert re.fullmatch(r'[a-z-]{1,64}', failure['kind'])
                assert failure['certainty'] is None or re.fullmatch(r'[a-z-]{1,32}', failure['certainty'])
                assert all(failure[k] is None or type(failure[k]) is int and 0 < failure[k] < 100000 for k in ('line', 'column'))
                step['node_movement_failures'].append(failure)
        step['python_movement_phases'] = []
        step['python_movement_failures'] = []
        step['python_native_failures'] = []
        step['python_consultation_phases'] = []
        step['python_consultation_failures'] = []
        for line in text.splitlines():
            if line.startswith('installed-python-movement-phase '):
                step['python_movement_phases'].append(python_movement_phase_diagnostic(json.loads(line.split(' ',1)[1])))
            if line.startswith('installed-python-movement-failure '):
                step['python_movement_failures'].append(python_movement_failure_diagnostic(json.loads(line.split(' ',1)[1])))
            if line.startswith('installed-python-native-failure '):
                step['python_native_failures'].append(python_native_failure_diagnostic(json.loads(line.split(' ',1)[1])))
            if line.startswith('installed-python-consultation-failure '):
                step['python_consultation_failures'].append(python_consultation_failure_diagnostic(json.loads(line.split(' ',1)[1])))
            if line.startswith('installed-python-consultation-phase '):
                phase=json.loads(line.split(' ',1)[1])
                assert set(phase)=={'algorithm','phase','seconds','deadline_seconds'}
                assert phase['algorithm'] in {'EdDSA','ES256'} and phase['phase'] in {'submitted','returned','completed','revoked','closed'}
                assert type(phase['seconds']) in (int,float) and 0<=phase['seconds']<=90 and phase['deadline_seconds']==15
                step['python_consultation_phases'].append(phase)
        step['node_administrative_phases'] = []
        step['node_administrative_failures'] = []
        step['node_administrative_drain_failures'] = []
        step['python_administrative_phases'] = []
        step['python_administrative_failures'] = []
        step['python_administrative_drain_failures'] = []
        for line in text.splitlines():
            if line.startswith('installed-python-administrative-drain-failure '):
                drain = json.loads(line.split(' ',1)[1])
                assert set(drain)=={'physical_peers','media_readers','helpers_joined','validators_joined'}
                assert all(type(v) is int and 0<=v<=100000 for v in drain.values())
                step['python_administrative_drain_failures'].append(drain)
            if line.startswith('installed-python-administrative-phase '):
                phase=json.loads(line.split(' ',1)[1])
                assert set(phase)=={'algorithm','phase','seconds','deadline_seconds'}
                assert phase['algorithm'] in {'EdDSA','ES256'} and phase['phase'] in {'ready','applied','read','read-refused','submitted','uncertain','refused','closed'}
                assert type(phase['seconds']) in (int,float) and 0<=phase['seconds']<=90 and phase['deadline_seconds']==15
                step['python_administrative_phases'].append(phase)
            if line.startswith('installed-python-administrative-failure '):
                step['python_administrative_failures'].append(python_administrative_failure_diagnostic(json.loads(line.split(' ',1)[1])))
            if line.startswith('installed-node-administrative-drain-failure '):
                drain = json.loads(line.split(' ',1)[1])
                assert set(drain) == {'physical_peers','media_readers','helpers_joined','validators_joined'}
                assert all(type(v) is int and 0 <= v <= 100000 for v in drain.values())
                step['node_administrative_drain_failures'].append(drain)
            if line.startswith('installed-node-administrative-phase '):
                phase = json.loads(line.split(' ', 1)[1])
                assert set(phase) == {'algorithm', 'phase', 'seconds', 'deadline_seconds'}
                assert phase['algorithm'] in {'EdDSA', 'ES256'} and phase['phase'] in {'ready','applied','read','read-refused','submitted','uncertain','refused','closed'}
                assert type(phase['seconds']) in (int,float) and 0 <= phase['seconds'] <= 90 and phase['deadline_seconds'] == 15
                step['node_administrative_phases'].append(phase)
            elif line.startswith('installed-node-administrative-failure '):
                failure = json.loads(line.split(' ', 1)[1])
                assert set(failure) == {'phase','checkpoint','kind','certainty','observedKind','observedCertainty','line','column'} and failure['phase'] == 'failed'
                assert failure['checkpoint'] in {'ready','apply','read','read-refused','refuse','held','held-cancel','close','invalid'}
                assert failure['observedKind'] is None or re.fullmatch(r'[a-z-]{1,64}',failure['observedKind'])
                assert failure['observedCertainty'] is None or re.fullmatch(r'[a-z-]{1,32}',failure['observedCertainty'])
                assert re.fullmatch(r'[a-z-]{1,64}',failure['kind'])
                assert failure['certainty'] is None or re.fullmatch(r'[a-z-]{1,32}',failure['certainty'])
                assert all(failure[k] is None or type(failure[k]) is int and 0 < failure[k] < 100000 for k in ('line','column'))
                step['node_administrative_failures'].append(failure)
        if child.returncode != 0:
            diagnostics = []
            for line in text.splitlines():
                if not line.startswith("{"):
                    continue
                try:
                    item = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if item.get("reason") == "compiler-message" and item["message"]["level"] == "error":
                    message = item["message"]
                    diagnostics.append({"code": (message.get("code") or {}).get("code"), "message": message["message"],
                                        "locations": [{k: span[k] for k in ("file_name", "line_start", "column_start")} for span in message["spans"]]})
            step["compiler_diagnostics"] = diagnostics
        report["steps"].append(step)
        print(json.dumps(step), flush=True)
        if defer_failure and (child.returncode != 0 or timed or forced):
            return None
        assert child.returncode == 0 and not timed and not forced, f"Phase failed: {name}; raw output retained privately"
        return text

    artifacts = {}
    try:
        for name, wanted, arguments in (
            ("client", "connect_client", ["--offline", "-p", "connect-client", "--lib"]),
            ("store", "connect_store", ["--offline", "-p", "connect-store", "--lib"]),
            ("library", "connect_server", ["--offline", "-p", "connect-server", "--features", "test-support", "--lib"]),
            ("hosting", "gate6_canonical_runtime", ["--offline", "-p", "connect-server", "--features", "test-support", "--test", "gate6_canonical_runtime"]),
            ("webtransport", "gate6_canonical_webtransport", ["--offline", "-p", "connect-server", "--features", "test-support", "--test", "gate6_canonical_webtransport"]),
            # Targeted locked upstream compilation may fetch its registry-only
            # test dependencies; it does not execute providers or remote lanes.
            ("rtp", "rvoip_rtp_core", ["--manifest-path", ".deps/rvoip/Cargo.toml", "-p", "rvoip-rtp-core", "--lib"]),
        ):
            text = execute("compile-" + name, ["cargo", "test", "--locked", "--jobs", "2",
                           *arguments, "--no-run", "--message-format=json"], 3600)
            rows = [json.loads(line) for line in text.splitlines() if line.startswith("{")]
            assert any(row.get("reason") == "build-finished" and row.get("success") for row in rows)
            executable = [row["executable"] for row in rows if row.get("reason") == "compiler-artifact"
                          and row["target"]["name"] == wanted and row["profile"]["test"] and row.get("executable")]
            assert len(executable) == 1
            artifacts[name] = executable[0]
            if (node_sdk or python_sdk) and name == 'hosting':
                cli = {row['executable'] for row in rows if row.get('reason') == 'compiler-artifact'
                       and row['target']['name'] == 'connect' and row.get('executable')
                       and not row['profile']['test']}
                assert len(cli) == 1, 'Missing exact compiled CLI for installed SDK fixture'
                cli = Path(cli.pop())
                assert cli.stat().st_size > 1_000_000 and os.access(cli, os.X_OK)
                with cli.open('rb') as stream:
                    assert stream.read(4) == b'\x7fELF', 'Native Linux CLI header required'
                execute('node-cli-preflight', [str(cli), 'sdk-stdio', '--help'], 10)
                artifacts['cli'] = str(cli)
        assert sources() == before, "Sources changed during compilation"
        # A failed compile must never stamp source times as a completed build.
        MANIFEST.write_text(json.dumps({"format": 1, "files": snapshot(ROOT)}, sort_keys=True) + "\n")
        report["compiled_artifacts"] = {name: digest(path) for name, path in artifacts.items()}
        inventory = execute("snapshot-test-inventory", [artifacts['store'], '--list'], 10)
        for case in (
            'single_credential_snapshot_checks_current_projection_without_duplicate_source_captures',
            'multi_credential_snapshot_keeps_each_credentials_projection_revalidation',
            'snapshot_source_shortcut_cannot_replace_projection_coverage_or_current_key_checks',
        ):
            assert f'peer_authority::tests::{case}: test' in inventory.splitlines()
        sdk_failures = []
        for name, binary, pattern, expected in (
            ("sdk-event-delivery", "client", "sdk::events::helper_delivery_tests::", 3),
            ("sdk-events", "client", "canonical::tests::sdk_events::", 10),
            ("sdk-ice-receipts", "client", "sdk::connection_updates::ice_receipt_tests::", 2),
            ("sdk-managed-delivery", "client", "sdk::helper::managed::tests::", 3),
            ("sdk-media-ipc-framing", "client", "sdk::helper::media_ipc::tests::", 6),
            ("sdk-media-ipc-routing", "client", "sdk::helper::media::", 6),
            ("sdk-media-ipc-runtime", "hosting", "sdk_handles::media_ipc::", 2),
            ("sdk-startup-context", "library", "cli::sdk_stdio::tests::", 3),
            ("admin-local-validation", "hosting", "sdk_handles::administration::local_administrative_validator_", 3),
            ("raw-local-validation", "hosting", "sdk_handles::raw::local_helper_validator_", 3),
            ("snapshot-store", "store", "events::tests::", 1),
            ("snapshot-peer-authority", "store", "peer_authority::tests::", 3),
            ("native-offer-cleanup-generation", "store", "connection_control::tests::native_wire::native_cleanup_", 2),
            ("native-ice-custody", "store", "connection_control::tests::native_wire::webrtc_ice::", 15),
            ("native-ice-restart", "webtransport", "canonical_websocket_control::webrtc::subscriptions::signed_rtc_ice_restart_", 2),
            ("native-candidate-retirement", "store", "connection_control::tests::replacement_sources::candidate::", 8),
            ("native-endpoint-reservation-contention", "store", "connection_control::tests::replacement_sources::custody::endpoint_reservation_refuses_contended_sql_or_authority_without_custody_or_effect", 1),
            ("native-return-reservation-contention", "store", "connection_control::tests::consultation_returns::native_return_reservation_refuses_contended_sql_or_authority_without_effect", 1),
            ("native-candidate-transfer", "store", "connection_control::tests::replacement_sources::commit::pending_candidate_commit_transfers_exact_public_slot_and_old_cleanup_cannot_retire_successor", 1),
            ("native-candidate-recovery", "store", "connection_control::tests::replacement_sources::commit::candidate_adoption_successor_recovery_never_reconstructs_old_offer_from_new_mapping", 1),
            ("rust-sdk-consultation-audio", "hosting", "canonical_native_connections::consultation::sdk_controls::production_public_rust_sdk_consultation_return_retains_actual_private_and_restored_audio", 1),
            ("rust-sdk-consultation-eof", "hosting", "canonical_native_connections::consultation::sdk_controls::production_public_rust_sdk_consultation_return_after_remote_eof_reports_no_continuing_call", 1),
            ("addressing", "library", "canonical_webrtc::addressing::tests::", 4),
            ("config", "library", "canonical_webrtc::config::tests::", 4),
            ("sip-config", "library", "canonical_telephone::tests::", 4),
            ("rtcp-udp", "rtp", "transport::udp::tests::", 32),
            ("rtcp-allocator", "rtp", "transport::allocator::tests::", 20),
            ("rtcp-session-route", "rtp", "session::tests::negotiated_rtcp_manual_reports_and_close_use_original_route", 1),
            ("owned-dtls", "library", "canonical_webrtc::tests::store_bound_peer_completes_actual_dtls_and_original_close_releases_capacity", 1),
            ("https-hosting", "hosting", "canonical_https_hosting::", 7),
            ("sip-unknown-account", "hosting", "canonical_native_connections::consultation::groups::replacement::telephone::configured::production_telephone_configured_unknown_account_refuses_before_binding_and_releases_installation", 1),
            ("sip-bind-failure", "hosting", "canonical_native_connections::consultation::groups::replacement::telephone::configured::production_telephone_configured_later_http_bind_failure_drains_started_sip", 1),
            ("sip-owner-drop", "hosting", "canonical_native_connections::consultation::groups::replacement::telephone::configured::production_telephone_configured_server_drop_fences_owner_and_drains_sip", 1),
            ("sip-readiness", "hosting", "canonical_native_connections::consultation::groups::replacement::telephone::initial::activation::production_mixed_session_activation_waits_for_actual_native_and_sip_readiness_with_exact_capabilities", 1),
        ):
            independent_sdk = name in {'rust-sdk-consultation-audio', 'rust-sdk-consultation-eof'}
            selection = [artifacts[binary], pattern]
            if name == 'https-hosting':
                # These two installed consumers are explicitly required below;
                # they are not part of the seven non-ignored surface tests.
                selection += ['--skip', 'canonical_https_hosting::node::']
                report['baseline_https_exclusion'] = {'pattern':'canonical_https_hosting::node::','functions':2,
                    'reason':'Installed Node consumers require their explicit environment and --ignored selection.',
                    'explicit_sdk_matrix_requested':node_sdk,'qualified_by_baseline':False}
            text = execute(name, selection + ["--test-threads=1"], 90, defer_failure=independent_sdk,
                environment={'RUST_LOG':'connect_service::canonical_webrtc=debug'} if name == 'native-ice-restart'
                else {'RUST_LOG':'connect_service::canonical_consultation=debug'} if independent_sdk else None)
            if text is None:
                sdk_failures.append(name)
                continue
            counts = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;", text)
            assert len(counts) == 1 and int(counts[0][0]) >= expected and counts[0][1:] == ("0", "0"), "Missing/skipped fixture coverage"
        def sdk_case(name, case, environment=None):
            text = execute(name, [artifacts['hosting'], case, '--exact', '--ignored', '--nocapture', '--test-threads=1'],
                           90, defer_failure=True, environment=environment)
            if text is None:
                sdk_failures.append(name)
            return text
        if node_sdk:
            https_results = []
            for algorithm, label in (('EdDSA','eddsa'),('ES256','es256')):
                case = 'canonical_https_hosting::node::installed_node_' + label + '_https_agrees_with_native_and_enforces_current_authority'
                text = sdk_case('installed-node-https-' + label, case)
                if text is None:
                    https_results.append({'algorithm':algorithm,'passed':False})
                    continue
                assert re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;',text) == [('1','0','0')]
                witnesses=[json.loads(line.split('installed-node-https-witness ',1)[1]) for line in text.splitlines() if 'installed-node-https-witness ' in line]
                assert len(witnesses)==1
                witness=witnesses[0]
                assert witness['profile']=='node-https/3' and witness['algorithm']==algorithm
                assert all(witness[field] for field in ('independent_native_original_record','independent_store_original_cause','one_original_message_and_operation','zero_forbidden_message_and_operation','node_process_joined','physical_peers_drained','media_readers_drained'))
                assert re.fullmatch(r'[0-9a-f]{64}',witness['initial']['event_id_sha256'])
                assert all(witness['initial'][f] for f in ('current_https_journal','original_replay_record','application_queued_receipt_current'))
                assert witness['queued_receipt_without_delivery_or_read']
                assert all(witness['completed'][f] for f in ('current_https_read_refused','current_https_replay_refused','current_https_new_mutation_refused','current_https_receipts_refused'))
                assert witness['helper_processes']==0 and witness['servers_joined']==2
                https_results.append(dict(witness,passed=True))
            report['installed_node_https']={'passed':all(v['passed'] for v in https_results),'profile':'node-https/3','results':https_results,'request_budget_ms':4000,'phase_deadline_seconds':15,'whole_function_deadline_seconds':90,'scope':'Explicit HTTPS facade against original local production TLS application surface; platform TLS validation with isolated fixture root, original signed native record, independent full Store causal link and exact journal row, queued journal receipt without delivery/read, current revocation, finite journal, one-effect replay, no forbidden mutation and original process/server drains. No named remote/account/full SDK qualification.'}
            administrative_results = []
            for algorithm, label in (('EdDSA','eddsa'),('ES256','es256')):
                for scenario, suffix in (('commands','covers_all_commands_and_original_recovery'),
                                         ('authority','enforces_current_authority_and_original_recovery'),
                                         ('uncertainty','recovers_held_committed_receipt_without_replay')):
                    name = 'installed_node_' + label + '_administration_' + suffix
                    text = sdk_case('installed-node-administration-' + label + '-' + scenario, 'sdk_handles::administration::' + name, {'CONNECT_GATE6_NODE_ADMIN':'1'})
                    if text is None:
                        administrative_results.append({'algorithm':algorithm,'scenario':scenario,'passed':False})
                        continue
                    assert re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;',text) == [('1','0','0')]
                    witnesses = [json.loads(line.split('installed-node-administrative-witness ',1)[1]) for line in text.splitlines() if 'installed-node-administrative-witness ' in line]
                    assert len(witnesses) == 1
                    witness = witnesses[0]
                    assert witness['profile'] == 'node-administration/3' and witness['algorithm'] == algorithm and witness['scenario'] == scenario
                    if scenario == 'commands':
                        assert witness['command_families'] == 11
                        assert witness['helpers_joined'] == 2 and witness['validators_joined'] == 26 and witness['servers_joined'] == 2
                        assert all(witness[field] for field in ('original_record_recovered','one_original_receipt','changed_intent_refused'))
                    elif scenario == 'authority':
                        assert witness['helpers_joined'] == 4 and witness['validators_joined'] == 10 and witness['servers_joined'] == 1
                        assert all(witness[field] for field in ('ordinary_submit_read_refused','revoked_current_read_refused',
                            'revoked_startup_replay_and_submit_not_submitted','independent_authorized_original_record_recovered','no_forbidden_or_revoked_receipts'))
                    else:
                        assert witness['helpers_joined'] == 2 and witness['validators_joined'] == 4 and witness['servers_joined'] == 1
                        assert all(witness[field] for field in ('actual_commit_held_response','original_cancelled_uncertain','queued_cancelled_not_submitted',
                            'fresh_original_record_recovered_without_replay','one_original_receipt','zero_queued_receipts','no_hidden_resubmission'))
                    assert witness['physical_peers_drained'] and witness['media_readers_drained']
                    administrative_results.append(dict(witness,passed=True))
            assert len(administrative_results) == 6
            report['installed_node_administration'] = {'passed': all(v['passed'] for v in administrative_results), 'profile':'node-administration/3',
                'results':administrative_results,'request_budget_ms':4000,'phase_deadline_seconds':15,'whole_function_deadline_seconds':90,
                'verified_helpers_joined':sum(v.get('helpers_joined',0) for v in administrative_results),
                'verified_validators_joined':sum(v.get('validators_joined',0) for v in administrative_results),
                'verified_servers_joined':sum(v.get('servers_joined',0) for v in administrative_results),
                'scope':'All11 actual trusted administrative commands and ordinary/current-revoked authority via installed Node SDK; independent Rust signed journal comparisons, exact replay after server restart, new revoked startup refusing original/new mutations without submission, independent authorized recovery, no unauthorized receipts and original joins/drains. Actual held HTTP commit/cancellation preserves original uncertainty, queued no-submission and fresh signed recovery without replay. Full SDK qualification remains separate.'}
            movement_results = []
            movement_cases = []
            for algorithm, label in (('EdDSA', 'eddsa'), ('ES256', 'es256')):
                suffixes = [f'{scope}_handoff_{outgoing}' for scope in ('session', 'conversation') for outgoing in ('observe', 'leave')]
                suffixes += ['move_candidate']
                suffixes += ['move_' + case for case in ('speaker', 'observer', 'revoked_endpoint', 'revoked_target_key')]
                suffixes += ['cancel_' + case for case in ('speaker', 'observer', 'revoked_endpoint')]
                suffixes += ['move_recording_failure']
                movement_cases.extend((f'installed_node_{label}_{suffix}', algorithm) for suffix in suffixes)
            assert len(movement_cases) == len(set(name for name, _ in movement_cases)) == 26
            for name, algorithm in movement_cases:
                scenario, algorithms, cases = name, [algorithm], 1
                case = 'canonical_native_connections::consultation::groups::sdk_movements::' + name
                text = sdk_case('installed-node-movement-' + scenario, case, {'CONNECT_GATE6_NODE_MOVEMENTS': '1'})
                if text is None:
                    movement_results.append({'scenario': scenario, 'passed': False})
                    continue
                assert re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', text) == [('1', '0', '0')]
                witnesses = [json.loads(line.split('installed-node-movement-witness ', 1)[1])
                             for line in text.splitlines() if 'installed-node-movement-witness ' in line]
                assert len(witnesses) == 1
                witness = witnesses[0]
                assert witness['profile'] == 'node-movements/3' and witness['scenario'] == scenario
                assert witness['algorithms'] == algorithms and witness['cases'] == cases
                assert witness['helpers_joined'] == cases * 2 and witness['servers_joined'] == cases
                assert witness['physical_peers_drained'] and witness['media_readers_drained']
                movement_results.append(dict(witness, passed=True))
            report['installed_node_movements'] = {'passed': all(v['passed'] for v in movement_results),
                'profile': 'node-movements/3', 'results': movement_results, 'request_budget_ms': 4000,
                'phase_deadline_seconds': 15, 'whole_function_deadline_seconds': 90,
                'scope': 'Actual native peers/audio with installed public Node controller; topology/role/binding match host result, fresh original-record recovery, current revocation/cancellation/known-commit failure, original helpers/servers/drains. No model/provider/remote or full SDK qualification.'}
            consultation_results = []
            for scenario, name, helpers in (
                ('restored', 'installed_node_consultation_and_return_preserve_actual_private_and_restored_audio', 3),
                ('remote-ended', 'installed_node_consultation_return_after_remote_eof_cannot_claim_a_continuing_call', 2),
            ):
                case = 'canonical_native_connections::consultation::sdk_controls::node_controls::' + name
                text = sdk_case('installed-node-consultation-' + scenario, case)
                if text is None:
                    consultation_results.append({'scenario': scenario, 'passed': False})
                    continue
                assert re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', text) == [('1', '0', '0')]
                witnesses = [json.loads(line.split('installed-node-consultation-witness ', 1)[1])
                             for line in text.splitlines() if 'installed-node-consultation-witness ' in line]
                assert len(witnesses) == 2 and {w['algorithm'] for w in witnesses} == {'EdDSA', 'ES256'}
                for witness in witnesses:
                    assert witness['audio_case'] == scenario and witness['current_return_authority']
                    assert witness['original_record_recovered'] and witness['helpers_joined'] == helpers and witness['server_joined']
                    assert witness['physical_peers_drained'] and witness['media_readers_drained']
                consultation_results.append({'scenario': scenario, 'passed': True,
                    'helpers_joined': sum(w['helpers_joined'] for w in witnesses), 'servers_joined': len(witnesses)})
            report['installed_node_consultation_audio'] = {'passed': all(v['passed'] for v in consultation_results),
                'results': consultation_results, 'algorithms': ['EdDSA', 'ES256'],
                'whole_function_deadline_seconds': 90, 'request_budget_ms': 4000, 'phase_deadline_seconds': 15,
                'scenarios': ['restored', 'remote-ended'], 'required_original_helpers_joined': 10, 'required_servers_joined': 4,
                'verified_original_helpers_joined': sum(v.get('helpers_joined', 0) for v in consultation_results),
                'verified_servers_joined': sum(v.get('servers_joined', 0) for v in consultation_results),
                'scope': 'Public installed Node controls with actual native private/restored audio and remote EOF; no model/provider/remote lane or full SDK qualification.'}
            report['scope'] += ' Also installed Node private/restored decoded audio and remote-EOF controls.'
            case = 'canonical_stdio::node_sdk::installed_typed_node_sdk_recovers_typed_operations_and_current_authority'
            text = sdk_case('installed-node-native-control', case)
            if text is None:
                report['installed_node_native'] = {'passed': False, 'functional_phase_profile': 'node-controls/11'}
            else:
                counts = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', text)
                assert counts == [('1', '0', '0')]
                witnesses = [json.loads(line.split('installed-typed-node-sdk-witness ', 1)[1])
                             for line in text.splitlines() if 'installed-typed-node-sdk-witness ' in line]
                assert len(witnesses) == 2
                assert {w['completed']['algorithm'] for w in witnesses} == {'EdDSA', 'ES256'}
                for witness in witnesses:
                    assert witness['functional_phase_profile'] == 'node-controls/11'
                    assert witness['controls']['session_participants_accepted']
                    assert all(witness['controls'][field] for field in ('incoming_original_hint','incoming_session_decision','incoming_close_control_live'))
                    assert re.fullmatch(r'[0-9a-f]{64}',witness['controls']['incoming_source_sha256'])
                    assert all(witness['initial'][f] for f in ('events_original_record','events_cursor_persisted'))
                    assert re.fullmatch(r'[0-9a-f]{64}',witness['initial']['event_id_sha256'])
                    assert all(witness['recovered'][f] for f in ('events_resume_original_record','events_resume_http_record','events_resume_new_fact'))
                    assert re.fullmatch(r'[0-9a-f]{64}',witness['recovered']['events_resume_event_id_sha256'])
                    assert isinstance(witness['recovered']['events_resume_record_id'],str) and witness['recovered']['events_resume_record_id']
                    assert witness['original_and_resumed_causal_store_verified']
                    assert witness['completed']['events_current_authority_no_data'] and witness['completed']['events_close_control_live']
                    assert witness['completed']['events_authority_outcome'] in {'request-failed','original-source-ended'}
                    assert witness['native']['native_controls_completed']
                    assert witness['native_data_independent_store'] and witness['servers_joined']==2
                    assert witness['media']['native_av_utility_capture'] is True
                    language_av_diagnostic(witness['decoded_language_av'],[660,440],[[False,False,True],[True,False,False]])
                    assert all(witness['media'][f] for f in ('native_media_negotiation','native_rtp_local_queue','native_data_typed_round_trip','native_data_full_64k','native_data_original_recovery','native_data_one_effect_replay','native_data_changed_intent_refused','native_data_foreign_custody_refused'))
                    assert type(witness['media']['native_clock_local_queue']) is bool and type(witness['media']['native_clock_unnegotiated_refused']) is bool and witness['media']['native_clock_local_queue']!=witness['media']['native_clock_unnegotiated_refused']
                    assert witness['initial']['movement_current_authority_refused']
                    assert witness['initial']['node_capability_support_only']
                    assert witness['initial']['native_connection_controls'] and witness['initial']['native_selection']
                    assert witness['initial']['native_http_record'] and witness['initial']['observation_only_stream']
                    assert witness['recovered']['typed_connection_end_recovered']
                    assert witness['receipts']['current_delivery_fact'] and witness['completed']['typed_receipts_current_authority']
                    assert all(witness['receipts'][field] for field in ('raw_signed_original_record', 'raw_same_id_one_message', 'raw_local_configuration_independent'))
                    assert witness['receipts']['http_same_delivery_fact']
                    assert witness['completed']['http_receipts_current_authority'] and witness['completed']['http_receipts_status'] in (401,403)
                    assert witness['completed']['raw_current_authority']
                    assert witness['completed']['helpers_joined'] == 5 and witness['completed']['local_validators_joined'] == 3
                    assert witness['server_shutdown_joined'] and witness['physical_peers_drained'] and witness['media_readers_drained']
                report['installed_node_native'] = {'passed': True, 'package': node_package,
                    'functional_phase_profile': 'node-controls/11', 'request_budget_ms': 4000,
                    'scenario_deadline_seconds': 15, 'whole_function_deadline_seconds': 90,
                    'algorithms': ['EdDSA', 'ES256'], 'original_helpers_joined': 10, 'local_validators_joined': 6, 'servers_joined': 4,
                    'causal_store_and_exact_journal_witnesses_per_algorithm': 2,
                    'physical_peers_drained': True, 'media_readers_drained': True,
                    'scope': 'Installed current SDK through actual Rust/native/HTTP control, full typed data and independently decoded identified synthetic PCMU/VP8 in both directions. No named remote/provider/model or full SDK qualification.'}
                report['scope'] += ' Also one installed Node native control function for both algorithms, separate callers/peers, identified independently decoded synthetic AV and original drains.'
        if python_sdk:
            results = []
            for algorithm, label in (('EdDSA', 'eddsa'), ('ES256', 'es256')):
                case = 'python_sdk::installed_python_' + label + '_native_recovers_after_restart_and_enforces_current_authority'
                text = sdk_case('installed-python-native-' + label, case)
                if text is None:
                    results.append({'algorithm': algorithm, 'passed': False})
                    continue
                assert re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', text) == [('1', '0', '0')]
                witnesses = [json.loads(line.split('installed-python-native-witness ', 1)[1]) for line in text.splitlines() if 'installed-python-native-witness ' in line]
                assert len(witnesses) == 1
                witness = witnesses[0]
                assert witness['profile'] == 'python-native/6' and witness['algorithm'] == algorithm
                language_av_diagnostic(witness['decoded_language_av'],[1100,880],[[False,True,True],[True,True,False]])
                for field in ('original_reference_recovered', 'same_id_replay', 'changed_intent_refused', 'typed_capability_refusal',
                              'signed_finite_journal', 'same_cause_after_restart', 'current_read_refused',
                              'current_replay_refused', 'current_new_mutation_refused', 'independent_store_original_cause',
                              'one_original_message_and_operation', 'zero_forbidden_message_and_operation',
                              'default_helper_on_path', 'python_process_joined', 'physical_peers_drained', 'media_readers_drained',
                              'participant_epoch_advanced', 'stale_revisions_refused', 'incoming_original_hint',
                              'session_decisions_completed', 'native_controls_completed', 'stream_selection_completed',
                              'independent_control_records', 'typed_message_event', 'native_media_negotiation', 'native_rtp_local_queue', 'native_data_typed_round_trip', 'native_data_full_64k',
                              'native_data_original_recovery', 'native_data_one_effect_replay', 'native_data_changed_intent_refused',
                              'native_data_foreign_custody_refused', 'native_data_independent_store', 'local_wait_cancelled_read_only',
                              'durable_cancel_original_preserved', 'unattributed_reconciliation_refused',
                              'canonical_http_same_original_record', 'independent_http_receipt_agreement',
                              'receipt_queued_without_delivery', 'attributed_delivery', 'read_default_withheld_then_opted_in',
                              'attributed_read', 'current_receipts_refused', 'synchronous_original_recovered'):
                    assert witness[field] is True
                assert (witness['helpers_joined'], witness['validators_joined'], witness['servers_joined']) == (10, 5, 3)
                assert witness['http_workers_joined'] == 2
                assert type(witness['native_clock_local_queue']) is bool and type(witness['native_clock_unnegotiated_refused']) is bool
                assert witness['native_clock_local_queue'] != witness['native_clock_unnegotiated_refused']
                results.append(dict(witness, passed=True))
            report['installed_python_native'] = {'passed': all(v['passed'] for v in results),
                'profile': 'python-native/6', 'results': results, 'request_budget_ms': 4000,
                'phase_deadline_seconds': 15, 'whole_function_deadline_seconds': 90,
                'scope': 'Clean installed wheel, original signed native Rust helper/server, participant epochs and stale revisions, Session decisions and incoming hints, native Connection/Stream controls, typed and finite journal agreement, saved reference across actual restart, current revocation, exact one-effect and joined original processes. Identified synthetic PCMU/VP8 decoded independently in both directions. No named remote/provider/model or full SDK qualification.'}
            administrative_results = []
            for algorithm, label in (('EdDSA','eddsa'),('ES256','es256')):
                for scenario, suffix in (('commands','covers_all_commands_and_original_recovery'),
                                         ('authority','enforces_current_authority_and_original_recovery'),
                                         ('uncertainty','recovers_held_committed_receipt_without_replay')):
                    name = 'installed_python_' + label + '_administration_' + suffix
                    text = sdk_case('installed-python-administration-' + label + '-' + scenario, 'sdk_handles::administration::' + name, {'CONNECT_GATE6_PYTHON_ADMIN':'1'})
                    if text is None:
                        administrative_results.append({'algorithm':algorithm,'scenario':scenario,'passed':False})
                        continue
                    assert re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;',text) == [('1','0','0')]
                    witnesses = [json.loads(line.split('installed-python-administrative-witness ',1)[1]) for line in text.splitlines() if 'installed-python-administrative-witness ' in line]
                    assert len(witnesses) == 1
                    witness = witnesses[0]
                    assert witness['profile'] == 'python-administration/1' and witness['algorithm'] == algorithm and witness['scenario'] == scenario
                    if scenario == 'commands':
                        assert witness['command_families'] == 11
                        assert witness['helpers_joined'] == 2 and witness['validators_joined'] == 26 and witness['servers_joined'] == 2
                        assert all(witness[field] for field in ('original_record_recovered','one_original_receipt','changed_intent_refused'))
                    elif scenario == 'authority':
                        assert witness['helpers_joined'] == 4 and witness['validators_joined'] == 10 and witness['servers_joined'] == 1
                        assert all(witness[field] for field in ('ordinary_submit_read_refused','revoked_current_read_refused',
                            'revoked_startup_replay_and_submit_not_submitted','independent_authorized_original_record_recovered','no_forbidden_or_revoked_receipts'))
                    else:
                        assert witness['helpers_joined'] == 2 and witness['validators_joined'] == 4 and witness['servers_joined'] == 1
                        assert all(witness[field] for field in ('actual_commit_held_response','original_cancelled_uncertain','queued_cancelled_not_submitted',
                            'fresh_original_record_recovered_without_replay','one_original_receipt','zero_queued_receipts','no_hidden_resubmission'))
                    assert witness['physical_peers_drained'] and witness['media_readers_drained']
                    administrative_results.append(dict(witness,passed=True))
            assert len(administrative_results) == 6
            report['installed_python_administration'] = {'passed': all(v['passed'] for v in administrative_results), 'profile':'python-administration/1',
                'results':administrative_results,'request_budget_ms':4000,'phase_deadline_seconds':15,'whole_function_deadline_seconds':90,
                'verified_helpers_joined':sum(v.get('helpers_joined',0) for v in administrative_results),
                'verified_validators_joined':sum(v.get('validators_joined',0) for v in administrative_results),
                'verified_servers_joined':sum(v.get('servers_joined',0) for v in administrative_results),
                'scope':'All11 actual trusted administrative commands and ordinary/current-revoked authority via installed Python SDK; independent Rust signed journal comparisons, exact replay after server restart, new revoked startup refusing original/new mutations without submission, independent authorized recovery, no unauthorized receipts and original joins/drains. Actual held HTTP commit/cancellation preserves original uncertainty, queued no-submission and fresh signed recovery without replay. Full SDK qualification remains separate.'}
            consultation_results = []
            for scenario, name, helpers in (
                ('restored', 'installed_python_consultation_and_return_preserve_actual_private_and_restored_audio', 3),
                ('remote-ended', 'installed_python_consultation_return_after_remote_eof_cannot_claim_a_continuing_call', 2),
            ):
                case = 'canonical_native_connections::consultation::sdk_controls::node_controls::' + name
                text = sdk_case('installed-python-consultation-' + scenario, case, {'CONNECT_GATE6_PYTHON_CONSULT':'1'})
                if text is None:
                    consultation_results.append({'scenario': scenario, 'passed': False})
                    continue
                assert re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', text) == [('1', '0', '0')]
                witnesses = [json.loads(line.split('installed-python-consultation-witness ', 1)[1])
                             for line in text.splitlines() if 'installed-python-consultation-witness ' in line]
                assert len(witnesses) == 2 and {w['algorithm'] for w in witnesses} == {'EdDSA', 'ES256'}
                for witness in witnesses:
                    assert witness['profile'] == 'python-consultation/1'
                    assert witness['http_workers_joined'] == 2
                    assert witness['audio_case'] == scenario and witness['current_return_authority']
                    assert witness['original_record_recovered'] and witness['helpers_joined'] == helpers and witness['server_joined']
                    assert witness['physical_peers_drained'] and witness['media_readers_drained']
                consultation_results.append({'scenario': scenario, 'passed': True,
                    'helpers_joined': sum(w['helpers_joined'] for w in witnesses), 'servers_joined': len(witnesses)})
            report['installed_python_consultation_audio'] = {'passed': all(v['passed'] for v in consultation_results),
                'results': consultation_results, 'algorithms': ['EdDSA', 'ES256'],
                'whole_function_deadline_seconds': 90, 'request_budget_ms': 4000, 'phase_deadline_seconds': 15,
                'scenarios': ['restored', 'remote-ended'], 'required_original_helpers_joined': 10, 'required_servers_joined': 4,
                'verified_original_helpers_joined': sum(v.get('helpers_joined', 0) for v in consultation_results),
                'verified_servers_joined': sum(v.get('servers_joined', 0) for v in consultation_results),
                'scope': 'Public installed Python controls with actual native private/restored audio and remote EOF; no model/provider/remote lane or full SDK qualification.'}
            report['scope'] += ' Also installed Python private/restored decoded audio and remote-EOF controls.'
            movements=[]
            for algorithm,label in (('EdDSA','eddsa'),('ES256','es256')):
                for suffix in ('session_handoff_observe','move_speaker'):
                    name=f'installed_python_{label}_{suffix}'
                    case='canonical_native_connections::consultation::groups::sdk_movements::'+name
                    text=sdk_case('installed-python-movement-'+name,case,environment={'CONNECT_GATE6_PYTHON_MOVEMENTS':'1'})
                    if text is None:
                        movements.append({'scenario':name,'algorithm':algorithm,'passed':False})
                        continue
                    assert re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;',text)==[('1','0','0')]
                    witnesses=[json.loads(line.split('installed-python-movement-witness ',1)[1]) for line in text.splitlines() if 'installed-python-movement-witness ' in line]
                    assert len(witnesses)==1
                    witness=witnesses[0]
                    assert witness['profile']=='python-movements/1' and witness['scenario']==name and witness['algorithms']==[algorithm]
                    assert (witness['cases'],witness['helpers_joined'],witness['servers_joined'])==(1,2,1)
                    assert witness['physical_peers_drained'] and witness['media_readers_drained']
                    movements.append(dict(witness,passed=True))
            report['installed_python_movements']={'passed':all(v['passed'] for v in movements),'profile':'python-movements/1',
                'results':movements,'request_budget_ms':4000,'phase_deadline_seconds':15,'whole_function_deadline_seconds':90,
                'scope':'Installed wheel/default PATH helper public handoff and Endpoint device replacement; independent actual native media, original-reference recovery and process/server joins. No programmatic Python media, named remote/provider/model or full SDK qualification.'}
        report['sdk_failures'] = sdk_failures
        assert not sdk_failures, 'Required installed SDK behavior failed; all independent scenes retained'
        report["behavior_passed"] = True
    finally:
        report["source_inputs_unchanged"] = sources() == before
        report["passed"] = report.get("behavior_passed", False) and report["source_inputs_unchanged"]
        output("source_unchanged", str(report["source_inputs_unchanged"]).lower())
        report["finished_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        (EVIDENCE / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    assert report["passed"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preflight", action="store_true")
    parser.add_argument("--node-sdk", action="store_true", help="Include installed synthetic caller/peer Node native control journey")
    parser.add_argument("--python-sdk", action="store_true", help="Include installed Python native restart/current-authority journey")
    args = parser.parse_args()
    preflight() if args.preflight else run(args.node_sdk, args.python_sdk)


if __name__ == "__main__":
    main()
