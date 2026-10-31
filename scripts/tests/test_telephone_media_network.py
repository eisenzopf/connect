"""Concrete endpoint, custody and evidence boundaries for owned telephone CI."""
import json
from pathlib import Path
import runpy
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
phone = runpy.run_path(str(ROOT / 'scripts/telephone/linux_remote_phone.py'))
component = runpy.run_path(str(ROOT / 'scripts/lib/telephone_network_component.py'))


class TelephoneNetworkBoundaryTests(unittest.TestCase):
    def test_linux_gateway_requires_explicit_owned_mode_and_the_same_concrete_advertised_host(self):
        self.assertEqual(phone['designated_gateway']({'gateway': '192.168.5.2'}), '192.168.5.2')
        valid = dict(owned_internal_bridge=True, gateway='172.28.0.1', host_media_ip='172.28.0.1')
        self.assertEqual(phone['designated_gateway'](valid), '172.28.0.1')
        for change in (dict(owned_internal_bridge=False), dict(owned_internal_bridge='true'),
                       dict(host_media_ip='172.29.0.1')):
            with self.subTest(change=change), self.assertRaises(AssertionError):
                phone['designated_gateway'](dict(valid, **change))
        for address in ('8.8.8.8', '127.0.0.1', '0.0.0.0', '224.0.0.1', '169.254.1.1'):
            with self.subTest(address=address), self.assertRaises(AssertionError):
                phone['designated_gateway'](dict(valid, gateway=address, host_media_ip=address))

    def test_uncertain_creation_recovers_only_its_exact_name_label_and_empty_original_network(self):
        parent = ROOT / '.local/test-owned-telephone-network'
        parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=parent) as directory:
            calls = []
            owner = component['OwnedBridge'](directory)
            identity = 'a' * 64
            info = dict(Name=owner.name, Id=identity, Internal=True, Driver='bridge', Containers={},
                Labels={'connect.gate6.telephone-media': owner.label},
                IPAM=dict(Config=[dict(Subnet='172.28.0.0/16', Gateway='172.28.0.1')]))
            def invoke(args):
                calls.append(args)
                if args[2] == 'create': raise subprocess.TimeoutExpired(args[:3], 20)
                if args[2] == 'inspect': return json.dumps([info])
                self.assertEqual(args, ['docker', 'network', 'rm', identity])
                return identity
            owner.invoke = invoke
            with self.assertRaises(subprocess.TimeoutExpired): owner.create()
            self.assertEqual(calls[0][-1], owner.name)
            self.assertIsNone(owner.network)
            report = owner.close()
            self.assertTrue(report['original_bridge_removed'])
            self.assertEqual(report['remaining_containers'], 0)
            saved = json.loads((Path(directory) / 'owned-telephone-network.json').read_text())
            self.assertEqual(saved['network'], identity)
            self.assertTrue(saved['cleanup']['original_bridge_removed'])

    def test_cleanup_cannot_remove_a_different_owner_or_an_occupied_network(self):
        parent = ROOT / '.local/test-owned-telephone-network'
        parent.mkdir(parents=True, exist_ok=True)
        for occupied in (False, True):
            with self.subTest(occupied=occupied), tempfile.TemporaryDirectory(dir=parent) as directory:
                owner = component['OwnedBridge'](directory)
                owner.started = True
                owner.network = 'a' * 64
                info = dict(Name=owner.name, Id=owner.network, Internal=True, Driver='bridge',
                    Containers={'retained': {}} if occupied else {},
                    Labels={'connect.gate6.telephone-media': owner.label if occupied else 'unrelated'})
                calls = []
                def invoke(args):
                    calls.append(args)
                    self.assertEqual(args[2], 'inspect')
                    return json.dumps([info])
                owner.invoke = invoke
                report = owner.close()
                self.assertFalse(report['original_bridge_removed'])
                self.assertEqual(report['cleanup_error'], 'AssertionError')
                self.assertEqual(len(calls), 1)

    def test_public_facts_drop_identifiers_nonce_addresses_errors_and_nonfinite_counters(self):
        value = dict(passed=False, failure={'type': 'AssertionError', 'message': 'private-secret'},
            source_revision='a'*40, profiles=[dict(algorithm='EdDSA', passed=False,
                guest=dict(os='linux', architecture='x86_64', received_rtp=180, nonce='private-secret'),
                server_witness=[dict(original_invite_replayed=True, call_id='private-secret')],
                server_failure=dict(media_go_written=False, private='private-secret', panic_locations=[
                    dict(file='crates/connect-server/tests/fixture.rs',line=100,column=7),
                    dict(file='/private-secret',line=1,column=1),
                    dict(file='crates/../private-secret',line=1,column=1)]),
                peer_nat=dict(worker_joined=True, unique_calls=1, error='private-secret',
                    rtp=dict(guest_to_server=180, server_to_guest=float('inf'), address='private-secret')))])
        result = component['public_result'](value)
        self.assertNotIn('private-secret', json.dumps(result, allow_nan=False))
        self.assertEqual(result['profiles'][0]['guest']['received_rtp'], 180)
        self.assertTrue(result['profiles'][0]['server_witness'][0]['original_invite_replayed'])
        self.assertNotIn('server_to_guest', result['profiles'][0]['peer_nat']['rtp'])
        self.assertEqual(result['profiles'][0]['server_failure']['panic_locations'],
            [dict(file='crates/connect-server/tests/fixture.rs',line=100,column=7)])


if __name__ == '__main__':
    unittest.main()
