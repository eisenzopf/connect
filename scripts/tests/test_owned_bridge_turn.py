"""TURN preparation must reject external network or filesystem custody."""
from pathlib import Path
import json
import runpy
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[2]
TURN=runpy.run_path(str(ROOT/'scripts/lib/owned_bridge_turn.py'))['OwnedBridgeTurn']


class TurnOwnership(unittest.TestCase):
    def fixture(self,path=None):
        return TURN('a'*64,'sha256:'+'b'*64,'sha256:'+'c'*64,'172.30.0.1',
            path or ROOT/'.local/gate-6/turn-ownership-uncreated','udp')

    def test_private_state_cannot_be_created_outside_the_owned_local_directory(self):
        with self.assertRaises(AssertionError):self.fixture(ROOT/'private-turn-uncreated')

    def test_default_or_named_network_is_never_accepted_as_owned_identity(self):
        with self.assertRaises(AssertionError):
            TURN('bridge','sha256:'+'b'*64,'sha256:'+'c'*64,'172.30.0.1',ROOT/'.local/turn-uncreated','udp')

    def test_external_network_is_refused_before_container_or_private_file_creation(self):
        turn=self.fixture();calls=[]
        def command(args,**kwargs):
            calls.append(args);return json.dumps([{'Id':'a'*64,'Internal':False,'Driver':'bridge'}])
        with patch.dict(turn.start.__globals__,{'command':command}):
            with self.assertRaises(AssertionError):turn.start()
        self.assertEqual(calls,[['docker','network','inspect','a'*64]])
        self.assertIsNone(turn.cid)


if __name__=='__main__':unittest.main()
