"""Current Linux SIP/RTP addressing on one exact owned internal bridge."""
import contextlib
import json
import os
from pathlib import Path
import re
import runpy
import subprocess
import sys
import uuid

ROOT = Path(__file__).resolve().parents[2]
NATIVE = runpy.run_path(str(ROOT / 'scripts/lib/native_network_component.py'))
RUNNER = runpy.run_path(str(ROOT / 'scripts/rehearse-gate6-native-media.py'))
IMAGE, digest, facts = NATIVE['IMAGE'], NATIVE['digest'], NATIVE['facts']


def command(args):
    return subprocess.check_output(args, text=True, stderr=subprocess.PIPE, timeout=20).strip()


class OwnedBridge:
    def __init__(self, private, invoke=command):
        self.private, self.invoke = Path(private), invoke
        self.name = 'connect-g6-phone-' + uuid.uuid4().hex[:12]
        self.label = uuid.uuid4().hex
        self.network = self.gateway = None
        self.started = False
        self.report = dict(requested=True, internal_bridge=True, original_bridge_removed=False,
            remaining_containers=None)
        self.save()

    def save(self):
        # Retain exact custody even if a Docker reply or later cleanup is lost.
        path = self.private / 'owned-telephone-network.json'
        value = dict(name=self.name, label=self.label, network=self.network, gateway=self.gateway,
            started=self.started, cleanup=self.report)
        with open(path, 'w', opener=lambda name, flags: os.open(name, flags, 0o600)) as stream:
            json.dump(value, stream)

    def inspect_owned(self, name):
        info = json.loads(self.invoke(['docker', 'network', 'inspect', name]))[0]
        assert info['Name'] == self.name and info['Labels'].get('connect.gate6.telephone-media') == self.label
        assert info['Internal'] is True and info['Driver'] == 'bridge'
        assert re.fullmatch(r'[0-9a-f]{64}', info['Id'])
        if self.network is not None:
            assert info['Id'] == self.network
        return info

    def create(self):
        self.started = True
        self.save()
        self.network = self.invoke(['docker', 'network', 'create', '--internal', '--driver', 'bridge',
            '--label', 'connect.gate6.telephone-media=' + self.label, self.name])
        assert re.fullmatch(r'[0-9a-f]{64}', self.network)
        info = self.inspect_owned(self.network)
        self.gateway = RUNNER['owned_gateway'](self.network, info)
        self.save()
        return self

    def close(self):
        try:
            if not self.started:
                self.report.update(requested=False, original_bridge_removed=True, remaining_containers=0)
            else:
                info = self.inspect_owned(self.network or self.name)
                self.network = info['Id']
                self.save()
                self.report['remaining_containers'] = len(info.get('Containers', {}))
                assert self.report['remaining_containers'] == 0
                assert self.invoke(['docker', 'network', 'rm', self.network]) == self.network
                self.report['original_bridge_removed'] = True
        except Exception as error:
            self.report['cleanup_error'] = type(error).__name__ if isinstance(error,
                (AssertionError, subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError, ValueError)) else 'other'
        self.save()
        return self.report


def public_result(value):
    result = facts(value, ('passed', 'source_revision', 'image_id', 'server_sha256',
        'source_inputs_unchanged', 'build_inputs_unchanged', 'server_unchanged'))
    result['profiles'] = []
    for profile in value.get('profiles', []):
        row = facts(profile, ('algorithm', 'passed', 'guest_exit_code', 'server_exit_code',
            'forced_server_cleanup', 'forced_guest_cleanup'))
        row['guest'] = facts(profile.get('guest', {}), ('passed', 'os', 'architecture', 'sip_invites',
            'sip_acks', 'sip_byes', 'sent_rtp', 'received_rtp', 'foreign_datagrams', 'rtcp_mux_offered',
            'server_advertised_media_ip_matched', 'unique_call_ids', 'owned_sockets_closed',
            'received_bytes', 'received_sha256'))
        row['server_witness'] = [facts(w, ('algorithm', 'durable_initial_calls', 'durable_native_offers',
            'original_invite_replayed', 'paired_session_end', 'physical_peers_drained', 'received_bytes',
            'server_shutdown_joined', 'sip_listener_released')) for w in profile.get('server_witness', [])]
        for key in ('host_decoded', 'guest_decoded'):
            row[key] = facts(profile.get(key, {}), ('passed', 'expected_hz', 'excluded_self_hz',
                'decoded_samples', 'active_frames', 'matching_frames', 'pcmu_sha256', 'decoded_s16le_sha256'))
        nat = profile.get('peer_nat', {})
        row['peer_nat'] = facts(nat, ('foreign_datagrams', 'journal_before_first_invite',
            'worker_joined', 'unique_calls', 'owned_rtcp_sink_packets'))
        for key in ('sip', 'rtp', 'rtcp', 'pcmu_rtp_seen', 'pcmu_rtp_dropped'):
            row['peer_nat'][key] = facts(nat.get(key, {}), ('guest_to_server', 'server_to_guest'))
        hashes = profile.get('logs_sha256', {})
        row['logs_sha256'] = {key: value for key, value in hashes.items() if key in ('server.log', 'guest.log')
            and isinstance(value, str) and re.fullmatch(r'[0-9a-f]{64}', value)}
        result['profiles'].append(row)
    if value.get('failure'):
        kind = value['failure'].get('type')
        result['failure'] = {'type': kind if kind in ('AssertionError', 'RuntimeError', 'OSError',
            'CalledProcessError', 'TimeoutExpired', 'ValueError', 'FileNotFoundError') else 'other'}
    return result


def run(root, private, server, inputs, execute):
    root, private = Path(root), Path(private)
    assert private.resolve().is_relative_to(root / '.local')
    execute('telephone-network-image-pull', ['docker', 'pull', IMAGE], 600)
    text = execute('telephone-network-image', ['docker', 'image', 'inspect', IMAGE], 15)
    image = json.loads(text)[0]
    assert image['Architecture'] == 'amd64' and image['Os'] == 'linux'
    version = execute('telephone-network-python', ['docker', 'run', '--rm', '--network', 'none', '--read-only',
        '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges', '--entrypoint', 'python3', image['Id'], '--version'], 30).strip()
    assert re.fullmatch(r'Python 3\.\d+\.\d+', version)
    metadata = private / 'telephone-network-build'
    metadata.mkdir(mode=0o700)
    source_file, server_file = metadata / 'source-inputs.json', metadata / 'server.json'
    source_file.write_text(json.dumps({name: value['sha256'] for name, value in inputs.items()}))
    server_file.write_text(json.dumps({'exit_code': 0, 'changed_inputs': [],
        'artifact': {'sealed': str(server), 'sha256': digest(server)}}))
    harness = runpy.run_path(str(root / 'scripts/rehearse-gate6-remote-telephone.py'))['main']
    parent = root / '.local/gate-6/telephone-remote'
    parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    owner = OwnedBridge(metadata)
    results = []
    try:
        owner.create()
        for mode in ('direct', 'loss'):
            output = parent / ('linux-ci-' + mode + '-' + uuid.uuid4().hex)
            previous = sys.argv
            try:
                sys.argv = ['telephone-network', '--server-build', str(server_file), '--server-inputs', str(source_file),
                    '--host-media-ip', owner.gateway, '--image', image['Id'], '--owned-linux-bridge', owner.network,
                    '--network-profile', mode, '--output', str(output)]
                code = 1
                with (private / ('telephone-network-' + mode + '.log')).open('w') as stream, contextlib.redirect_stdout(stream):
                    try:
                        harness()
                    except SystemExit as stopped:
                        code = stopped.code
                raw = json.loads((output / 'summary.json').read_text())
                result = public_result(raw)
                result['network_profile'] = mode
                result['passed'] = code == 0 and raw['passed']
                if result['passed']:
                    assert len(result['profiles']) == 2
                    for row in result['profiles']:
                        assert row['guest']['os'] == 'linux' and row['guest']['architecture'] == 'x86_64'
                        assert all(w['original_invite_replayed'] and w['paired_session_end']
                            and w['physical_peers_drained'] and w['server_shutdown_joined']
                            and w['sip_listener_released'] for w in row['server_witness'])
                results.append(result)
            finally:
                sys.argv = previous
    finally:
        network = owner.close()
    return dict(scope='Current native Linux x86_64 host and SIP/RTP peer on one owned internal bridge, controlled fixed peer-side NAT and public native SDK PCMU: original direct/20percentRTP-loss ×EdDSA/ES256 decode, journal-before-INVITE, replay, signed end and drains. No WAN/provider/PSTN/model, video, full matrix, performance or acceptance qualification.',
        passed=len(results) == 2 and all(result['passed'] for result in results) and network['original_bridge_removed'],
        results=results, image_id=image['Id'], guest_python=version, owned_network=network,
        full_task_qualified=False, acceptance_qualified=False)
