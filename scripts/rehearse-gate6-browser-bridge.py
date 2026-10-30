#!/usr/bin/env python3
"""Owned Linux x86 host/browser namespaces on an internal Docker bridge only."""
from pathlib import Path
import argparse
import hashlib
import ipaddress
import json
import os
import platform
import re
import runpy
import signal
import socket
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
CASE = 'canonical_webrtc_browser::remote::distinct_network_browser_server_child'
INPUTS = runpy.run_path(str(ROOT/'scripts/check-gate6-ice-restart.py'))['inputs']


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def command(args):
    return subprocess.check_output(args, text=True).strip()


def media_port(ip):
    for _ in range(64):
        start = 20000 + int.from_bytes(os.urandom(2), 'big') % 39985
        held = []
        try:
            for port in range(start, start+16):
                sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                held.append(sock)
                sock.bind((ip, port))
            return start
        except OSError:
            pass
        finally:
            for sock in held:
                sock.close()
    raise RuntimeError('No owned media range available')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--producer', type=Path, required=True)
    parser.add_argument('--source-manifest', type=Path, required=True)
    parser.add_argument('--image', required=True)
    parser.add_argument('--algorithm', choices=['EdDSA', 'ES256'], required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    assert platform.system() == 'Linux' and platform.machine() == 'x86_64'
    assert re.search(r'^vendor_id\s*:', Path('/proc/cpuinfo').read_text(), re.M)
    assert re.fullmatch(r'sha256:[0-9a-f]{64}', args.image)
    image = json.loads(command(['docker', 'image', 'inspect', args.image]))[0]
    assert image['Id'] == args.image and image['Architecture'] == 'amd64' and image['Os'] == 'linux'
    output = args.output.resolve()
    assert output.is_relative_to(ROOT/'.local')
    output.mkdir(parents=True, mode=0o700, exist_ok=False)
    producer = json.loads(args.producer.read_text())
    assert producer['passed'] and producer['runtime_library_profiles']['passed']
    binary = Path(producer['artifacts'][0]['path'])
    assert digest(binary) == producer['artifacts'][0]['sha256']
    before = INPUTS()
    assert before == json.loads(args.source_manifest.read_text())
    tools = {str(p.relative_to(ROOT)):digest(p)
        for folder in ['playwright','playwright-core']
        for p in (ROOT/'scripts/video/observer-tools/node_modules'/folder).rglob('*')
        if p.is_file() and p.suffix in ('.js','.mjs','.json')}
    assert tools
    (output/'tool-inputs.json').write_text(json.dumps(tools, indent=2)+'\n')
    report = dict(scope=__doc__, algorithm=args.algorithm, passed=False, image_id=args.image,
        binary_sha256=digest(binary), source_count=len(before), source_manifest_sha256=digest(args.source_manifest),
        tool_inputs_sha256=digest(output/'tool-inputs.json'))
    base = ROOT/'.local/gate-6/browser-remote'
    base.mkdir(parents=True, mode=0o700, exist_ok=True)
    fixture = base/('b-'+uuid.uuid4().hex[:8])
    fixture.mkdir(mode=0o700)
    (fixture/'private-tmp').mkdir(mode=0o700)
    name = 'connect-g6-'+uuid.uuid4().hex[:12]
    network = None
    server = browser = None
    forced = False
    stage = 'bridge-create'
    cidfile = fixture/'container-id'
    try:
        network = command(['docker', 'network', 'create', '--internal', '--driver', 'bridge', name])
        info = json.loads(command(['docker', 'network', 'inspect', network]))[0]
        assert info['Id'] == network and info['Internal'] and info['Driver'] == 'bridge' and not info['Containers']
        ip = info['IPAM']['Config'][0]['Gateway']
        assert isinstance(ipaddress.ip_address(ip), ipaddress.IPv4Address) and ipaddress.ip_address(ip).is_private
        port = media_port(ip)
        host_namespace = hashlib.sha256(os.readlink('/proc/self/ns/net').encode()).hexdigest()
        environment = dict(os.environ, CONNECT_G6_REMOTE_BROWSER_DIR=str(fixture),
            CONNECT_G6_REMOTE_BROWSER_ALGORITHM=args.algorithm, CONNECT_G6_REMOTE_BROWSER_MEDIA_PORT=str(port),
            CONNECT_G6_OWNED_BROWSER_BRIDGE='1', CONNECT_G6_OWNED_BROWSER_BRIDGE_IP=ip, TMPDIR=str(fixture/'private-tmp'))
        with (fixture/'server.log').open('w') as server_log, (fixture/'container.log').open('w') as browser_log:
            stage = 'server-readiness'
            server = subprocess.Popen([str(binary), CASE, '--exact', '--ignored', '--nocapture', '--test-threads=1'],
                cwd=ROOT, env=environment, stdout=server_log, stderr=subprocess.STDOUT, start_new_session=True)
            until = time.monotonic()+45
            while not (fixture/'ready.json').exists():
                assert server.poll() is None and time.monotonic() < until, 'Owned bridge server readiness failed'
                time.sleep(.025)
            ready = json.loads((fixture/'ready.json').read_text())
            assert ready['media_public_ip'] == ip and ready['media_port_start'] == port and ready['media_port_end'] == port+15
            module = ROOT/'scripts/video/observer-tools/node_modules/playwright/index.mjs'
            assert module.is_file()
            docker = ['docker', 'run', '--rm', '--cidfile', str(cidfile), '--read-only', '--network', network,
                '--platform', 'linux/amd64', '--cpus', '4', '--memory', '6g', '--shm-size', '1g',
                '--cap-drop', 'ALL', '--cap-add', 'DAC_OVERRIDE', '--security-opt', 'no-new-privileges',
                '--tmpfs', '/tmp:rw,nosuid,nodev,size=1g',
                '--add-host', 'connect.fixture.internal:'+ip,
                '--mount', f'type=bind,source={ROOT},target={ROOT},readonly',
                '--mount', f'type=bind,source={fixture},target={fixture}',
                '--mount', f'type=bind,source={fixture},target=/fixture', '--workdir', str(ROOT)]
            for key,value in dict(CONNECT_GATE6_OWNED_BROWSER_CONTAINER='1', CONNECT_PLAYWRIGHT_MODULE=str(module),
                PYTHONDONTWRITEBYTECODE='1', GIT_CONFIG_COUNT='1', GIT_CONFIG_KEY_0='safe.directory',
                GIT_CONFIG_VALUE_0=str(ROOT), TMPDIR='/tmp/connect-fixtures', CONNECT_G6_BRIDGE_OUTPUT=str(fixture)).items():
                docker += ['--env', key+'='+value]
            docker += [args.image, 'sh', '-eu', '-c',
                'mkdir -p "$TMPDIR"; CONNECT_GATE6_CHROME="$(python3 -c "import glob; p=glob.glob(\"/ms-playwright/chromium-*/chrome-linux64/chrome\"); assert len(p)==1; print(p[0])")"; export CONNECT_GATE6_CHROME; python3 scripts/video/owned_pulse_audio.py --output "$CONNECT_G6_BRIDGE_OUTPUT/audio" -- node scripts/video/owned_bridge_browser.mjs "$CONNECT_G6_BRIDGE_OUTPUT"']
            browser = subprocess.Popen(docker, cwd=ROOT, stdout=browser_log, stderr=subprocess.STDOUT, start_new_session=True)
            stage = 'browser-container'
            browser_exit = browser.wait(timeout=140)
            report['browser_exit_code'] = browser_exit
            # Root in the isolated browser image writes private0600/0700 files.
            # Return only this fresh fixture to its host owner before reading;
            # never relax modes or recursively follow browser profile symlinks.
            if os.getuid() != 0:
                subprocess.run(['sudo', '-n', 'chown', '--recursive', '--no-dereference', '--',
                    f'{os.getuid()}:{os.getgid()}', str(fixture)], check=True, timeout=30)
            report['startup_browser_summary_present'] = (fixture/'browser-summary.json').is_file()
            report['startup_audio_summary_present'] = (fixture/'audio/summary.json').is_file()
            if not report['startup_browser_summary_present']:
                text = (fixture/'container.log').read_text(errors='replace').lower()
                report['startup_error_categories'] = [name for name,pattern in [
                    ('permission','permission denied'),('read-only-filesystem','read-only file system'),
                    ('audio-startup','owned audio startup failed'),('browser-version','current pinned cft required')]
                    if pattern in text]
            assert (fixture/'browser-summary.json').stat().st_uid == os.getuid()
            report['private_fixture_ownership_normalized'] = True
            browser_summary = json.loads((fixture/'browser-summary.json').read_text())
            audio_summary = json.loads((fixture/'audio/summary.json').read_text())
            media = browser_summary.get('media')
            if media is not None:
                # Exclude diagnostic wire IDs, private state and raw content.
                browser_summary['media'] = dict(status=media.get('status'),phase=media.get('phase'),
                    error_class=media.get('error_class'),owned_container_bridge=media.get('owned_container_bridge'),
                    observations=[dict(decoded_fps=o['decoded']/o['elapsed_seconds'],
                        changing_fps=o['changing']/o['elapsed_seconds'],audio=o['audio_profile']['status'],
                        motion=o['motion_profile']['status'],source_identity=o['source_identity']['status'],
                        av_acceptance=o['av_acceptance']) for o in media.get('observations',[])])
            report.update(browser=browser_summary, audio=audio_summary, browser_exit_code=browser_exit,
                different_network_namespaces=browser_summary['network_namespace_sha256'] != host_namespace,
                internal_bridge=True)
            (fixture/'done').touch()
            stage = 'server-retirement'
            server_exit = server.wait(timeout=20)
            text = (fixture/'server.log').read_text(errors='replace')
            witnesses = [json.loads(line.split('remote-browser-server-witness ',1)[1]) for line in text.splitlines()
                if 'remote-browser-server-witness ' in line]
            assert len(witnesses) == 1
            report.update(server_exit_code=server_exit, server_witness=witnesses[0],
                original_server_joined=True, original_container_joined=browser.poll() is not None)
            report['passed'] = browser_exit == server_exit == 0 and browser_summary['passed'] and audio_summary['passed']
            report['passed'] = report['passed'] and report['different_network_namespaces']
    except Exception as error:
        report['failure'] = dict(stage=stage,error_class=type(error).__name__ if type(error).__name__ in
            {'AssertionError','TimeoutExpired','FileNotFoundError','CalledProcessError','ValueError','KeyError'} else 'other')
        raise
    finally:
        if browser is not None and browser.poll() is None:
            forced = True
            if cidfile.exists():
                cid = cidfile.read_text().strip()
                assert re.fullmatch(r'[0-9a-f]{64}', cid)
                subprocess.run(['docker', 'stop', '--time', '5', cid], capture_output=True, timeout=15)
            try:
                browser.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(browser.pid, signal.SIGKILL)
                browser.wait()
        if server is not None and server.poll() is None:
            (fixture/'done').touch()
            try:
                server.wait(timeout=20)
            except subprocess.TimeoutExpired:
                forced = True
                os.killpg(server.pid, signal.SIGTERM)
                try:
                    server.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(server.pid, signal.SIGKILL)
                    server.wait()
        if network is not None:
            info = json.loads(command(['docker', 'network', 'inspect', network]))[0]
            report['bridge_containers_drained'] = not info['Containers']
            removed = subprocess.run(['docker', 'network', 'rm', network], capture_output=True).returncode == 0
            report['owned_bridge_removed'] = removed
            report['passed'] = report['passed'] and not info['Containers'] and removed
        report['forced_cleanup'] = forced
        report['sources_unchanged'] = INPUTS() == before
        report['tools_unchanged'] = all(digest(ROOT/name) == value for name,value in tools.items()) and digest(binary) == producer['artifacts'][0]['sha256']
        report['tool_count'] = len(tools)
        report['passed'] = report['passed'] and not forced and report['sources_unchanged'] and report['tools_unchanged']
        (output/'summary.json').write_text(json.dumps(report, indent=2)+'\n')
    assert report['passed'], 'Owned internal bridge media did not qualify; private fixture retained'


if __name__ == '__main__':
    main()
