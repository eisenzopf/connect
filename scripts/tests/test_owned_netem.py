"""Privileged fixture setup must refuse host or foreign queue ownership."""
from pathlib import Path
import runpy
import unittest
from unittest.mock import patch

NETEM=runpy.run_path(str(Path(__file__).resolve().parents[1]/'lib/owned_netem.py'))['OwnedNetem']


class NetemOwnership(unittest.TestCase):
    def owned(self):
        with patch('platform.system',return_value='Linux'),patch.object(Path,'is_file',return_value=True),\
                patch.dict('os.environ',{'CONNECT_GATE6_OWNED_BROWSER_CONTAINER':'1'}):
            return NETEM(25000,25007)

    def test_host_is_refused_before_any_network_command(self):
        with patch('platform.system',return_value='Linux'),patch.object(Path,'is_file',return_value=False),\
                patch.object(NETEM,'command') as command:
            with self.assertRaises(AssertionError):NETEM(25000,25007)
            command.assert_not_called()

    def test_existing_operator_queue_is_never_replaced(self):
        netem=self.owned()
        with patch.object(netem,'qdiscs',return_value=[{'kind':'fq_codel','handle':'123:'}]),\
                patch.object(netem,'command') as command:
            with self.assertRaises(AssertionError):netem.__enter__()
            command.assert_not_called()
            self.assertFalse(netem.installed)

    def test_cleanup_refuses_a_foreign_root_after_ownership_changed(self):
        netem=self.owned();netem.installed=True
        with patch.object(netem,'qdiscs',return_value=[{'kind':'fq_codel','handle':'1:'}]),\
                patch.object(netem,'command') as command:
            with self.assertRaises(AssertionError):netem.close()
            command.assert_not_called()


if __name__=='__main__':unittest.main()
