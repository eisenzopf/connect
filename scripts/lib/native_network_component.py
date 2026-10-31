"""Rehearse current public SDK PCMU across one owned native Linux bridge.

The caller owns source verification and compilation; this keeps all credentials,
raw logs, synthetic audio and executable paths in ignored fixture directories.
"""
import contextlib
import hashlib
import json
import math
import re
from pathlib import Path
import runpy
import sys
import uuid

IMAGE = 'mcr.microsoft.com/playwright@sha256:d65108d1a8095d8b0c4a5e32bf1807a40a5ce90ff0c78473e1116b1c243235e8'


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def facts(value, names):
    """Only bounded numeric/boolean facts, digests and declared enum labels."""
    enums = {'algorithm': {'EdDSA', 'ES256'}, 'profile': {'remote'}, 'os': {'linux', 'macos'},
        'architecture': {'x86_64', 'aarch64'}, 'open_refused': {'timeout'},
        'mode': {'direct', 'loss', 'unreachable-port'}}
    result = {}
    for key in names:
        item = value.get(key)
        if key not in value:
            continue
        if key in enums:
            if isinstance(item, str) and item in enums[key]: result[key] = item
        elif key == 'source_revision':
            if isinstance(item, str) and re.fullmatch(r'[0-9a-f]{40}', item): result[key] = item
        elif key == 'image_id':
            if isinstance(item, str) and re.fullmatch(r'sha256:[0-9a-f]{64}', item): result[key] = item
        elif key.endswith('_sha256'):
            if isinstance(item, str) and re.fullmatch(r'[0-9a-f]{64}', item): result[key] = item
        elif key in ('guest_exit_code', 'server_exit_code'):
            if item is None or isinstance(item, int) and -128 <= item <= 255: result[key] = item
        elif item is None or isinstance(item, bool) or isinstance(item, (int, float)) and math.isfinite(item) and 0 <= item <= 10**12:
            result[key] = item
    return result


def public_result(value):
    """Copy finite fixture facts, never raw errors/configuration or paths."""
    result = facts(value, ('passed', 'mode', 'source_revision', 'server_executable_sha256',
        'guest_executable_sha256', 'image_id', 'runner_sha256', 'drop_every',
        'source_inputs_unchanged', 'build_inputs_unchanged', 'executables_unchanged'))
    result['owned_network'] = facts(value.get('owned_network', {}),
        ('requested', 'internal_bridge', 'original_bridge_removed', 'remaining_containers'))
    result['profiles'] = []
    for profile in value.get('profiles', []):
        row = facts(profile, ('algorithm', 'passed', 'guest_exit_code', 'server_exit_code',
            'elapsed_seconds', 'guest_log_sha256', 'server_log_sha256', 'observed_unanswered_datagrams'))
        for key in ('guest_witness', 'server_witness'):
            row[key] = [facts(witness, ('algorithm', 'profile', 'os', 'architecture', 'open_refused',
                'no_offer_submitted', 'elapsed_seconds', 'sent_frames', 'received_bytes',
                'owned_peer_shutdown', 'durable_native_offers', 'physical_peers_drained',
                'guest_open_refused', 'post_timeout_signed_heartbeat')) for witness in profile.get(key, [])]
        for key in ('decoded_at_host', 'decoded_at_guest'):
            if key in profile:
                row[key] = facts(profile[key], ('expected_hz', 'excluded_self_hz', 'decoded_samples',
                    'active_frames', 'matching_frames', 'pcmu_sha256', 'decoded_s16le_sha256', 'passed'))
        if 'transport_fault' in profile:
            fault = profile['transport_fault']
            row['transport_fault'] = facts(fault, ('drop_every_after_media_go', 'foreign_datagrams', 'worker_joined'))
            for direction in ('guest_to_server', 'server_to_guest'):
                row['transport_fault'][direction] = facts(fault.get(direction, {}), ('eligible', 'dropped', 'forwarded'))
        result['profiles'].append(row)
    classes = {'AssertionError', 'RuntimeError', 'CalledProcessError', 'TimeoutExpired',
        'OSError', 'ValueError', 'FileNotFoundError'}
    if value.get('failure'):
        kind = value['failure'].get('type')
        result['failure'] = {'type': kind if kind in classes else 'other'}
    if value.get('network_cleanup_error'):
        kind = value['network_cleanup_error']
        result['network_cleanup_error'] = kind if kind in classes else 'other'
    return result


def run(root, private, server, probe, inputs, execute):
    root, private = Path(root), Path(private)
    assert private.resolve().is_relative_to(root / '.local')
    assert all(Path(path).is_file() for path in (server, probe))
    execute('native-network-image-pull', ['docker', 'pull', IMAGE], 600)
    text = execute('native-network-image', ['docker', 'image', 'inspect', IMAGE], 15)
    image = json.loads(text)[0]
    assert image['Architecture'] == 'amd64' and image['Os'] == 'linux'
    metadata = private / 'native-network-build'
    metadata.mkdir(mode=0o700)
    source_file = metadata / 'source-inputs.json'
    source_file.write_text(json.dumps({name: value['sha256'] for name, value in inputs.items()}))
    server_file, guest_file = metadata / 'server.json', metadata / 'guest.json'
    server_file.write_text(json.dumps({'exit_code': 0, 'changed_inputs': [],
        'artifact': {'sealed': str(server), 'sha256': digest(server)}}))
    guest_file.write_text(json.dumps({'passed': True, 'executable': str(probe),
        'executable_sha256': digest(probe), 'image_id': image['Id'],
        'source_inputs_sha256': json.loads(source_file.read_text())}))
    harness = runpy.run_path(str(root / 'scripts/rehearse-gate6-native-media.py'))['main']
    parent = root / '.local/gate-6/native-remote'
    parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    results = []
    for mode, extra in [('direct', []), ('loss', ['--drop-every', '10']),
                        ('unreachable-port', ['--unreachable-port'])]:
        output = parent / ('linux-ci-' + mode + '-' + uuid.uuid4().hex)
        previous = sys.argv
        try:
            sys.argv = ['native-network', '--server-build', str(server_file), '--server-inputs', str(source_file),
                '--guest-build', str(guest_file), '--image', image['Id'], '--output', str(output),
                '--owned-linux-bridge', *extra]
            # Run in this owner, rather than detach a nested fixture supervisor
            # whose original children would outlive a killed wrapper process.
            with (private / ('native-network-' + mode + '.log')).open('w') as stream, contextlib.redirect_stdout(stream):
                code = harness()
            report = json.loads((output / 'summary.json').read_text())
            result = public_result(report)
            result['passed'] = code == 0 and report['passed']
            assert report['owned_network']['requested']
            assert all(row['algorithm'] in ('EdDSA', 'ES256') for row in report['profiles'])
        finally:
            sys.argv = previous
        results.append(result)
    return dict(scope='Current native Linux x86_64 host/client public SDK PCMU direct, bounded encrypted UDP loss and unreachable-port components; no WAN, video, full matrix, performance or acceptance qualification.',
        passed=len(results) == 3 and all(result['passed'] for result in results),
        results=results, image_id=image['Id'], full_task_qualified=False, acceptance_qualified=False)
