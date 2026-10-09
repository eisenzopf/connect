"""Offline client regressions. No provider credentials or network effects."""
import concurrent.futures
import json
from pathlib import Path
import threading
import time
import tempfile
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from connect_sdk import ConnectError, HttpClient, NativeAudioClient, NativeClient
from connect_sdk.protocol import decode_response

HELPER = str(Path(__file__).with_name('helper_fixture.py').resolve())
ROOT = Path(__file__).resolve().parents[2]


class NativeTests(unittest.TestCase):
    def test_remote_media_end_is_prompt_and_distinct_from_uncertainty(self):
        with NativeAudioClient('media-ended', 'conversation', 'session', 'remote', HELPER, timeout=1) as media:
            for operation in [media.receive, media.receive, lambda: media.send_pcmu(bytes(160))]:
                with self.assertRaises(ConnectError) as error:
                    operation()
                self.assertEqual(error.exception.code, 'media-ended')

    def test_media_diagnostics_cannot_fill_an_unread_helper_pipe(self):
        with tempfile.TemporaryFile() as diagnostics:
            with NativeAudioClient('media-noisy', 'conversation', 'session', 'remote',
                                   HELPER, timeout=2, stderr=diagnostics) as media:
                frame = bytes(160)
                media.send_pcmu(frame)
                self.assertEqual(media.receive(), frame)
            diagnostics.seek(0)
            self.assertGreater(len(diagnostics.read()), 65536)

    def test_binary_media_helper_is_bounded_and_keeps_audio_out_of_json(self):
        with NativeAudioClient('media-ok', 'conversation', 'session', 'remote', HELPER) as media:
            self.assertEqual(media.publisher_stream_id, 'fixture-stream')
            media.subscribe('other-stream')
            frame = bytes(range(160))
            media.send_pcmu(frame)
            self.assertEqual(media.receive(), frame)
            with self.assertRaises(ConnectError) as error:
                media.send_pcmu(b'short')
            self.assertEqual(error.exception.code, 'invalid-request')

        for mode in ['media-stall', 'media-oversize']:
            with self.subTest(mode=mode), self.assertRaises(ConnectError):
                NativeAudioClient(mode, 'conversation', 'session', 'remote', HELPER, timeout=.2)

    def test_partial_and_absent_responses_obey_deadline_and_reap_helper(self):
        for mode in ['fragment', 'stall']:
            with self.subTest(mode=mode):
                client = NativeClient(mode, HELPER, timeout=.4)
                start = time.monotonic()
                with self.assertRaises(ConnectError) as error:
                    client.health()
                self.assertEqual(error.exception.code, 'transport-uncertain')
                self.assertLess(time.monotonic() - start, 1.5)
                self.assertIsNotNone(client._process.poll())
                with self.assertRaises(ConnectError):
                    client.health()
                client.close(); client.close()

    def test_malformed_frames_close_client_with_protocol_error(self):
        for mode in ['oversize', 'malformed', 'wrong-id', 'wrong-kind', 'bad-fields', 'invalid-utf8']:
            with self.subTest(mode=mode), NativeClient(mode, HELPER) as client:
                with self.assertRaises(ConnectError) as error:
                    client.health()
                self.assertEqual(error.exception.code, 'protocol')
                self.assertIsNotNone(client._process.poll())

    def test_chunked_frames_and_concurrent_callers(self):
        with NativeClient('chunked', HELPER) as client:
            with concurrent.futures.ThreadPoolExecutor(max_workers=4) as executor:
                results = list(executor.map(lambda _: client.health(), range(8)))
            self.assertTrue(all(r['installation_id'] == 'fixture' for r in results))

    def test_application_denial_preserves_client(self):
        with NativeClient('denied', HELPER) as client:
            with self.assertRaises(ConnectError) as error:
                client.health()
            self.assertEqual(error.exception.code, 'forbidden')
            self.assertEqual(client.health()['schema'], 7)

    def test_missing_helper_has_stable_error(self):
        with self.assertRaises(ConnectError) as error:
            NativeClient("unused", "/no-such-connect-helper")
        self.assertEqual(error.exception.code, "transport-uncertain")

    def test_invalid_deadline_is_rejected(self):
        for value in [0, -1, float('inf'), float('nan'), 61, True]:
            with self.assertRaises(ValueError):
                NativeClient('unused', HELPER, timeout=value)


class HttpTests(unittest.TestCase):
    def test_raw_artifact_bytes_round_trip_without_base64(self):
        artifact = {
            'id': 'artifact', 'conversation_id': 'conversation', 'owner_id': 'owner',
            'name': 'sample.bin', 'media_type': 'application/octet-stream',
            'size': 4, 'received': 4, 'sha256': '0' * 64, 'state': 'uploading',
            'created_at': 1, 'expires_at': None,
        }

        class Handler(BaseHTTPRequestHandler):
            uploaded = None
            def log_message(self, *args): pass
            def do_PUT(self):
                Handler.uploaded = self.rfile.read(int(self.headers['Content-Length']))
                body = json.dumps({'profile': 'connect-control/1', 'result': {'Ok': {'kind': 'artifact', 'data': artifact}}}).encode()
                self.send_response(200); self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body)
            def do_GET(self):
                body = b'raw!'
                self.send_response(200); self.send_header('Content-Type', 'application/octet-stream')
                self.send_header('X-Connect-Offset', '0'); self.send_header('X-Connect-Eof', 'true')
                self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body)

        server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
        try:
            client = HttpClient(f'http://127.0.0.1:{server.server_port}', 'fixture')
            result = client.upload_artifact_chunk('conversation', 'artifact', 'append-1', 0, b'raw!')
            self.assertEqual(result['data']['id'], 'artifact')
            self.assertEqual(Handler.uploaded, b'raw!')
            chunk = client.download_artifact_chunk('conversation', 'artifact', 0, 4)
            self.assertEqual(chunk, {'data': b'raw!', 'offset': 0, 'eof': True})
        finally:
            server.shutdown(); server.server_close(); thread.join()

    def test_http_denial_validation_redirects_bounds_and_stall(self):
        class Handler(BaseHTTPRequestHandler):
            mode = 'denied'
            requests = 0
            def log_message(self, *args): pass
            def do_POST(self):
                Handler.requests += 1
                self.rfile.read(int(self.headers['Content-Length']))
                self.send_response(302 if self.mode == 'redirect' else 403 if self.mode == 'denied' else 200)
                self.send_header('Location', '/must-not-follow')
                self.end_headers()
                body = b'{"profile":"connect-control/1","result":{"Err":{"code":"forbidden","message":"fixture"}}}'
                if self.mode == 'malformed': body = b'{}'
                if self.mode == 'oversize': body = b'x' * 600001
                if self.mode == 'stall':
                    self.wfile.write(b'{'); self.wfile.flush(); time.sleep(1); return
                try: self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError): pass
        server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
        try:
            for mode, code in [('denied','forbidden'),('malformed','protocol'),('oversize','protocol'),('redirect','protocol'),('stall','transport-uncertain')]:
                with self.subTest(mode=mode):
                    Handler.mode = mode
                    before = Handler.requests
                    start = time.monotonic()
                    with self.assertRaises(ConnectError) as error:
                        HttpClient(f'http://127.0.0.1:{server.server_port}', 'fixture', timeout=.2).health()
                    self.assertEqual(error.exception.code, code)
                    self.assertEqual(Handler.requests, before + 1)
                    self.assertLess(time.monotonic() - start, 1)
        finally:
            server.shutdown(); server.server_close(); thread.join()


class ContractTests(unittest.TestCase):
    def test_rust_result_fixtures_validate_in_python(self):
        for path in (ROOT/'contracts/fixtures/results').glob('*.json'):
            response = json.loads(path.read_text())['payload']
            result = response['result']
            if 'Err' in result:
                with self.assertRaises(ConnectError) as error:
                    decode_response(json.dumps(response), 'health')
                self.assertEqual(error.exception.code, result['Err']['code'])
            else:
                self.assertEqual(decode_response(json.dumps(response), result['Ok']['kind']), result['Ok'])

if __name__ == '__main__': unittest.main()
