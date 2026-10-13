"""Stress-profile verifiers must reject RPC/tool errors and fabricated recovery."""
import json
import contextlib
from pathlib import Path
import sys
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.gate4_phases import phases
from lib.mcp_load_http import ADMISSIONS, BODY_BYTES, Endpoint, admitted_body, encode, success
from lib.mcp_load_profile import pending_waits, recall, slow_bodies


def response(content, pending=False):
    return 200, json.dumps({'jsonrpc': '2.0', 'id': 1, 'result': {
        'isError': False, 'structuredContent': content, '_meta': {'connect': {'pending': pending}}}}).encode()


class McpLoadVerifierTests(unittest.TestCase):
    def connection(self, data):
        connection = Mock()
        buffer = bytearray(data)

        def receive(size):
            self.assertEqual(size, 1)
            byte = bytes(buffer[:size])
            del buffer[:size]
            return byte

        connection.sock.recv.side_effect = receive
        return connection, buffer

    def test_stalled_body_requires_admission_and_preserves_the_final_response(self):
        final = b'HTTP/1.1 504 Gateway Timeout\r\nContent-Length: 0\r\n\r\n'
        connection, buffer = self.connection(b'HTTP/1.1 100 Continue\r\n\r\n' + final)
        endpoint = Endpoint('127.0.0.1:1', 'fixture-token')
        maximum = encode('connect_task_status', {})
        maximum += b' ' * (BODY_BYTES - len(maximum))

        def send(body):
            self.assertEqual(bytes(buffer), final, 'Body sent before exact interim header was consumed')
            self.assertEqual(body, maximum[:-1])

        connection.send.side_effect = send
        with patch('lib.mcp_load_http.http.client.HTTPConnection', return_value=connection) as create, patch(
                'lib.mcp_load_http.time.monotonic', return_value=0):
            opened, started = endpoint.stalled(maximum, admission_deadline=2)
        self.assertIs(opened, connection)
        self.assertEqual(started, 0)
        create.assert_called_once_with(endpoint.address, timeout=2)
        connection.putheader.assert_any_call('Expect', '100-continue')
        connection.putheader.assert_any_call('Content-Length', '524288')
        connection.endheaders.assert_called_once_with()
        connection.send.assert_called_once_with(maximum[:-1])
        connection.getresponse.assert_not_called()
        connection.close.assert_not_called()
        connection.sock.settimeout.assert_any_call(12)

    def test_rejected_admission_never_sends_body_or_replaces_the_request(self):
        connection, _ = self.connection(b'HTTP/1.1 503 Service Unavailable\r\n\r\n')
        endpoint = Endpoint('127.0.0.1:1', 'fixture-token')
        with patch('lib.mcp_load_http.http.client.HTTPConnection', return_value=connection) as create, patch(
                'lib.mcp_load_http.time.monotonic', return_value=0):
            with self.assertRaisesRegex(AssertionError, 'not admitted'):
                endpoint.stalled(b'body', admission_deadline=2)
        self.assertEqual(create.call_count, 1)
        connection.send.assert_not_called()
        connection.close.assert_called_once_with()

    def test_interim_header_is_bounded_and_closed_or_expired_admission_fails(self):
        for data, now, message in [(b'', 0, 'closed'), (b'x' * 4096, 0, 'Oversized'),
                                   (b'HTTP/1.1 100 Continue\r\n\r\n', 2, 'two seconds')]:
            with self.subTest(message=message):
                connection, _ = self.connection(data)
                with patch('lib.mcp_load_http.time.monotonic', return_value=now):
                    with self.assertRaisesRegex(AssertionError, message):
                        admitted_body(connection, 2)

    def test_expired_shared_setup_deadline_does_not_open_another_socket(self):
        with patch('lib.mcp_load_http.http.client.HTTPConnection') as create, patch(
                'lib.mcp_load_http.time.monotonic', return_value=2):
            with self.assertRaisesRegex(AssertionError, 'two seconds'):
                Endpoint('127.0.0.1:1', 'fixture-token').stalled(b'body', admission_deadline=2)
        create.assert_not_called()

    def test_both_waves_require_all_admitted_slots_and_original_body_deadlines(self):
        endpoint, report = Mock(token='fixture-token'), Mock()
        report.phase.side_effect = lambda _: contextlib.nullcontext()
        clock = [0]
        opened, deadlines = [], []

        def stalled(body, *, admission_deadline):
            self.assertEqual(len(body), BODY_BYTES)
            deadlines.append(admission_deadline)
            connection = Mock()
            started = clock[0]

            def completed():
                clock[0] = started + 10
                return Mock(status=504, read=Mock(return_value=b''))

            connection.getresponse.side_effect = completed
            opened.append(connection)
            return connection, started

        endpoint.stalled.side_effect = stalled
        endpoint.request.side_effect = [
            (503, b''), response({'data': {'id': 'task'}}),
            (503, b''), response({'data': {'id': 'task'}}),
        ]
        with patch('lib.mcp_load_profile.time.monotonic', side_effect=lambda: clock[0]):
            slow_bodies(endpoint, b' ' * BODY_BYTES, {'id': 'task'}, report)
        self.assertEqual(len(opened), ADMISSIONS * 2)
        self.assertEqual(deadlines, [2] * ADMISSIONS + [12] * ADMISSIONS)
        for connection in opened:
            connection.getresponse.assert_called_once_with()
            connection.close.assert_called_once_with()
        self.assertEqual(report.latency.call_count, ADMISSIONS * 2)
        for entry in report.checkpoint.call_args_list:
            wave = next(iter(entry.kwargs.values()))
            self.assertEqual(wave['admitted'], 32)
            self.assertEqual(wave['admission_confirmations'], 32)
            self.assertEqual(wave['held_body_bytes_each'], 524287)
            self.assertEqual(wave['timeouts'], 32)
            self.assertEqual(wave['excess_status'], 503)
            self.assertEqual(wave['deadline_status'], 504)
            self.assertEqual(wave['recovered_status'], 200)

    def test_saturation_must_not_accept_31_slots_or_an_early_rejection_as_timeout(self):
        for exhausted, body_status in [(True, 504), (False, 503)]:
            with self.subTest(exhausted=exhausted):
                endpoint, report = Mock(token='fixture-token'), Mock()
                report.phase.side_effect = lambda _: contextlib.nullcontext()
                connections = [Mock() for _ in range(ADMISSIONS)]
                endpoint.stalled.side_effect = [(c, 0) for c in connections]
                for c in connections:
                    c.getresponse.return_value = Mock(status=body_status, read=Mock(return_value=b''))
                endpoint.request.return_value = response({'data': {'id': 'task'}}) if exhausted else (503, b'')
                times = [0, 0, 2] if exhausted else [0, 0, 10]
                with patch('lib.mcp_load_profile.time.monotonic', side_effect=times):
                    with self.assertRaises(AssertionError):
                        slow_bodies(endpoint, b' ' * BODY_BYTES, {'id': 'task'}, report)
                report.checkpoint.assert_not_called()
                for c in connections:
                    c.close.assert_called_once_with()

    def test_status_rpc_and_tool_errors_are_never_success(self):
        good = response({'data': {'id': 'task'}})
        self.assertEqual(success(good)['structuredContent']['data']['id'], 'task')
        bad_rpc = json.loads(good[1])
        bad_rpc['error'] = {'code': -1}
        bad_tool = json.loads(good[1])
        bad_tool['result']['isError'] = True
        wrong_id = json.loads(good[1])
        wrong_id['id'] = 2
        for value in [(503, good[1]), (200, json.dumps(bad_rpc)), (200, json.dumps(bad_tool)), (200, json.dumps(wrong_id))]:
            with self.assertRaises(AssertionError):
                success(value)

    def test_pending_wait_rejects_changed_cursor_or_missing_pending_marker(self):
        baseline = {'data': {'events': [], 'next': 'original'}}
        for value in [response(baseline, False), response({'data': {'events': [], 'next': 'replacement'}}, True)]:
            endpoint = Mock()
            endpoint.request.return_value = response(baseline)
            report = Mock()
            report.phase.return_value.__enter__ = Mock()
            report.phase.return_value.__exit__ = Mock(return_value=False)
            with patch('lib.mcp_load_profile.burst', return_value=[value] * 32), patch(
                    'lib.mcp_load_profile.time.monotonic', side_effect=[0, 9.3]):
                with self.assertRaises(AssertionError):
                    pending_waits(endpoint, report)

    def test_recall_rejects_oversized_projection_and_stalled_cursor(self):
        for content in [
            {'data': {'sources': [], 'next': None, 'padding': 'x' * (32 * 1024)}},
            {'data': {'sources': [], 'next': None}},
        ]:
            report = Mock()
            report.phase.return_value.__enter__ = Mock()
            report.phase.return_value.__exit__ = Mock(return_value=False)
            with patch('lib.mcp_load_profile.burst', return_value=[response(content)] * 32):
                with self.assertRaises(AssertionError):
                    recall(Mock(), {'missing-source'}, report)

    def test_framing_boundary_is_valid_json_before_http_probe(self):
        encoded = encode('connect_task_status', {})
        maximum = encoded + b' ' * (BODY_BYTES - len(encoded))
        self.assertEqual(len(maximum), 524288)
        self.assertEqual(json.loads(maximum)['params']['name'], 'connect_task_status')
        self.assertEqual(len(maximum + b' '), 524289)

    def test_load_phase_follows_normal_build_in_both_modes(self):
        for source_only in (True, False):
            names = [phase[0] for phase in phases(source_only)]
            self.assertEqual(names.count('mcp-operating-load'), 1)
            self.assertLess(names.index('current-binaries'), names.index('mcp-operating-load'))


if __name__ == '__main__':
    unittest.main()
