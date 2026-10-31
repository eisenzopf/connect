"""Ownership and public-evidence boundaries for the current native network lane."""
import copy
import json
from pathlib import Path
import runpy
import unittest

ROOT = Path(__file__).resolve().parents[2]
runner = runpy.run_path(str(ROOT / 'scripts/rehearse-gate6-native-media.py'))
component = runpy.run_path(str(ROOT / 'scripts/lib/native_network_component.py'))


class NativeNetworkBoundaryTests(unittest.TestCase):
    def test_only_the_exact_empty_internal_bridge_supplies_the_designated_gateway(self):
        identity = 'a' * 64
        info = dict(Id=identity, Internal=True, Driver='bridge', Containers={},
            IPAM=dict(Config=[dict(Subnet='172.28.0.0/16', Gateway='172.28.0.1')]))
        self.assertEqual(runner['owned_gateway'](identity, info), '172.28.0.1')
        for change in (dict(Id='b'*64), dict(Internal=False), dict(Driver='host'),
                       dict(Containers={'unowned': {}}),
                       dict(IPAM=dict(Config=[dict(Subnet='172.28.0.0/16', Gateway='127.0.0.1')])),
                       dict(IPAM=dict(Config=[dict(Subnet='172.28.0.0/16', Gateway='172.29.0.1')]))):
            with self.subTest(change=change), self.assertRaises(AssertionError):
                runner['owned_gateway'](identity, dict(info, **change))

    def test_no_raw_error_config_private_witness_or_nonfinite_counter_enters_public_evidence(self):
        value = dict(passed=False, mode='loss', scope='private-secret', source_revision='a'*40,
            image_id='sha256:'+'b'*64, failure=dict(type='AssertionError', message='private-secret'),
            owned_network=dict(requested=True, internal_bridge=True, original_bridge_removed=True,
                remaining_containers=0, configuration='private-secret'), profiles=[dict(algorithm='EdDSA',
                passed=False, guest_witness=[dict(algorithm='EdDSA', profile='remote', os='linux',
                    architecture='x86_64', received_bytes=8000, credential='private-secret')],
                server_witness=[dict(algorithm='private-secret', physical_peers_drained=True)],
                transport_fault=dict(error='private-secret', worker_joined=True,
                    guest_to_server=dict(eligible=100, dropped=10, forwarded=90, address='private-secret'),
                    server_to_guest=dict(eligible=float('inf'), dropped=-1, forwarded=90)))])
        before = copy.deepcopy(value)
        result = component['public_result'](value)
        self.assertNotIn('private-secret', json.dumps(result, allow_nan=False))
        self.assertEqual(result['profiles'][0]['guest_witness'][0]['received_bytes'], 8000)
        self.assertEqual(result['profiles'][0]['transport_fault']['guest_to_server']['dropped'], 10)
        self.assertNotIn('eligible', result['profiles'][0]['transport_fault']['server_to_guest'])
        self.assertNotIn('dropped', result['profiles'][0]['transport_fault']['server_to_guest'])
        self.assertEqual(value, before)


if __name__ == '__main__':
    unittest.main()
