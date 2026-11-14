"""Pure actual-source parser/propagation controls; never import a workload producer."""
import argparse
import ast
import copy
import io
from contextlib import redirect_stderr
from pathlib import Path
from types import SimpleNamespace
import unittest

ROOT = Path(__file__).resolve().parents[2]


def tree(relative):
    return ast.parse((ROOT / relative).read_text())


def execute(nodes, values):
    module = ast.fix_missing_locations(ast.Module(body=copy.deepcopy(nodes), type_ignores=[]))
    exec(compile(module, '<actual-fixture-wiring>', 'exec'), values)
    return values


def flag_test(node):
    return isinstance(node, ast.If) and 'rtc_audio_facts' in ast.unparse(node.test)


class RtcAudioFactsFixture(unittest.TestCase):
    def test_actual_two_parsers_default_off_and_accept_only_explicit_flag(self):
        for relative, required in [
            ('scripts/run-gate6-transport-matrix.py', ['--phase', 'worker', '--output', '/unused']),
            ('scripts/rehearse-gate6-browser-matrix.py', ['--stage', 'quality', '--transport', 'chrome-wss-webrtc', '--signing', 'EdDSA', '--output', '/unused']),
        ]:
            main = next(n for n in tree(relative).body if isinstance(n, ast.FunctionDef) and n.name == 'main')
            declarations = [n for n in main.body if isinstance(n, ast.Expr) and isinstance(n.value, ast.Call)
                            and isinstance(n.value.func, ast.Attribute) and isinstance(n.value.func.value, ast.Name)
                            and n.value.func.value.id == 'parser' and n.value.func.attr == 'add_argument']
            parser = argparse.ArgumentParser()
            execute(declarations, dict(parser=parser, Path=Path))
            self.assertFalse(parser.parse_args(required).rtc_audio_facts)
            self.assertTrue(parser.parse_args(required + ['--rtc-audio-facts']).rtc_audio_facts)
            with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                parser.parse_args(required + ['--rtc-audio-facts=false'])

    def test_actual_coordinator_and_worker_forward_option_without_selecting_cells(self):
        coordinator = next(n for n in tree('scripts/lib/transport_matrix_coordinator.py').body
                           if isinstance(n, ast.FunctionDef) and n.name == 'run')
        declaration = next(n for n in coordinator.body if isinstance(n, ast.Assign)
                           and any(isinstance(t, ast.Name) and t.id == 'worker' for t in n.targets))
        enabled = next(n for n in coordinator.body if flag_test(n))
        worker = next(n for n in ast.walk(tree('scripts/run-gate6-transport-matrix.py'))
                      if flag_test(n) and 'cell' in ast.unparse(n.test))
        for value in (False, True):
            state = execute([declaration, enabled], dict(args=SimpleNamespace(rtc_audio_facts=value), revision='original-source'))
            self.assertEqual(state['worker'].count('--rtc-audio-facts'), int(value))
            self.assertEqual(state['worker'][:6], ['--phase', 'worker', '--stage', 'full', '--expected-revision', 'original-source'])
            for transport in ('chrome-webtransport', 'native-quic', 'chrome-wss-webrtc', 'firefox-wss-webrtc'):
                argv = ['original-programme']
                execute([worker], dict(args=SimpleNamespace(rtc_audio_facts=value), cell=dict(transport=transport), sys=SimpleNamespace(argv=argv)))
                self.assertEqual(argv, ['original-programme'] + (['--rtc-audio-facts'] if value and transport.endswith('wss-webrtc') else []))

    def test_actual_rehearsal_binds_same_boolean_to_private_input_and_original_server_targets(self):
        main = next(n for n in tree('scripts/rehearse-gate6-browser-matrix.py').body if isinstance(n, ast.FunctionDef) and n.name == 'main')
        option = next(n for n in main.body if isinstance(n, ast.Assign)
                      and any(isinstance(t, ast.Name) and t.id == 'rtc_audio_facts' for t in n.targets))
        log = next(n for n in main.body if isinstance(n, ast.Assign) and any(isinstance(t, ast.Subscript)
                   and isinstance(t.value, ast.Name) and t.value.id == 'env' and isinstance(t.slice, ast.Constant)
                   and t.slice.value == 'RUST_LOG' for t in n.targets))
        enabled = next(n for n in main.body if isinstance(n, ast.If) and isinstance(n.test, ast.Name) and n.test.id == 'rtc_audio_facts')
        counter_log = next(n for n in main.body if isinstance(n, ast.If)
                           and 'original_completion_deferral_ms' in ast.unparse(n.test)
                           and 'RUST_LOG' in ast.unparse(n))
        keyword = next(n for n in ast.walk(main) if isinstance(n, ast.keyword) and n.arg == 'rtcAudioFacts')
        self.assertIsInstance(keyword.value, ast.Name)
        self.assertEqual(keyword.value.id, 'rtc_audio_facts')
        base_log = ('warn,connect_service::canonical_replacement=debug,rvoip_core::replacement_commit=debug,'
                    'rvoip_core::audio_summary=debug,connect_service::packet_refusal=debug,'
                    'connect_service::runtime_retention=debug,quinn_proto::datagram_queue=debug')
        private_targets = ('rvoip_webrtc_stack::audio_facts=debug', 'rvoip_webrtc::audio_facts=debug',
                           'rvoip_webrtc::canonical::media=debug')
        for requested in (False, True):
            for transport in ('chrome-webtransport', 'chrome-wss-webrtc', 'firefox-wss-webrtc'):
                env = {'PATH': 'unchanged', 'RUST_LOG': 'trace,foreign_target=trace'}
                values = execute([option, log, enabled], dict(args=SimpleNamespace(rtc_audio_facts=requested, transport=transport), env=env))
                expected = requested and transport.endswith('wss-webrtc')
                self.assertEqual(values['rtc_audio_facts'], expected)
                self.assertEqual(env['RUST_LOG'].count('rvoip_webrtc_stack::audio_facts=debug'), int(expected))
                self.assertEqual(env['RUST_LOG'].count('rvoip_webrtc::audio_facts=debug'), int(expected))
                self.assertEqual(env['RUST_LOG'].count('rvoip_webrtc::canonical::media=debug'), int(expected))
                self.assertEqual(env['RUST_LOG'], base_log + (',' + ','.join(private_targets) if expected else ''))
                self.assertNotIn('trace', env['RUST_LOG'])
                self.assertEqual(env['PATH'], 'unchanged')
                for hold in (None, 0, 250, -1, 500):
                    for stage, cycles, network in [('quality', 1, 'local'), ('admission', 1, 'local'),
                                                   ('quality', 3, 'local'), ('quality', 1, 'remote-profile-fixture')]:
                        counter_env = {'PATH': 'unchanged', 'RUST_LOG': 'trace,foreign_target=trace'}
                        counter_args = SimpleNamespace(rtc_audio_facts=requested, transport=transport,
                            original_completion_deferral_ms=hold, stage=stage, resource_cycles=cycles, network=network)
                        execute([option, log, enabled, counter_log], dict(args=counter_args, env=counter_env))
                        counter = hold in (0, 250) and stage == 'quality' and cycles == 1 and network == 'local' and transport.endswith('wss-webrtc')
                        wanted = base_log + (',' + ','.join(private_targets) if expected else '')
                        wanted += ',rvoip_webrtc_stack::owned_rtp_refusal=debug' if counter else ''
                        self.assertEqual(counter_env['RUST_LOG'], wanted)
                        self.assertNotIn('trace', counter_env['RUST_LOG'])
                        self.assertEqual(counter_env['PATH'], 'unchanged')
        node = (ROOT / 'scripts/video/browser_matrix_admission.mjs').read_text()
        self.assertIn('page.addInitScript(installPeerDiagnostics,{peerBound:16,rtcAudioFacts})', node)
        self.assertLess(node.index('page.addInitScript(installPeerDiagnostics'), node.index('await page.goto(spec.origin'))
        self.assertIn("assert.equal(typeof rtcAudioFacts,'boolean')", node)


if __name__ == '__main__':
    unittest.main()
