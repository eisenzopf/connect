"""Opt-in native SDK intake against separate local server/CLI processes."""
import copy
import json
import os
from pathlib import Path
import sys
import tempfile
import time
import tomllib
import unittest

from installation_paths import SOURCE as ROOT, RUNTIME, verify_imports
sys.path.insert(0, str(ROOT / "scripts"))
sys.path.insert(0, str(RUNTIME / "examples/agent-presence"))
from lib.local_process import Server, port, run
from connect_sdk import NativeClient
from incoming.intake import Policy, ensure_task
from incoming.state import Ledger


class PublicIntakeTests(unittest.TestCase):
    def test_native_incoming_task_survives_restart_and_replays_one_intent(self):
        binary = Path(os.environ["CONNECT_TEST_BINARY"]).resolve()
        verify_imports(binary)
        with tempfile.TemporaryDirectory(prefix="connect-incoming-") as root:
            root = Path(root)
            state = root / "server"
            run("init", "--directory", state, "--example", "review-loop", "--bind", f"127.0.0.1:{port()}", binary=binary)
            operator = tomllib.loads((state / "operator.toml").read_text())
            peer_id = tomllib.loads((state / "agent.toml").read_text())["identity_id"]
            run("issue", "--config", state / "operator.toml", "--server-config", state / "server.toml", "--identity-id", peer_id,
                "--recipients", operator["identity_id"], "--output", state / "human.toml", binary=binary)
            policy = Policy({"contacts": {peer_id: "connect:agent@local"},
                             "operations": ["send", "history", "inbox", "task_cancel"], "task_ttl_seconds": 600})
            binding = {"identity_id": operator["identity_id"], "policy": policy.value}
            ledger = Ledger(root / "intake", binding)
            env = {key: os.environ[key] for key in ("PATH", "HOME", "LANG", "TMPDIR") if key in os.environ}
            try:
                with Server(state, binary=binary, env=env):
                    with NativeClient(state / "operator.toml", executable=str(binary)) as owner, NativeClient(state / "human.toml", executable=str(binary)) as human:
                        conversation = owner.create("incoming-shared-conversation", [peer_id])
                        body = json.dumps({"action": "contact_assistant", "request_id": "first-contact", "body": "Please review my proposal"})
                        human.send("contact-message-1", conversation["id"], body)
                        ledger.stage(owner.inbox(), policy.classify, int(time.time()))
                        job = ledger.pending()
                        original_command = copy.deepcopy(job["command"])
                        task = ensure_task(owner, ledger, job, policy)
                        self.assertEqual(task["conversation_id"], conversation["id"])
                        self.assertEqual(task["initiating_event_id"], job["source"]["event_id"])
                ledger.close()
                ledger = Ledger(root / "intake", binding)
                with Server(state, binary=binary, env=env):
                    with NativeClient(state / "operator.toml", executable=str(binary)) as owner, NativeClient(state / "human.toml", executable=str(binary)) as human:
                        replay = ensure_task(owner, ledger, ledger.pending(), policy)
                        self.assertEqual(replay["id"], task["id"])
                        self.assertEqual(ledger.pending()["command"], original_command)
                        human.send("contact-message-2", conversation["id"], body)
                        ledger.stage(owner.inbox(cursor=ledger.get("cursor")), policy.classify, int(time.time()))
                        self.assertEqual(ledger.db.execute("SELECT COUNT(*) FROM jobs").fetchone()[0], 1)
                        cancelled = owner.task({"action": "cancel", "request_id": "cancel-incoming", "task_id": task["id"], "reason": "local check"})
                        self.assertEqual(cancelled["kind"], "record")
                        self.assertEqual(owner.negotiate_task(task["id"])["state"], "cancelled")
                        # Read current terminal state instead of retrying a now
                        # forbidden creation or launching another model turn.
                        closed = ensure_task(owner, ledger, ledger.pending(), policy)
                        self.assertEqual(closed["state"], "cancelled")
                        self.assertEqual(closed["id"], task["id"])
            finally:
                ledger.close()


if __name__ == "__main__":
    unittest.main()
