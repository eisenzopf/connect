"""Completed host checkpoint must survive a lost app ACK without a second turn."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from incoming.host import Host, save
from incoming.state import private_directory


class HostCheckpointTests(unittest.TestCase):
    def test_completed_checkpoint_does_not_dispatch_another_host_turn(self):
        with tempfile.TemporaryDirectory() as root:
            directory = private_directory(Path(root) / "job")
            installation = private_directory(directory / "host")
            session = str(uuid.uuid4())
            save(installation / "host-state.json", {"profile": "connect-codex-host/1", "completed": True, "task_id": "original", "session_id": session})
            host = Host("must-not-run", "must-not-run", "owner.toml", "127.0.0.1:1", "127.0.0.1:2")
            result = host.run(directory, {"id": "original"})
            self.assertEqual(result["session_id"], session)
            with self.assertRaises(ValueError):
                host.run(directory, {"id": "different"})

    def test_saved_intent_cannot_be_replaced_or_read_through_a_symlink(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "issue.json"
            save(path, {"request_id": "original"})
            save(path, {"request_id": "original"})
            with self.assertRaises(ValueError):
                save(path, {"request_id": "replacement"})
            self.assertEqual(json.loads(path.read_text())["request_id"], "original")
            alias = Path(root) / "alias"
            alias.symlink_to(path)
            with self.assertRaises(ValueError):
                save(alias, {"request_id": "original"})
