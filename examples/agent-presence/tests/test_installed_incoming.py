"""Explicit local installed-runtime qualification of the delivered intake command."""
import contextlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import uuid
import tomllib
import unittest

from installation_paths import SOURCE as ROOT, RUNTIME, verify_imports
sys.path.insert(0, str(ROOT / "scripts"))
sys.path.insert(0, str(RUNTIME / "examples/agent-presence"))
from lib.local_process import Server, port, run
from connect_sdk import NativeClient
from incoming_runtime_peer import Peer


class InstalledIncomingTests(unittest.TestCase):
    def test_independent_contact_reaches_actual_packaged_host_and_checkpoint_prevents_second_turn(self):
        binary = Path(os.environ["CONNECT_TEST_BINARY"]).resolve()
        verify_imports(binary)
        codex = Path(os.environ["CONNECT_TEST_CODEX_BINARY"]).resolve()
        evidence = Path(os.environ.get("CONNECT_TEST_EVIDENCE_DIR", ROOT / ".local/incoming-runtime")) / str(uuid.uuid4())
        evidence.mkdir(mode=0o700, parents=True)
        with contextlib.nullcontext(evidence) as directory:
            root = Path(directory)
            state, app = root / "server", root / "app"
            run("init", "--directory", state, "--example", "review-loop", "--bind", f"127.0.0.1:{port()}", binary=binary)
            mcp_address, http_address = f"127.0.0.1:{port()}", f"127.0.0.1:{port()}"
            with (state / "server.toml").open("a") as config:
                config.write(f'\n[mcp]\nbind = "{mcp_address}"\n')
            operator = tomllib.loads((state / "operator.toml").read_text())
            peer_id = tomllib.loads((state / "agent.toml").read_text())["identity_id"]
            run("issue", "--config", state / "operator.toml", "--server-config", state / "server.toml", "--identity-id", peer_id,
                "--recipients", operator["identity_id"], "--output", state / "human.toml", binary=binary)
            policy = root / "policy.json"
            policy.write_text(json.dumps({"contacts": {peer_id: "connect:agent@local"},
                                         "operations": ["send", "history", "inbox", "task_cancel"], "task_ttl_seconds": 600}))
            peer = Peer()
            try:
                shim = root / "offline-runtime.py"
                source = (ROOT / "crates/connect-server/tests/fixtures/codex-packaged-runtime.py").read_text()
                self.assertFalse(any(char.isspace() for char in sys.executable))
                # Use the selected test interpreter directly. PATH/env lookup is
                # not part of the installed Codex/Connect behavior being qualified.
                shim.write_text("#!" + sys.executable + "\n" + source.split("\n", 1)[1])
                shim.chmod(0o700)
                for home in ("codex-home", "user-home"):
                    (root / home).mkdir(mode=0o700)
                shim.with_suffix(".json").write_text(json.dumps({"binary": str(codex), "provider": peer.url,
                                                               "codex_home": str(root / "codex-home"), "user_home": str(root / "user-home")}))
                env = {key: os.environ[key] for key in ("PATH", "HOME", "LANG", "TMPDIR") if key in os.environ}
                env["PYTHONPATH"] = str(RUNTIME / "sdk/python")
                command = [sys.executable, str(RUNTIME / "examples/agent-presence/codex-inbox.py"),
                           "--config", str(state / "operator.toml"), "--policy", str(policy), "--state", str(app),
                           "--http-address", http_address, "--mcp-address", mcp_address,
                           "--executable", str(binary), "--codex-binary", str(shim)]
                def intake():
                    result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=180)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    return json.loads(result.stdout)
                with Server(state, binary=binary, env=env, extra_args=("--http-bind", http_address)):
                    with NativeClient(state / "operator.toml", executable=str(binary)) as owner, NativeClient(state / "human.toml", executable=str(binary)) as human:
                        cid = owner.create("incoming-conversation", [peer_id])["id"]
                        message = human.send("first-contact", cid, json.dumps({"action": "contact_assistant", "request_id": "incoming-001", "body": "Please confirm indigo-927 for the release."}))
                        owner.close()
                        human.close()
                    result = intake()
                    with NativeClient(state / "operator.toml", executable=str(binary)) as owner, NativeClient(state / "human.toml", executable=str(binary)) as human:
                        self.assertEqual(result["status"], "host-turn-completed")
                        self.assertEqual(result["conversation_id"], cid)
                        self.assertEqual(result["continuation"], "explicit-resume")
                        history = human.history(cid)
                        self.assertEqual(len(history["messages"]), 2)
                        self.assertEqual(history["messages"][1]["body"], f"Received from {message['message_id']}: Please confirm indigo-927 for the release.")
                        task = owner.negotiate_task(result["task_id"])
                        self.assertEqual(task["conversation_id"], cid)
                        self.assertEqual(len(peer.requests), 2)
                        self.assertEqual(peer.errors, [])
                        with sqlite3.connect(app / "intake.sqlite3") as db:
                            # Reproduce the app state just before the completion
                            # ACK; the packaged host's durable checkpoint remains.
                            db.execute("UPDATE jobs SET disposition='pending'")
                        replay = intake()
                        self.assertEqual(replay["session_id"], result["session_id"])
                        self.assertEqual(len(peer.requests), 2)
                        self.assertEqual(human.history(cid), history)
                        owner.task({"action": "cancel", "request_id": "cancel-contact", "task_id": task["id"], "reason": "local qualification"})
                        with sqlite3.connect(app / "intake.sqlite3") as db:
                            db.execute("UPDATE jobs SET disposition='pending'")
                        self.assertEqual(intake()["status"], "task-closed")
                        self.assertEqual(len(peer.requests), 2)
                        self.assertEqual(len(human.history(cid)["messages"]), 2)
                        grant = json.loads((Path(result["installation"]) / "task-grant.json").read_text())
                        wire = json.dumps(peer.requests)
                        self.assertNotIn(operator["credential"], wire)
                        self.assertNotIn(grant["grant"]["bearer"], wire)
            finally:
                peer.close()


if __name__ == "__main__":
    unittest.main()
