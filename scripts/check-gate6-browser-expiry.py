#!/usr/bin/env python3
"""Original expired-TURN browser tests in fresh network-disabled Linux namespaces."""
import argparse
import base64
import hashlib
import hmac
import json
import os
from pathlib import Path
import platform
import re
import runpy
import secrets
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
INPUTS = runpy.run_path(str(ROOT / 'scripts/check-gate6-ice-restart.py'))['inputs']
OWNED = runpy.run_path(str(ROOT / 'scripts/lib/owned_fixture_container.py'))
command, Container = OWNED['command'], OWNED['OwnedContainer']
TURN_REFERENCE = 'coturn/coturn@sha256:59af4221a2c45c1195a1a4a865542b3ea744be1dd8e228d11bde87c6631081cf'

HOLDER_CODE = """import os,signal,sys
def reap(*_):
 while True:
  try:
   if os.waitpid(-1,os.WNOHANG)[0]==0:break
  except ChildProcessError:break
signal.signal(signal.SIGCHLD,reap)
signal.signal(signal.SIGTERM,lambda *_:sys.exit(0))
print('owned-namespace-ready',flush=True)
while True:signal.pause()
"""



def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def private(path, text):
    with open(path, 'x', opener=lambda name, flags: os.open(name, flags, 0o600)) as stream:
        stream.write(text)


def tools():
    return {str(path.relative_to(ROOT)): digest(path) for folder in ('playwright', 'playwright-core')
        for path in (ROOT / 'scripts/video/observer-tools/node_modules' / folder).rglob('*')
        if path.is_file() and path.suffix in ('.js', '.mjs', '.json')}


def repository_inputs():
    result = INPUTS()
    for path in (ROOT / 'examples/conversation-client').rglob('*'):
        if path.is_file():
            result[str(path.relative_to(ROOT))] = digest(path)
    return result


def fixture(directory, transport):
    shared = secrets.token_hex(32)
    def credentials(expiry):
        user = f'{expiry}:synthetic-g6-expiry'
        return user, base64.b64encode(hmac.new(shared.encode(), user.encode(), hashlib.sha1).digest()).decode()
    user, password = credentials(int(time.time()) + 110)
    expired, expired_password = credentials(int(time.time()) - 60)
    private(directory / 'turnserver.conf', '\n'.join([
        'listening-ip=127.0.0.1', 'relay-ip=127.0.0.1', 'listening-port=3478',
        'min-port=32000', 'max-port=32015', 'relay-threads=1', 'realm=gate6.invalid',
        'use-auth-secret', f'static-auth-secret={shared}', 'allow-loopback-peers',
        'allowed-peer-ip=127.0.0.1', 'denied-peer-ip=0.0.0.0-255.255.255.255',
        'no-multicast-peers', 'no-tls', 'no-dtls', 'no-tcp-relay',
        'no-tcp' if transport == 'udp' else 'no-udp', 'fingerprint', 'verbose',
        'user-quota=8', 'total-quota=8', 'drain-min-allocations=0',
        f'userdb={directory}/turn.sqlite', f'pidfile={directory}/turn.pid',
        'log-file=stdout', 'simple-log', 'new-log-timestamp=false', '']))
    private(directory / 'fixture.json', json.dumps(dict(listener_port=3478, media_port_start=24000,
        media_port_end=24007, all_policy=False, transport=transport)))
    private(directory / 'references.toml', 'version = 1\n' + ''.join(
        f'\n[references.{name}]\nstorage = "environment"\nvariable = "{name}"\n'
        for name in ('CONNECT_GATE6_TURN_USER', 'CONNECT_GATE6_TURN_PASSWORD')))
    env = dict(CONNECT_GATE6_TURN_FIXTURE=str(directory / 'fixture.json'),
        CONNECT_SECRET_REFERENCES=str(directory / 'references.toml'),
        CONNECT_GATE6_TURN_USER=user, CONNECT_GATE6_TURN_PASSWORD=password,
        CONNECT_GATE6_TURN_EXPIRED_USER=expired, CONNECT_GATE6_TURN_EXPIRED_PASSWORD=expired_password,
        CONNECT_GATE6_CANONICAL_BROWSER_ARTIFACTS=str(directory / 'browser'),
        CONNECT_GATE6_OWNED_BROWSER_CONTAINER='1', TMPDIR=str(directory / 'tmp'),
        CONNECT_PLAYWRIGHT_MODULE=str(ROOT / 'scripts/video/observer-tools/node_modules/playwright/index.mjs'),
        GIT_CONFIG_COUNT='1', GIT_CONFIG_KEY_0='safe.directory', GIT_CONFIG_VALUE_0=str(ROOT),
        PYTHONDONTWRITEBYTECODE='1')
    private(directory / 'environment', ''.join(key + '=' + value + '\n' for key, value in env.items()))
    (directory / 'browser').mkdir(mode=0o700)
    (directory / 'tmp').mkdir(mode=0o700)
    return expired


def expiry_facts(directory, turn_log, expired):
    browser = json.loads((directory / 'browser/browser-result.jsonl').read_text())['expired_relay']
    oracle = json.loads((directory / 'browser/expired-relay-oracle.json').read_text())
    pulse = json.loads((directory / 'audio/summary.json').read_text())
    assert browser['status'] == 'passed' and browser['reason'] == 'expired'
    assert browser['original_contexts_closed'] == 2 and len(browser['results']) == 2
    results = []
    for index, row in enumerate(browser['results']):
        assert row['participant'] == index and row['error_code'] in ('unsupported-media-profile', 'timeout')
        assert 0 <= row['seconds'] <= 9 and row['limit_seconds'] == 8
        assert row['logical_offers'] == 0 and row['submission_attempted'] is False
        assert row['owned_peer_closed'] is True and row['owned_audio_tracks_ended'] is True
        assert row['remote_description_set'] is False
        results.append({key: row[key] for key in ('participant', 'error_code', 'seconds', 'limit_seconds',
            'logical_offers', 'submission_attempted', 'owned_peer_closed', 'owned_audio_tracks_ended', 'remote_description_set')})
    assert oracle == dict(passed=True, logical_operations=0, physical_allocations=0)
    assert pulse['passed'] and pulse['original_process_joined'] and pulse['exit_code'] == 0 and not pulse.get('forced_kill')
    unauthorized = len(re.findall(r'incoming packet message processed, error 401: Unauthorized', turn_log))
    refusals = turn_log.count(f'credentials of user <{expired}> have expired')
    allocations = len(re.findall(r'ALLOCATE processed, success', turn_log))
    assert unauthorized > 0 and refusals > 0 and allocations == 0
    return dict(browser=dict(status='passed', reason='expired', original_contexts_closed=2, results=results),
        oracle=oracle, audio=dict(passed=True, original_process_joined=True, exit_code=0,
        binary_sha256=pulse['binary_sha256'], version=pulse['version']),
        unauthorized_responses=unauthorized, expired_credential_refusals=refusals, allocation_successes=allocations)


def run_case(binary, image, turn_image, directory, transport, algorithm):
    directory.mkdir(mode=0o700)
    expired = fixture(directory, transport)
    (directory / 'namespace').mkdir(mode=0o700)
    (directory / 'turn').mkdir(mode=0o700)
    holder = Container(directory / 'namespace', image, 'none')
    turn = None
    report = dict(transport=transport, algorithm=algorithm, passed=False,
        test='canonical_webrtc_browser::canonical_chrome_relay_expired_credentials_' + algorithm.lower())
    try:
        holder.create(['--cpus', '4', '--memory', '6g', '--pids-limit', '512', '--shm-size', '1g',
            '--tmpfs', '/tmp:rw,nosuid,nodev,mode=1777,size=1g', '--workdir', str(ROOT),
            '--mount', f'type=bind,source={ROOT},target={ROOT},readonly',
            '--mount', f'type=bind,source={directory},target={directory}'], 'python3',
            ['-c', HOLDER_CODE])
        until = time.monotonic() + 5
        while 'owned-namespace-ready' not in command(['docker', 'logs', holder.cid]):
            assert holder.inspect()['State']['Running'] and time.monotonic() < until
            time.sleep(.025)
        turn = Container(directory / 'turn', turn_image, 'container:' + holder.cid)
        turn.create(['--cap-add', 'NET_BIND_SERVICE', '--cpus', '1', '--memory', '256m', '--pids-limit', '64',
            '--tmpfs', '/tmp:rw,nosuid,nodev,mode=1777,size=16m',
            '--mount', f'type=bind,source={directory},target={directory}', '--workdir', str(directory)],
            '/usr/bin/turnserver', ['-c', str(directory / 'turnserver.conf')])
        # Exercise the selected real listener and match the original transaction.
        ready = """import socket,struct,secrets,time,sys
kind=sys.argv[1];transaction=secrets.token_bytes(12);request=struct.pack('!HHI12s',1,0,0x2112A442,transaction)
until=time.monotonic()+5
while True:
 try:
  with socket.socket(socket.AF_INET,socket.SOCK_DGRAM if kind=='udp' else socket.SOCK_STREAM) as s:
   s.settimeout(.5)
   if kind=='udp':s.sendto(request,('127.0.0.1',3478));response,source=s.recvfrom(2048);assert source==('127.0.0.1',3478)
   else:
    s.connect(('127.0.0.1',3478));s.sendall(request);response=b''
    while len(response)<20:
     part=s.recv(20-len(response))
     if not part:raise OSError('owned STUN stream ended')
     response+=part
   assert len(response)>=20 and response[:2]==b'\\x01\\x01' and response[8:20]==transaction
   break
 except (OSError,AssertionError):
  if time.monotonic()>=until:raise
  time.sleep(.025)
print('ready')"""
        assert command(['docker', 'exec', holder.cid, 'python3', '-c', ready, transport]) == 'ready'
        report['control_binding_ready'] = True
        chrome = command(['docker', 'exec', holder.cid, 'python3', '-c',
            'import glob; p=glob.glob("/ms-playwright/chromium-[0-9]*/chrome-linux64/chrome")+glob.glob("/ms-playwright/chromium-[0-9]*/chrome-linux/chrome"); assert len(p)==1; print(p[0])'])
        assert re.fullmatch(r'/ms-playwright/chromium-[0-9]+/(?:chrome-linux64|chrome-linux)/chrome', chrome)
        node = command(['docker', 'exec', holder.cid, 'which', 'node'])
        assert node.startswith('/') and '\n' not in node
        version = command(['docker', 'exec', holder.cid, chrome, '--version'])
        assert version == 'Google Chrome for Testing 156.0.8078.4'
        report['browser_version'] = version
        report['tool_sha256'] = {}
        for name, path in [('chrome', chrome), ('node', node), ('packages', '/usr/local/share/connect-browser-packages.tsv')]:
            sha = command(['docker', 'exec', holder.cid, 'sha256sum', path]).split()[0]
            assert re.fullmatch(r'[0-9a-f]{64}', sha)
            report['tool_sha256'][name] = sha
        args = ['docker', 'exec', '--env-file', str(directory / 'environment'),
            '--env', 'CONNECT_GATE6_CHROME=' + chrome, '--env', 'CONNECT_GATE6_NODE=' + node,
            holder.cid, 'python3', 'scripts/video/owned_pulse_audio.py', '--output', str(directory / 'audio'),
            '--', str(binary), report['test'], '--exact', '--ignored', '--nocapture', '--test-threads=1']
        with (directory / 'test.log').open('x') as log:
            result = subprocess.run(args, stdout=log, stderr=subprocess.STDOUT, timeout=120)
        report['test_exit_code'] = result.returncode
        text = (directory / 'test.log').read_text()
        assert result.returncode == 0 and re.findall(r'^test result: ok\. (\d+) passed; 0 failed;', text, re.M) == ['1']
        turn_log = command(['docker', 'logs', turn.cid])
        report.update(expiry_facts(directory, turn_log, expired))
        # No new fixture children may survive the original test/audio joins.
        assert len(command(['docker', 'top', holder.cid, '-eo', 'pid,comm']).splitlines()) == 2
        report['original_children_joined'] = True
        command(['docker', 'kill', '--signal', 'USR1', turn.cid])
        began = time.monotonic()
        while True:
            turn_log = command(['docker', 'logs', turn.cid])
            if 'Drain complete (0 allocations remaining, threshold 0)' in turn_log:
                break
            assert turn.inspect()['State']['Running'] and time.monotonic() - began < 10
            time.sleep(.05)
        report['zero_allocations_before_exit'] = True
        report['allocation_drain_seconds'] = time.monotonic() - began
        assert report['allocation_drain_seconds'] <= 10
        assert command(['docker', 'wait', turn.cid], timeout=10) == '0'
        private(directory / 'turnserver.log', turn_log)
        report['turn_log_sha256'] = digest(directory / 'turnserver.log')
        report['behavior_passed'] = True
    except Exception as error:
        report['failure'] = dict(type=type(error).__name__ if isinstance(error,
            (AssertionError, subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError, ValueError, KeyError)) else 'other')
    finally:
        if turn:
            try:
                turn.inspect()
                log_path = directory / 'turnserver.log'
                if not log_path.exists():
                    private(log_path, command(['docker', 'logs', turn.cid]))
                    report['turn_log_sha256'] = digest(log_path)
            except Exception:
                report['private_turn_log_unavailable'] = True
            report['turn_cleanup'] = turn.remove()
        report['namespace_cleanup'] = holder.remove(original_exit=report.get('original_children_joined', False))
        report['passed'] = bool(report.get('behavior_passed') and all(
            row.get('original_container_joined') and row.get('original_exit_code') == 0
            and row.get('owned_container_removed') and not row.get('forced_cleanup')
            for row in (report.get('turn_cleanup', {}), report['namespace_cleanup'])))
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image', required=True)
    parser.add_argument('--turn-image', required=True)
    args = parser.parse_args()
    assert platform.system() == 'Linux' and platform.machine() == 'x86_64' and os.getuid() != 0
    base = ROOT / '.local/gate6-browser-media/evidence'
    output = base / 'expired-turn'
    output.mkdir(mode=0o700, exist_ok=False)
    producer = json.loads((base / 'build/summary.json').read_text())
    assert producer['passed'] and producer['runtime_library_profiles']['passed']
    before = INPUTS()
    assert before == json.loads((base / 'build/source-inputs.json').read_text())
    repository = repository_inputs()
    (output / 'repository-inputs.json').write_text(json.dumps(repository, indent=2) + '\n')
    binary = Path(producer['artifacts'][0]['path'])
    assert digest(binary) == producer['artifacts'][0]['sha256']
    for image in (args.image, args.turn_image):
        assert re.fullmatch(r'sha256:[0-9a-f]{64}', image)
        info = json.loads(command(['docker', 'image', 'inspect', image]))[0]
        assert info['Id'] == image and info['Os'] == 'linux' and info['Architecture'] == 'amd64'
    turn = json.loads(command(['docker', 'image', 'inspect', TURN_REFERENCE]))[0]
    assert turn['Id'] == args.turn_image
    prefix = ['docker', 'run', '--rm', '--network', 'none', '--read-only', '--cap-drop', 'ALL',
        '--cap-add', 'NET_BIND_SERVICE', '--security-opt', 'no-new-privileges', '--entrypoint']
    version = command([*prefix, '/usr/bin/turnserver', args.turn_image, '--version'])
    assert version == '4.18.0'
    turn_sha = command([*prefix, '/usr/bin/sha256sum', args.turn_image, '/usr/bin/turnserver']).split()[0]
    assert re.fullmatch(r'[0-9a-f]{64}', turn_sha)
    tool_inputs = tools()
    assert tool_inputs
    (output / 'tool-inputs.json').write_text(json.dumps(tool_inputs, indent=2) + '\n')
    private_base = ROOT / '.local/gate-6/turn-expiry-linux'
    private_base.mkdir(parents=True, mode=0o700, exist_ok=True)
    private_root = private_base / uuid.uuid4().hex
    private_root.mkdir(mode=0o700)
    cases = []
    for transport in ('udp', 'tcp'):
        for algorithm in ('EdDSA', 'ES256'):
            cases.append(run_case(binary, args.image, args.turn_image, private_root / (transport + '-' + algorithm), transport, algorithm))
    report = dict(scope=__doc__ + ' Original expiry refusal/zero logical offers and allocations only; no distinct-network media, provider, performance or full G6-054 qualification.',
        image_id=args.image, turn_image_id=args.turn_image, turn_reference=TURN_REFERENCE,
        turn_version=version, turn_binary_sha256=turn_sha, source_manifest_sha256=digest(base / 'build/source-inputs.json'),
        binary_sha256=producer['artifacts'][0]['sha256'], tool_inputs_sha256=digest(output / 'tool-inputs.json'),
        sources_unchanged=repository_inputs() == repository, tools_unchanged=tools() == tool_inputs,
        source_count=len(repository), repository_inputs_sha256=digest(output / 'repository-inputs.json'),
        binary_unchanged=digest(binary) == producer['artifacts'][0]['sha256'], cases=cases,
        full_task_qualified=False, acceptance_qualified=False)
    report['passed'] = all(row['passed'] for row in cases) and all(report[key]
        for key in ('sources_unchanged', 'tools_unchanged', 'binary_unchanged'))
    (output / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
    assert report['passed'], 'Required expired TURN tests failed or were unrun'


if __name__ == '__main__':
    main()
