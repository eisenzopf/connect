#!/usr/bin/env python3
"""Real verified QUIC conformance with independently implemented Python control."""
import argparse
import asyncio
import hashlib
import importlib.metadata
import importlib.util
import json
import os
import pathlib
import subprocess
import sys
import datetime
import tomllib
import traceback

from lib.local_process import BINARY, ROOT, Server, port, run

CLIENT_ROOT = ROOT / 'examples/independent-uctp-client'
from aioquic.quic.logger import QuicLogger


def native(state, operation, binary):
    return json.loads(run('request', '--config', state / 'agent.toml', '--envelope', value=operation, binary=binary).stdout)


async def expect_error(client, operation, code):
    try:
        await client.request(operation)
    except ApplicationError as error:
        assert error.code == code, (error.code, code)
    else:
        raise AssertionError('Operation accepted unexpectedly')


async def reject_bad_frame(config, data):
    async with open_client(config) as peer:
        peer.writer.write(data)
        await peer.writer.drain()
        # A framing failure terminates the server's reliable read/write pumps.
        # An idle timeout alone does not count as rejection.
        async with asyncio.timeout(3):
            try:
                await peer.receive()
            except asyncio.IncompleteReadError:
                return
            raise AssertionError('Malformed frame was not closed')


async def standalone_cli(config, expected_identity, log_path):
    """Exercise the shipped entrypoint as its own interpreter process."""
    with log_path.open('w') as log:
        process = await asyncio.create_subprocess_exec(
            sys.executable, str(CLIENT_ROOT / 'client.py'), '--config', str(config),
            stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE, stderr=log)
        try:
            async with asyncio.timeout(15):
                ready = json.loads(await process.stdout.readline())
                assert ready['ready']['alpn'] == 'uctp/1' and ready['ready']['tls_verified']
                process.stdin.write(json.dumps({'op': 'resolve', 'address': 'connect:agent@local'}).encode() + b'\n')
                await process.stdin.drain()
                outcome = json.loads(await process.stdout.readline())
                assert outcome['Ok']['data']['id'] == expected_identity
                process.stdin.close()
                assert await process.wait() == 0
        finally:
            if process.returncode is None:
                process.kill()
                await process.wait()


async def qualify(state, binary, observations=None):
    config = state / 'reviews.toml'
    native_config = tomllib.loads((state / 'agent.toml').read_text())
    observations = {} if observations is None else observations
    observations['stage'] = 'server-first-readiness'
    packet_trace = QuicLogger()
    with Server(state, binary=binary, log_path=state.parent / 'server-first.log'):
        observations['stage'] = 'standalone-client-entrypoint'
        await standalone_cli(config, native_config['identity_id'], state.parent / 'standalone-client.log')
        observations['standalone_client_process'] = 'authenticated verified QUIC and resolved native identity'
        observations['stage'] = 'tls-wrong-server-name'
        untrusted = state / 'wrong-server.toml'
        untrusted.write_text(config.read_text().replace('server_name = "localhost"', 'server_name = "untrusted.invalid"'))
        try:
            async with open_client(untrusted):
                raise AssertionError('Unverified server was accepted')
        except ConnectionError:
            pass
        observations['tls_wrong_server_name'] = 'rejected before authentication; no trust fallback'
        observations['stage'] = 'exchange-native-and-independent-controls'
        async with open_client(config, packet_trace) as independent:
            observations['transport'] = independent.transport
            identity = await independent.request({'op': 'resolve', 'address': 'connect:agent@local'})
            assert identity['kind'] == 'identity' and identity['data']['id'] == native_config['identity_id']
            conversation = await independent.request({'op': 'create', 'request_id': 'independent-create',
                                                        'participants': [identity['data']['id']]})
            cid = conversation['data']['id']
            send = {'op': 'send', 'request_id': 'independent-send', 'conversation_id': cid, 'body': 'independent hello'}
            receipt = await independent.request(send)
            assert receipt['kind'] == 'accepted'
            first_inbox = native(state, {'op': 'inbox', 'cursor': None, 'limit': 100, 'wait_ms': 0}, binary)['result']['Ok']['data']
            message_events = [e for e in first_inbox['events'] if e.get('message') and e['message']['id'] == receipt['data']['message_id']]
            assert len(message_events) == 1
            native_reply = native(state, {'op': 'send', 'request_id': 'native-reply', 'conversation_id': cid, 'body': 'native reply'}, binary)['result']['Ok']
            page = await independent.request({'op': 'inbox', 'cursor': None, 'limit': 100, 'wait_ms': 0})
            events = page['data']['events']
            assert any(e.get('message') and e['message']['id'] == native_reply['data']['message_id'] for e in events)
            cursor = page['data']['cursor']
            assert cursor
            # Stable mutation IDs are distinct from fresh envelope IDs.
            assert await independent.request(send) == receipt
            await expect_error(independent, dict(send, body='conflicting replay'), 'conflict')
            await expect_error(independent, {'op': 'resolve', 'address': 'connect:operator@local'}, 'forbidden')
            await expect_error(independent, {'op': 'send', 'request_id': 'foreign-send', 'conversation_id': 'foreign-conversation', 'body': 'denied'}, 'forbidden')
            await expect_error(independent, {'op': 'send', 'request_id': 'large-send', 'conversation_id': cid, 'body': 'x' * 16385}, 'invalid-request')
            await expect_error(independent, {'op': 'health', 'unrecognized': True}, 'invalid-request')
            vectors = json.loads((ROOT / 'contracts/fixtures/wire/rejections-v1.json').read_text())['vectors']
            for vector in vectors:
                rejected = await independent.exchange(vector['envelope'])
                assert rejected['type'] == vector['expected_type']
                assert rejected['payload']['code'] == vector['expected_code']
            observations['checks'] = ['standalone-client-entrypoint', 'verified-quic-authentication', 'resolve', 'send-native-observation',
                                      'native-reply-independent-observation', 'stable-mutation-replay',
                                      'conflicting-replay', 'unauthorized-identity', 'unauthorized-conversation',
                                      'oversized-message', 'unknown-request-field', 'incompatible-profile', 'incompatible-envelope-version']
        # Independent incoming contact creates another conversation while peer is offline.
        observations['stage'] = 'incoming-and-cursor-reconnect'
        incoming = native(state, {'op': 'create', 'request_id': 'native-create', 'participants': [tomllib.loads(config.read_text())['identity_id']]}, binary)['result']['Ok']['data']['id']
        incoming_receipt = native(state, {'op': 'send', 'request_id': 'incoming-contact', 'conversation_id': incoming, 'body': 'independently initiated'}, binary)['result']['Ok']['data']
        async with open_client(config, packet_trace) as resumed:
            page = await resumed.request({'op': 'inbox', 'cursor': cursor, 'limit': 100, 'wait_ms': 0})
            resumed_events = page['data']['events']
            assert any(e.get('message') and e['message']['id'] == incoming_receipt['message_id'] for e in resumed_events)
            assert not set(e['id'] for e in events).intersection(e['id'] for e in resumed_events)
            resumed_cursor = page['data']['cursor']
            assert await resumed.request(send) == receipt
            history = await resumed.request({'op': 'history', 'conversation_id': cid, 'after': None, 'limit': 100})
            assert [m['id'] for m in history['data']['messages']].count(receipt['data']['message_id']) == 1
            observations['checks'] += ['reconnect-cursor-resume', 'independent-incoming-contact', 'reconnect-mutation-replay-one-effect']
        observations['stage'] = 'uncertain-ack-replay'
        uncertain = dict(send, request_id='independent-uncertain', body='ack not observed')
        async with open_client(config) as interrupted:
            interrupted.writer.write(encode(envelope('connect.request', dict(profile=PROFILE, **uncertain), cid=cid)))
            await interrupted.writer.drain()
            await asyncio.sleep(.01)
            # Deliberately close without consuming any result; preserve logical request ID.
        async with open_client(config) as recovered:
            uncertain_receipt = await recovered.request(uncertain)
            assert await recovered.request(uncertain) == uncertain_receipt
            observed_history = native(state, {'op': 'history', 'conversation_id': cid, 'after': None, 'limit': 100}, binary)['result']['Ok']['data']['messages']
            assert sum(m['id'] == uncertain_receipt['data']['message_id'] for m in observed_history) == 1
            observations['checks'] += ['uncertain-ack-replay-one-native-effect']
        # Positive replay survives server restart below; frame corruption uses dedicated peers.
        observations['stage'] = 'malformed-oversized-frames'
        framing = json.loads((ROOT / 'contracts/fixtures/wire/framing-v1.json').read_text())['vectors']
        for vector in framing:
            if vector['expected'] == 'stream-closed':
                observations['stage'] = 'wire-rejection:' + vector['name']
                await reject_bad_frame(config, bytes.fromhex(vector['frame_hex']))
        observations['checks'] += ['malformed-json-frame-rejection', 'malformed-envelope-rejection', 'oversized-wire-frame-rejection']
    observations['stage'] = 'server-restart-readiness'
    with Server(state, binary=binary, log_path=state.parent / 'server-restart.log'):
        observations['stage'] = 'restart-cursor-replay-and-current-authority'
        after_restart = native(state, {'op': 'send', 'request_id': 'native-after-restart', 'conversation_id': incoming, 'body': 'after restart'}, binary)['result']['Ok']['data']
        async with open_client(config, packet_trace) as restarted:
            restart_page = await restarted.request({'op': 'inbox', 'cursor': resumed_cursor, 'limit': 100, 'wait_ms': 0})
            assert any(e.get('message') and e['message']['id'] == after_restart['message_id'] for e in restart_page['data']['events'])
            assert not set(e['id'] for e in resumed_events).intersection(e['id'] for e in restart_page['data']['events'])
            observations['checks'] += ['restart-cursor-resume']
            assert await restarted.request(send) == receipt
            await expect_error(restarted, {'op': 'inbox', 'cursor': 'malformed-cursor', 'limit': 100, 'wait_ms': 0}, 'invalid-request')
            agent_cursor = first_inbox['cursor']
            await expect_error(restarted, {'op': 'inbox', 'cursor': agent_cursor, 'limit': 100, 'wait_ms': 0}, 'invalid-request')
            # Revocation rechecks current authority even when replaying a saved receipt.
            credential_id = tomllib.loads(config.read_text())['credential_id']
            run('revoke', '--config', state / 'operator.toml', '--server-config', state / 'server.toml', credential_id, binary=binary)
            try:
                await restarted.request(send)
            except ApplicationError as error:
                assert error.code in ('unauthorized', 'forbidden')
            else:
                raise AssertionError('Revoked cached receipt was disclosed')
        observations['checks'] += ['restart-replay', 'malformed-cursor', 'foreign-identity-cursor', 'revoked-current-authority-replay']
        try:
            async with open_client(config):
                raise AssertionError('Revoked credential authenticated')
        except ApplicationError as error:
            assert error.code == 'unauthorized'
        observations['checks'] += ['revoked-authentication']
    observations['stage'] = 'packet-path-evidence'
    traces = packet_trace.to_dict()['traces']
    counts = {}
    for trace in traces:
        for event in trace['events']:
            name = event['name']
            counts[name] = counts.get(name, 0) + 1
    assert counts.get('transport:packet_sent', 0) > 0 and counts.get('transport:packet_received', 0) > 0
    observations['packet_path'] = {'implementation': 'aioquic qlog transport events', 'event_counts': counts,
                                   'connections': len(traces), 'raw_trace': 'not exported; no TLS secrets log enabled'}
    observations['stage'] = 'complete'
    return observations


def run_git_revision():
    return subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()


def source_dirty():
    return bool(subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT))


def main():
    global CLIENT_ROOT, ApplicationError, MAX_FRAME, PROFILE, encode, envelope, open_client
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=pathlib.Path, default=BINARY,
                        help='Already compiled current/installed Connect; no hidden cold compilation')
    parser.add_argument('--output', type=pathlib.Path)
    parser.add_argument('--source-revision', help='Explicit candidate revision for a verified extracted source archive without .git')
    parser.add_argument('--work-root', type=pathlib.Path, help='Private disposable qualification state outside the extracted source archive')
    parser.add_argument('--client-root', type=pathlib.Path, default=CLIENT_ROOT,
                        help='Exact source or installed independent-client artifact directory')
    args = parser.parse_args()
    CLIENT_ROOT = args.client_root.resolve(strict=True)
    spec = importlib.util.spec_from_file_location('qualified_independent_client', CLIENT_ROOT / 'client.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    ApplicationError, MAX_FRAME, PROFILE = module.ApplicationError, module.MAX_FRAME, module.PROFILE
    encode, envelope, open_client = module.encode, module.envelope, module.open_client
    if importlib.metadata.version('aioquic') != '1.6.1' or PROFILE != 'connect-control/1':
        parser.error('Selected runtime/client profile does not match the pinned conformance profile')
    os.umask(0o077)
    parent = args.work_root or ROOT / '.local/independent-uctp/runs'
    parent.mkdir(parents=True, exist_ok=True)
    work = parent / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    work.mkdir(mode=0o700)
    state = work / 'state'
    observed = {'status': 'running', 'runtime_version': importlib.metadata.version('aioquic'),
                'python': sys.version, 'server_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
                'client_sha256': hashlib.sha256((CLIENT_ROOT / 'client.py').read_bytes()).hexdigest(),
                'source_revision': args.source_revision or run_git_revision(),
                'source_dirty': None if args.source_revision else source_dirty(),
                'source_binding': 'caller-verified archive' if args.source_revision else 'git working tree',
                'requirements_sha256': hashlib.sha256((CLIENT_ROOT / 'requirements.txt').read_bytes()).hexdigest(),
                'checker_sha256': hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
                'checks_not_run': 'remaining checks until conformance returns successfully',
                'media': 'not tested; independent control client has no media implementation'}
    try:
        observed['stage'] = 'selected-native-version'
        observed['server_version'] = run('--version', binary=args.binary).stdout.strip()
        observed['stage'] = 'initialize-fresh-state'
        run('init', '--directory', state, '--example', 'review-loop', '--bind', f'127.0.0.1:{port()}', binary=args.binary)
        observed.update(asyncio.run(qualify(state, args.binary, observed)))
        if hashlib.sha256(args.binary.read_bytes()).hexdigest() != observed['server_sha256']:
            raise RuntimeError('Native binary changed during qualification')
        if hashlib.sha256((CLIENT_ROOT / 'client.py').read_bytes()).hexdigest() != observed['client_sha256']:
            raise RuntimeError('Independent client changed during qualification')
        observed.update(status='passed-control-only', checks_not_run=[], tested_inputs_stable=True)
    except BaseException as error:
        observed.update(status='failed', error_class=type(error).__name__,
                        error_frames=[{'file': pathlib.Path(frame.filename).name, 'line': frame.lineno,
                                       'function': frame.name} for frame in traceback.extract_tb(error.__traceback__)])
    (work / 'result.json').write_text(json.dumps(observed, indent=2) + '\n')
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(observed, indent=2) + '\n')
    print(json.dumps(observed, indent=2))
    return 0 if observed['status'] == 'passed-control-only' else 1


if __name__ == '__main__':
    sys.exit(main())
