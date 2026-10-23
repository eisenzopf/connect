"""Disposable media fixtures and concurrent files through the public CLI only.

Every process uses the explicitly selected binary. No private store/Core access,
compilation or checkout discovery belongs to this fixture protocol boundary.
"""
import asyncio
import base64
import hashlib
import json
import math
import re
import time
import tomllib

from lib.local_process import Server, port, run


def native_request(state, who, operation, binary):
    result = json.loads(run('request', '--config', state / (who + '.toml'),
        '--envelope', value=operation, binary=binary).stdout)['result']
    if 'Ok' not in result: raise RuntimeError('Public media setup operation rejected: ' + operation['op'])
    return result['Ok']['data']


def prepare(work, binary):
    state = work / 'state'
    run('init', '--directory', state, '--example', 'review-loop', '--bind', f'127.0.0.1:{port()}', binary=binary)
    identities = {name: tomllib.loads((state / (name + '.toml')).read_text())['identity_id']
        for name in ('reviews', 'agent')}
    for name, peer in [('reviews', 'agent'), ('agent', 'reviews')]:
        run('issue', '--config', state / 'operator.toml', '--server-config', state / 'server.toml',
            '--identity-id', identities[name], '--recipients', identities[peer], '--voice',
            '--output', state / (name + '-av.toml'), binary=binary)
        (state / (name + '-av.toml')).replace(state / (name + '.toml'))
    with Server(state, binary=binary, log_path=work / 'setup-server.log'):
        conversation = native_request(state, 'reviews', {'op': 'create', 'request_id': 'video-create',
            'participants': [identities['agent']]}, binary)['id']
        session = native_request(state, 'reviews', {'op': 'session_invite', 'request_id': 'video-invite',
            'conversation_id': conversation, 'participant_id': identities['agent'], 'media': ['audio', 'video']}, binary)['id']
        native_request(state, 'agent', {'op': 'session_accept', 'request_id': 'video-accept',
            'conversation_id': conversation, 'session_id': session}, binary)
    configs = {name: tomllib.loads((state / (name + '.toml')).read_text()) for name in identities}
    signaling = f'127.0.0.1:{port()}'
    with (state / 'server.toml').open('a') as output:
        output.write('\n[webrtc]\nbind = ' + json.dumps(signaling) + '\n[webrtc.route]\n')
        for name, value in {'credential_id': configs['reviews']['credential_id'], 'conversation_id': conversation,
                'assistant_identity_id': identities['agent'], 'session_id': session}.items():
            output.write(name + ' = ' + json.dumps(value) + '\n')
    return state, identities, configs, conversation, session, 'ws://' + signaling


async def concurrent_file(state, conversation, binary, proxy=False, readiness=None):
    # One MiB over public reliable Connect operations while AV occupies the server.
    ready = await readiness if readiness is not None else None
    if readiness is None: await asyncio.sleep(1)
    content = b''.join(hashlib.sha256(f'connect-media-fixture-{index}'.encode()).digest()
        for index in range(32768))
    expected = hashlib.sha256(content).hexdigest(); began = time.monotonic(); began_unix = time.time()
    async def request(who, operation):
        selected = state / (who + ('-impaired' if proxy else '') + '.toml')
        child = await asyncio.create_subprocess_exec(str(binary), 'request', '--config', str(selected), '--envelope',
            stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
        try:
            out, err = await asyncio.wait_for(child.communicate(json.dumps(operation).encode()), 15)
            if child.returncode: raise RuntimeError('Concurrent reliable CLI request failed: ' + operation['op'])
            result = json.loads(out)['result']
            if 'Ok' not in result: raise RuntimeError('Concurrent reliable operation rejected: ' + operation['op'])
            return result['Ok']['data']
        finally:
            if child.returncode is None:
                child.terminate()
                try: await asyncio.wait_for(child.wait(), 1)
                except asyncio.TimeoutError: child.kill(); await child.wait()
    artifact = await request('reviews', {'op': 'artifact_begin', "to": "all", 'request_id': 'video-file-begin',
        'conversation_id': conversation, 'name': 'generated-media-qualification.bin',
        'media_type': 'application/octet-stream', 'size': len(content), 'sha256': expected})
    aid = artifact['id']; latencies = []
    for offset in range(0, len(content), 32768):
        start = time.monotonic()
        await request('reviews', {'op': 'artifact_append', 'request_id': f'video-file-chunk-{offset}',
            'conversation_id': conversation, 'artifact_id': aid, 'offset': offset,
            'data': base64.b64encode(content[offset:offset + 32768]).decode()})
        latencies.append(time.monotonic() - start)
        await asyncio.sleep(max(0, 32768 * 8 / 4_000_000 - latencies[-1]))
    finished = await request('reviews', {'op': 'artifact_finish', 'request_id': 'video-file-finish',
        'conversation_id': conversation, 'artifact_id': aid})
    if finished['state'] != 'ready': raise RuntimeError('Concurrent artifact not ready')
    received = bytearray()
    for offset in range(0, len(content), 32768):
        start = time.monotonic()
        chunk = await request('agent', {'op': 'artifact_get', 'conversation_id': conversation,
            'artifact_id': aid, 'offset': offset, 'limit': 32768})
        if chunk['offset'] != offset: raise RuntimeError('Concurrent artifact returned wrong offset')
        received.extend(base64.b64decode(chunk['data']))
        await asyncio.sleep(max(0, 32768 * 8 / 4_000_000 - (time.monotonic() - start)))
    if received != content or hashlib.sha256(received).hexdigest() != expected:
        raise RuntimeError('Concurrent reliable artifact hash mismatch')
    return {'status': 'passed', 'bytes': len(content), 'chunk_bytes': 32768, 'sha256': expected,
        'elapsed_seconds': time.monotonic() - began,
        'start_unix_seconds': began_unix, 'end_unix_seconds': time.time(),
        'readiness_observation': ready, 'maximum_append_seconds': max(latencies),
        'fixture_file_payload_pacing_bits_per_second': 4_000_000,
        'scope': 'public reliable CLI operation streams concurrent with AV; separate authenticated QUIC connections' + (' through the same counted impairment proxy' if proxy else '')}



def transfer_containment(transfer, windows):
    """A completed file outside the media window cannot prove concurrent load."""
    began = transfer.get('start_unix_seconds'); ended = transfer.get('end_unix_seconds')
    valid_time = all(isinstance(at, (int, float)) and math.isfinite(at) for at in [began, ended])
    rows = []
    for slot, (start, stop) in enumerate(windows):
        valid = valid_time and all(math.isfinite(at) for at in [start, stop]) and start < stop and began < ended
        contained = valid and start <= began <= ended <= stop
        overlap = max(0, min(ended, stop) - max(began, start)) if valid else None
        rows.append({'receiver_slot': slot + 1, 'window_start_unix_seconds': start,
            'window_end_unix_seconds': stop, 'overlap_seconds': overlap, 'fully_contained': contained})
    return {'status': 'passed' if rows and all(row['fully_contained'] for row in rows) else 'failed',
        'method': 'actual public artifact begin through verified complete download Unix boundaries must lie inside each independently observed clean receive window',
        'receivers': rows}
