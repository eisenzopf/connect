"""Stress-profile verifiers must reject RPC/tool errors and fabricated recovery."""
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.gate4_phases import phases
from lib.mcp_load_http import BODY_BYTES, encode, success
from lib.mcp_load_profile import pending_waits, recall


def response(content, pending=False):
    return 200, json.dumps({'jsonrpc': '2.0', 'id': 1, 'result': {
        'isError': False, 'structuredContent': content, '_meta': {'connect': {'pending': pending}}}}).encode()


class McpLoadVerifierTests(unittest.TestCase):
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
