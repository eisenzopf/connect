#!/usr/bin/env python3
"""Owned Linux x86 host/browser and optional TURN namespaces on one internal bridge."""
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
    parser.add_argument('--relay-transport',choices=['udp','tcp'])
    parser.add_argument('--turn-image')
    args = parser.parse_args()
    assert platform.system() == 'Linux' and platform.machine() == 'x86_64'
    assert os.getuid() != 0, 'Run the owned fixture as the non-root host operator'
    assert bool(args.relay_transport)==bool(args.turn_image)
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
        relay_transport=args.relay_transport,
        binary_sha256=digest(binary), source_count=len(before), source_manifest_sha256=digest(args.source_manifest),
        tool_inputs_sha256=digest(output/'tool-inputs.json'))
    base = ROOT/'.local/gate-6/browser-remote'
    base.mkdir(parents=True, mode=0o700, exist_ok=True)
    fixture = base/('b-'+uuid.uuid4().hex[:8])
    fixture.mkdir(mode=0o700)
    (fixture/'private-tmp').mkdir(mode=0o700)
    name = 'connect-g6-'+uuid.uuid4().hex[:12]
    network = None
    server = browser = turn = None
    forced = False
    stage = 'browser-discovery'
    cidfile = fixture/'container-id'
    try:
        chrome = command(['docker','run','--rm','--read-only','--network','none',
            '--env','PYTHONDONTWRITEBYTECODE=1',args.image,'python3','-c',
            'import glob; p=glob.glob("/ms-playwright/chromium-[0-9]*/chrome-linux64/chrome")+glob.glob("/ms-playwright/chromium-[0-9]*/chrome-linux/chrome"); assert len(p)==1; print(p[0])'])
        assert re.fullmatch(r'/ms-playwright/chromium-[0-9]+/(?:chrome-linux64|chrome-linux)/chrome',chrome)
        stage = 'bridge-create'
        network = command(['docker', 'network', 'create', '--internal', '--driver', 'bridge', name])
        info = json.loads(command(['docker', 'network', 'inspect', network]))[0]
        assert info['Id'] == network and info['Internal'] and info['Driver'] == 'bridge' and not info['Containers']
        ip = info['IPAM']['Config'][0]['Gateway']
        assert isinstance(ipaddress.ip_address(ip), ipaddress.IPv4Address) and ipaddress.ip_address(ip).is_private
        port = media_port(ip)
        if args.relay_transport:
            assert re.fullmatch(r'sha256:[0-9a-f]{64}',args.turn_image)
            turn_image=json.loads(command(['docker','image','inspect',args.turn_image]))[0]
            assert turn_image['Id']==args.turn_image and turn_image['Architecture']=='amd64' and turn_image['Os']=='linux'
            stage='owned-turn-start'
            Turn=runpy.run_path(str(ROOT/'scripts/lib/owned_bridge_turn.py'))['OwnedBridgeTurn']
            turn=Turn(network,args.turn_image,args.image,ip,fixture/'turn',args.relay_transport)
            turn.start()
        host_namespace = hashlib.sha256(os.readlink('/proc/self/ns/net').encode()).hexdigest()
        environment = dict(os.environ, CONNECT_G6_REMOTE_BROWSER_DIR=str(fixture),
            CONNECT_G6_REMOTE_BROWSER_ALGORITHM=args.algorithm, CONNECT_G6_REMOTE_BROWSER_MEDIA_PORT=str(port),
            CONNECT_G6_OWNED_BROWSER_BRIDGE='1', CONNECT_G6_OWNED_BROWSER_BRIDGE_IP=ip, TMPDIR=str(fixture/'private-tmp'))
        if turn:environment.update(turn.environment)
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
            docker = ['docker', 'run', '--rm', '--cidfile', str(cidfile), '--user',f'{os.getuid()}:{os.getgid()}',
                '--read-only', '--network', network,
                '--platform', 'linux/amd64', '--cpus', '4', '--memory', '6g', '--shm-size', '1g',
                '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges',
                '--tmpfs', '/tmp:rw,nosuid,nodev,mode=1777,size=1g',
                '--add-host', 'connect.fixture.internal:'+ip,
                '--mount', f'type=bind,source={ROOT},target={ROOT},readonly',
                '--mount', f'type=bind,source={fixture},target={fixture}',
                '--mount', f'type=bind,source={fixture},target=/fixture', '--workdir', str(ROOT)]
            for key,value in dict(CONNECT_GATE6_OWNED_BROWSER_CONTAINER='1', CONNECT_PLAYWRIGHT_MODULE=str(module),
                PYTHONDONTWRITEBYTECODE='1', GIT_CONFIG_COUNT='1', GIT_CONFIG_KEY_0='safe.directory',
                GIT_CONFIG_VALUE_0=str(ROOT), TMPDIR='/tmp', CONNECT_GATE6_CHROME=chrome,
                CONNECT_GATE6_CANONICAL_BROWSER_ARTIFACTS=str(fixture)).items():
                docker += ['--env', key+'='+value]
            docker += [args.image,'python3','scripts/video/owned_pulse_audio.py','--output',str(fixture/'audio'),
                '--','node','scripts/video/owned_bridge_browser.mjs',str(fixture)]
            browser = subprocess.Popen(docker, cwd=ROOT, stdout=browser_log, stderr=subprocess.STDOUT, start_new_session=True)
            stage = 'browser-container'
            browser_exit = browser.wait(timeout=140)
            report['browser_exit_code'] = browser_exit
            # Host and container share the same non-root fixture owner.
            report['startup_browser_summary_present'] = (fixture/'browser-summary.json').is_file()
            report['startup_audio_summary_present'] = (fixture/'audio/summary.json').is_file()
            if not report['startup_browser_summary_present']:
                text = (fixture/'container.log').read_text(errors='replace').lower()
                report['startup_error_categories'] = [name for name,pattern in [
                    ('permission','permission denied'),('read-only-filesystem','read-only file system'),
                    ('audio-startup','owned audio startup failed'),('browser-version','current pinned cft required')]
                    if pattern in text]
                report['startup_source_sites'] = [dict(file=name,line=int(line))
                    for name,line in re.findall(r'File "[^"\n]*/(scripts/[A-Za-z0-9_/.-]+\.py)", line (\d+)',text)[-8:]]
            assert (fixture/'browser-summary.json').stat().st_uid == os.getuid()
            report['private_fixture_same_uid'] = True
            browser_summary = json.loads((fixture/'browser-summary.json').read_text())
            audio_summary = json.loads((fixture/'audio/summary.json').read_text())
            media = browser_summary.get('media')
            if media is not None:
                # Exclude diagnostic wire IDs, private state and raw content.
                browser_summary['media'] = dict(status=media.get('status'),phase=media.get('phase'),
                    error_class=media.get('error_class'),owned_container_bridge=media.get('owned_container_bridge'),
                    relay_path=media.get('relay_path'),
                    observations=[dict(decoded_fps=o['decoded']/o['elapsed_seconds'],
                        changing_fps=o['changing']/o['elapsed_seconds'],rendered_fps=o['rendered']/o['elapsed_seconds'],
                        motion_sample_pixels=o.get('motion_sample_pixels'),identity_sample_pixels=o.get('identity_sample_pixels'),
                        av_sync={key:o['av_sync'].get(key) for key in ['status','median_abs_skew_ms','p95_abs_skew_ms']},
                        audio=o['audio_profile']['status'],
                        motion=o['motion_profile']['status'],source_identity=o['source_identity']['status'],
                        av_acceptance=o['av_acceptance']) for o in media.get('observations',[])])
                if media.get('ice_restarts'):
                    restart=media['ice_restarts']
                    browser_summary['media']['ice_restarts']=dict(status=restart['status'],profile=restart['profile'],
                        results=[{key:row[key] for key in ['participant','ice_generation','stream_revision',
                            'connection_generation','discarded_answers','signed_requests','ownership','decoded_recovery']}
                            for row in restart['results']])
            report.update(browser=browser_summary, audio=audio_summary, browser_exit_code=browser_exit,
                different_network_namespaces=browser_summary['network_namespace_sha256'] != host_namespace,
                internal_bridge=True)
            (fixture/'done').touch()
            stage = 'server-retirement'
            server_exit = server.wait(timeout=20)
            report.update(server_exit_code=server_exit, original_server_joined=True,
                original_container_joined=browser.poll() is not None)
            text = (fixture/'server.log').read_text(errors='replace')
            report['server_panic_locations'] = [dict(file=name,line=int(line),column=int(column))
                for name,line,column in re.findall(
                    r'panicked at (crates/[A-Za-z0-9_/.-]+\.rs):(\d+):(\d+)',text)[:8]]
            witnesses = [json.loads(line.split('remote-browser-server-witness ',1)[1]) for line in text.splitlines()
                if 'remote-browser-server-witness ' in line]
            assert len(witnesses) == 1
            report['server_witness'] = witnesses[0]
            report['passed'] = browser_exit == server_exit == 0 and browser_summary['passed'] and audio_summary['passed']
            report['passed'] = report['passed'] and report['different_network_namespaces']
    except Exception as error:
        report['failure'] = dict(stage=stage,error_class=type(error).__name__ if type(error).__name__ in
            {'AssertionError','TimeoutExpired','FileNotFoundError','CalledProcessError','ValueError','KeyError'} else 'other',
            owned_startup_reason=getattr(error,'owned_docker_failure',None))
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
            if turn:
                try:
                    report['turn']=turn.close()
                    report['passed']=report['passed'] and report['turn']['passed'] and report['turn']['allocation_successes']>=4
                except Exception as error:
                    report['turn_cleanup_failure']=type(error).__name__;report['passed']=False
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
