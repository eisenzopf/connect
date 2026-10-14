"""Language-neutral frame vectors, hostile replies and implementation independence."""
import ast
import asyncio
import json
import pathlib
import struct
import unittest

from client import Client, ProtocolError, close_writer, decode, encode, envelope

ROOT = pathlib.Path(__file__).resolve().parents[2]


class WireVectors(unittest.TestCase):
    def test_published_frames_are_exact_and_parser_is_independent(self):
        vectors = json.loads((ROOT / 'contracts/fixtures/wire/framing-v1.json').read_text())['vectors']
        for vector in vectors:
            frame = bytes.fromhex(vector['frame_hex'])
            if vector['expected'] == 'valid-envelope':
                self.assertEqual(encode(vector['json']), frame)
                self.assertEqual(struct.unpack('!I', frame[:4])[0], vector['utf8_bytes'])
                self.assertEqual(decode(frame[4:]), vector['json'])
            elif vector['name'] in ('malformed-json', 'malformed-envelope'):
                with self.assertRaises(ProtocolError):
                    decode(frame[4:])
        tree = ast.parse((ROOT / 'examples/independent-uctp-client/client.py').read_text())
        imported = {n.names[0].name.split('.')[0] if isinstance(n, ast.Import) else n.module.split('.')[0]
                    for n in ast.walk(tree) if isinstance(n, (ast.Import, ast.ImportFrom))}
        self.assertEqual(imported, {'argparse', 'asyncio', 'datetime', 'json', 'pathlib', 're',
                                   'ssl', 'struct', 'sys', 'tomllib', 'uuid', 'contextlib', 'aioquic'})
        source = (ROOT / 'examples/independent-uctp-client/client.py').read_text()
        self.assertNotIn('subprocess', source)
        self.assertNotIn('connect_sdk', source)
        self.assertNotIn('ctypes', source)

    def test_bad_envelope_rejects_before_use(self):
        good = envelope('connect.result', {})
        for patch in ({'v': 255}, {'v': True}, {'id': 'bad\n'}, {'ts': '2026-01-01T00:00:00'},
                      {'payload': None}, {'in_reply_to': 'x' * 129}):
            with self.subTest(patch=patch), self.assertRaises(ProtocolError):
                decode(json.dumps(dict(good, **patch)).encode())


class HostileReplies(unittest.IsolatedAsyncioTestCase):
    async def test_wrong_kind_missing_fields_and_profile_fail(self):
        for payload in ({'profile': 'other/1', 'result': {'Ok': {'kind': 'identity', 'data': {}}}},
                        {'profile': 'connect-control/1', 'result': {'Ok': {'kind': 'alien', 'data': {}}}},
                        {'profile': 'connect-control/1', 'result': {'Ok': {'kind': 'identity', 'data': {}}}},
                        {'profile': 'connect-control/1', 'result': {'Err': {'code': None}}}):
            peer = Client(None, None, 1)
            async def exchange(_value):
                return envelope('connect.result', payload)
            peer.exchange = exchange
            with self.assertRaises(ProtocolError):
                await peer.request({'op': 'resolve', 'address': 'connect:agent@local'})

    async def test_missing_profile_never_sends_credential(self):
        peer = Client(None, None, 1)
        calls = []
        async def exchange(value):
            calls.append(value)
            return envelope('auth.challenge', {'accepted_methods': ['bearer'], 'server_capabilities': {}})
        peer.exchange = exchange
        with self.assertRaises(ProtocolError):
            await peer.authenticate('private-fixture-token')
        self.assertEqual(len(calls), 1)
        self.assertNotIn('private-fixture-token', json.dumps(calls))

    async def test_finished_quic_stream_cleanup_preserves_terminal_state(self):
        from aioquic.asyncio.protocol import QuicConnectionProtocol
        from aioquic.quic.configuration import QuicConfiguration
        from aioquic.quic.connection import QuicConnection
        quic = QuicConnection(configuration=QuicConfiguration(is_client=True))
        protocol = QuicConnectionProtocol(quic)
        _reader, writer = await protocol.create_stream()
        # The peer finished this real runtime stream before cleanup received it.
        quic._streams_finished.add(writer.get_extra_info('stream_id'))
        close_writer(writer)
        self.assertTrue(writer.is_closing())
        class BrokenWriter:
            def close(self):
                raise ValueError('Cannot send data on unknown peer-initiated stream')
        with self.assertRaises(ValueError):
            close_writer(BrokenWriter())

    async def test_oversized_frame_and_truncated_prefix(self):
        for data in (b'\x00\x10\x00\x01', b'\x00\x00\x00'):
            reader = asyncio.StreamReader()
            reader.feed_data(data)
            reader.feed_eof()
            peer = Client(reader, None, 1)
            with self.assertRaises((ProtocolError, asyncio.IncompleteReadError)):
                await peer.receive()


if __name__ == '__main__':
    unittest.main()
