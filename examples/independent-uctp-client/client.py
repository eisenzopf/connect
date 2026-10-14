#!/usr/bin/env python3
"""Independent Connect control/1 over aioquic. No helper or Connect SDK imports."""
import argparse
import asyncio
import datetime
import json
import pathlib
import re
import ssl
import struct
import sys
import tomllib
import uuid
from contextlib import asynccontextmanager

from aioquic.asyncio import connect
from aioquic.quic.configuration import QuicConfiguration

PROFILE = 'connect-control/1'
MAX_FRAME = 1024 * 1024
MAX_REQUEST = 64 * 1024


class ProtocolError(Exception):
    """A peer violated the negotiated control profile."""


class ApplicationError(Exception):
    def __init__(self, code):
        self.code = code
        super().__init__(code)


def envelope(kind, payload, **fields):
    return dict(v=1, type=kind, id='env_' + uuid.uuid4().hex,
                ts=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                payload=payload, **fields)


def encode(value, limit=MAX_FRAME):
    data = json.dumps(value, separators=(',', ':'), ensure_ascii=False).encode()
    if not data or len(data) > limit:
        raise ProtocolError('frame-size')
    return struct.pack('!I', len(data)) + data


def decode(data):
    try:
        value = json.loads(data)
        if (not isinstance(value, dict) or type(value.get('v')) is not int
                or value['v'] != 1 or not isinstance(value.get('payload'), dict)
                or not all(isinstance(value.get(k), str) and value[k] for k in ('id', 'ts', 'type'))):
            raise ValueError()
        timestamp = datetime.datetime.fromisoformat(value['ts'].replace('Z', '+00:00'))
        if timestamp.tzinfo is None:
            raise ValueError()
        for key in ('id', 'in_reply_to'):
            if key in value and not (isinstance(value[key], str) and re.fullmatch(r'[A-Za-z0-9._~-]{1,128}', value[key])):
                raise ValueError()
        return value
    except (ValueError, TypeError, UnicodeError) as error:
        raise ProtocolError('malformed-envelope') from error


def close_writer(writer):
    """The pinned QUIC runtime may finish/reset a stream before local cleanup."""
    try:
        writer.close()
    except (RuntimeError, ValueError) as error:
        if str(error) not in ('Cannot send data after the stream was reset',
                             'Cannot send data on a finished stream'):
            raise


class Client:
    def __init__(self, reader, writer, deadline):
        self.reader, self.writer, self.deadline = reader, writer, deadline
        self.hints = []
        self.failed = False

    async def receive(self):
        size = struct.unpack('!I', await self.reader.readexactly(4))[0]
        if not 0 < size <= MAX_FRAME:
            raise ProtocolError('frame-size')
        return decode(await self.reader.readexactly(size))

    async def exchange(self, value):
        if self.failed:
            raise ProtocolError('connection-terminal')
        try:
            return await self._exchange(value)
        except BaseException:
            self.failed = True
            close_writer(self.writer)
            raise

    async def _exchange(self, value):
        # One absolute deadline includes draining hints; no per-hint extension.
        async with asyncio.timeout(self.deadline):
            self.writer.write(encode(value, MAX_REQUEST))
            await self.writer.drain()
            for _ in range(257):
                reply = await self.receive()
                if reply.get('in_reply_to') == value['id']:
                    return reply
                if reply['type'] != 'connect.event' or len(self.hints) >= 256:
                    raise ProtocolError('unexpected-correlation')
                self.hints.append(reply)
            raise ProtocolError('hint-capacity')

    async def authenticate(self, credential):
        challenge = await self.exchange(envelope('auth.hello', {
            'device': {'id': 'independent-python', 'kind': 'desktop', 'platform': sys.platform,
                       'sdk_version': '1'}, 'auth_methods': ['bearer'], 'capabilities': {}}))
        payload = challenge['payload']
        if (challenge['type'] != 'auth.challenge' or 'bearer' not in payload.get('accepted_methods', [])
                or PROFILE not in payload.get('server_capabilities', {}).get('application_profiles', [])):
            raise ProtocolError('unsupported-profile')
        # This deployed bearer profile correlates response to its own envelope,
        # not to the challenge. The nonce is not a bearer challenge response.
        session = await self.exchange(envelope('auth.response', {'method': 'bearer', 'credential': credential}))
        if session['type'] != 'auth.session':
            raise ApplicationError('unauthorized')
        if not all(isinstance(session['payload'].get(k), str) and session['payload'][k]
                   for k in ('identity_id', 'participant_id', 'expires_at', 'session_token')):
            raise ProtocolError('malformed-auth-session')
        return session['payload']['identity_id']

    async def request(self, operation):
        if not isinstance(operation, dict):
            raise ProtocolError('operation-shape')
        if operation.get('op') not in ('resolve', 'create', 'send', 'history', 'inbox', 'request_status', 'health'):
            raise ProtocolError('unsupported-client-operation')
        fields = {'cid': operation['conversation_id']} if 'conversation_id' in operation else {}
        reply = await self.exchange(envelope('connect.request', dict(profile=PROFILE, **operation), **fields))
        if reply['type'] == 'error':
            raise ApplicationError({401: 'unauthorized', 403: 'forbidden', 503: 'storage-unavailable'}
                                   .get(reply['payload'].get('code'), 'protocol'))
        if (reply['type'] != 'connect.result' or reply['payload'].get('profile') != PROFILE
                or ('cid' in reply and reply['cid'] != fields.get('cid'))):
            raise ProtocolError('result-profile')
        result = reply['payload'].get('result')
        if not isinstance(result, dict) or len(result) != 1:
            raise ProtocolError('result-shape')
        if 'Err' in result:
            code = result['Err'].get('code')
            if not isinstance(code, str):
                raise ProtocolError('error-shape')
            raise ApplicationError(code)
        outcome = result.get('Ok')
        if not isinstance(outcome, dict) or not isinstance(outcome.get('kind'), str) or 'data' not in outcome:
            raise ProtocolError('outcome-shape')
        expected = {'resolve': ('identity', ('id', 'alias')),
                    'create': ('conversation', ('id', 'participants', 'closed')),
                    'send': ('accepted', ('request_id', 'message_id', 'conversation_id')),
                    'history': ('history', ('messages', 'next')),
                    'inbox': ('inbox', ('events', 'cursor')),
                    'request_status': ('request_status', ('state', 'outcome')),
                    'health': ('health', ('installation_id', 'schema', 'profile'))}.get(operation.get('op'))
        if expected is None:
            raise ProtocolError('unsupported-client-operation')
        kind, required = expected
        if (outcome['kind'] != kind or not isinstance(outcome['data'], dict)
                or any(key not in outcome['data'] for key in required)):
            raise ProtocolError('outcome-fields')
        data = outcome['data']
        string_fields = {'identity': ('id', 'alias'),
                         'conversation': ('id',),
                         'accepted': ('request_id', 'message_id', 'conversation_id'),
                         'request_status': ('state',),
                         'health': ('installation_id', 'profile')}.get(kind, ())
        if any(not isinstance(data[key], str) or not data[key] for key in string_fields):
            raise ProtocolError('outcome-field-type')
        if kind == 'conversation' and (not isinstance(data['participants'], list)
                or any(not isinstance(p, str) for p in data['participants']) or type(data['closed']) is not bool):
            raise ProtocolError('conversation-shape')
        if kind == 'health' and type(data['schema']) is not int:
            raise ProtocolError('health-shape')
        if kind in ('history', 'inbox'):
            items, cursor = ('messages', 'next') if kind == 'history' else ('events', 'cursor')
            if not isinstance(data[items], list) or not (data[cursor] is None or isinstance(data[cursor], str)):
                raise ProtocolError('page-shape')
            if any(not isinstance(item, dict) or not isinstance(item.get('id'), str) for item in data[items]):
                raise ProtocolError('page-entry')
        return outcome


@asynccontextmanager
async def open_client(path, quic_logger=None):
    path = pathlib.Path(path)
    config = tomllib.loads(path.read_text())
    host, port = config['endpoint'].rsplit(':', 1)
    cert = pathlib.Path(config['trust_certificate'])
    if not cert.is_absolute():
        cert = path.parent / cert
    # Certificate DER is the explicitly selected trust anchor; verification stays REQUIRED.
    tls = QuicConfiguration(is_client=True, alpn_protocols=['uctp/1'],
                            server_name=config['server_name'], verify_mode=ssl.CERT_REQUIRED,
                            cadata=ssl.DER_cert_to_PEM_cert(cert.read_bytes()).encode(), idle_timeout=30,
                            quic_logger=quic_logger)
    async with asyncio.timeout(config['timeout_ms'] / 1000):
        context = connect(host.strip('[]'), int(port), configuration=tls)
        protocol = await context.__aenter__()
    writer = None
    try:
        if protocol._quic.tls.alpn_negotiated != 'uctp/1':
            raise ProtocolError('alpn')
        reader, writer = await protocol.create_stream()
        client = Client(reader, writer, config['timeout_ms'] / 1000)
        client.identity_id = await client.authenticate(config['credential'])
        client.transport = {'runtime': 'aioquic', 'alpn': protocol._quic.tls.alpn_negotiated,
                            'tls_verified': True, 'stream_id': writer.get_extra_info('stream_id'),
                            'network': 'UDP/QUIC', 'control_only': True}
        yield client
    finally:
        if writer is not None:
            close_writer(writer)
        await context.__aexit__(None, None, None)


async def main(path):
    async with open_client(path) as client:
        # Stdin JSON lines allow distinct processes to maintain one live connection.
        print(json.dumps({'ready': client.transport}), flush=True)
        while True:
            line = await asyncio.to_thread(sys.stdin.buffer.readline, MAX_REQUEST + 1)
            if not line:
                break
            if len(line) > MAX_REQUEST:
                raise ProtocolError('request-size')
            try:
                print(json.dumps({'Ok': await client.request(json.loads(line))}), flush=True)
            except ApplicationError as error:
                print(json.dumps({'Err': {'code': error.code}}), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=pathlib.Path, required=True)
    args = parser.parse_args()
    try:
        asyncio.run(main(args.config))
    except (ApplicationError, ProtocolError, TimeoutError, OSError, ValueError) as error:
        # Never print exception values containing peer inputs or credentials.
        print(json.dumps({'failed': type(error).__name__}), file=sys.stderr)
        sys.exit(1)
