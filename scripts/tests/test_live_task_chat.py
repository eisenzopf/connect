"""Local chat rehearsal rejection and recovery; no model/provider accounts."""
import importlib.util
import json
import os
from pathlib import Path
from types import SimpleNamespace
import sys
import tempfile
import time
import unittest
from unittest.mock import MagicMock, patch

LIVE = Path(__file__).resolve().parents[1] / "live"
sys.path.insert(0, str(LIVE))
from chat_transport import identifier, origin
from voice_runtime import save

spec = importlib.util.spec_from_file_location("task_chat", LIVE / "task-chat.py")
chat = importlib.util.module_from_spec(spec)
spec.loader.exec_module(chat)


class ChatTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name).resolve()
        self.setup = self.root / "setup"
        self.setup.mkdir(mode=0o700)
        self.setup_value = {"profile": "connect-vapi-bootstrap/1", "nonce": "nonce",
                            "external_id": "session", "definition": {
                                "origin": "chat_session", "task_id": "task", "assistant_id": "assistant",
                                "expires_at": int(time.time()) + 300}}
        save(self.setup / "intent.json", self.setup_value)
        self.prompt = self.root / "prompt.txt"
        self.prompt.write_text("Recall the earlier decision through Connect and explain it.")
        self.config = self.root / "native.toml"
        self.config.write_text('credential_id="connect-task-grant:00000000-0000-0000-0000-000000000001"')
        self.config.chmod(0o600)
        self.args = SimpleNamespace(action="capture", output=self.root / ".local/run",
                                    config=self.config, setup=self.setup, task_id="task",
                                    runtime_root=None, executable=None, api_base="https://api.vapi.ai",
                                    prompt=self.prompt, chat_id=None)
        self.client = MagicMock()
        self.client.__enter__.return_value = self.client
        self.client.negotiate_task.return_value = {"conversation_id": "conversation"}
        self.client.task.return_value = {"kind": "recall", "data": {"entries": ["SOURCE-FACT"]}}
        self.runtime = SimpleNamespace(client=MagicMock(return_value=self.client), binary="unused",
                                       evidence={"runtime": "unit-fixture"})
        self.session = {"id": "session", "assistantId": "assistant", "status": "active",
                        "metadata": {"variableValues": {"connect_invocation_id": "nonce"}}}
        self.calls = []
        self.receipt = None
        previous = os.umask(0o077)
        self.addCleanup(os.umask, previous)

    def provider(self, base, method, route, body, directory, name):
        self.calls.append((method, route, body))
        if route.startswith("/session/"):
            return self.session
        if method == "POST":
            intent = json.loads((self.args.output / "intent.json").read_text())
            self.assertEqual(intent["state"], "dispatched")
            self.receipt = {"id": "chat-id", **body, "output": [{"role": "assistant", "content": "Answer"}]}
        return self.receipt

    def execute(self, request=None):
        with patch.object(chat, "ROOT", self.root), patch.object(chat, "load", return_value=self.runtime), \
             patch.object(chat, "request", side_effect=request or self.provider):
            return chat.execute(self.args)

    def test_one_turn_uses_existing_session_and_retains_context(self):
        result = self.execute()
        self.assertEqual(result["result"], "captured")
        self.assertEqual([x[:2] for x in self.calls], [("GET", "/session/session"), ("POST", "/chat")])
        body = self.calls[-1][2]
        self.assertEqual(set(body), {"sessionId", "input", "name", "stream"})
        self.assertEqual(body["sessionId"], "session")
        self.assertFalse(body["stream"])
        self.assertEqual(len(body["name"]), 40)
        after = list(self.args.output.glob("inspection-*/after.json"))
        self.assertEqual(len(after), 1)
        self.assertIn("SOURCE-FACT", after[0].read_text())
        with self.assertRaises(FileExistsError):
            self.execute()
        self.assertEqual(len(self.calls), 2)

    def test_lost_post_response_reconciles_exact_receipt_without_reposting(self):
        def lost(*args):
            response = self.provider(*args)
            if args[1] == "POST":
                raise TimeoutError("receipt lost")
            return response
        self.assertEqual(self.execute(lost)["result"], "uncertain")
        original = json.loads((self.args.output / "intent.json").read_text())
        self.args.action, self.args.prompt, self.args.chat_id = "reconcile", None, "chat-id"
        self.session["status"] = "expired"  # Inspection does not start another model turn.
        self.assertEqual(self.execute()["result"], "captured")
        self.assertEqual(sum(method == "POST" for method, _, _ in self.calls), 1)
        final = json.loads((self.args.output / "intent.json").read_text())
        self.assertEqual(final["name"], original["name"])
        self.assertEqual(final["session_id"], original["session_id"])

    def test_normalized_provider_input_reconciles_without_another_post(self):
        def lost(*args):
            response = self.provider(*args)
            if args[1] == "POST":
                self.receipt["input"] = [{"role": "user", "content": args[3]["input"]}]
                raise TimeoutError("receipt lost")
            return response
        self.assertEqual(self.execute(lost)["result"], "uncertain")
        self.args.action, self.args.prompt, self.args.chat_id = "reconcile", None, "chat-id"
        self.assertEqual(self.execute()["result"], "captured")
        self.assertEqual(sum(method == "POST" for method, _, _ in self.calls), 1)

    def test_normalized_input_cannot_change_the_original_turn(self):
        intent = {"session_id": "session", "name": "name", "input": "Original text"}
        receipt = {"id": "chat-id", "sessionId": "session", "name": "name",
                   "output": [{"role": "assistant", "content": "Answer"}]}
        for value in [[], [{"role": "assistant", "content": "Original text"}],
                      [{"role": "user", "content": "Changed text"}],
                      [{"role": "user", "content": "Original text", "extra": True}],
                      [{"role": "user", "content": "Original text"}] * 2]:
            with self.subTest(input=value), self.assertRaises(ValueError):
                chat.verify_chat({**receipt, "input": value}, intent)

    def test_revoked_task_denies_provider_access(self):
        self.client.negotiate_task.side_effect = ValueError("revoked")
        self.assertEqual(self.execute()["result"], "failed")
        self.assertEqual(self.calls, [])

    def test_expiry_after_observation_prevents_post(self):
        self.client.negotiate_task.side_effect = [{"conversation_id": "conversation"}, ValueError("expired")]
        self.assertEqual(self.execute()["result"], "failed")
        self.assertEqual([x[0] for x in self.calls], ["GET"])

    def test_wrong_nonce_and_delivery_sessions_fail_before_post(self):
        for key, value in [("metadata", {}), ("transport", {"phoneNumberId": "number"}),
                           ("customer", {"number": "+15555550101"}), ("status", "expired")]:
            with self.subTest(key=key):
                with self.assertRaises(ValueError):
                    chat.verify_session({**self.session, key: value}, {
                        "session_id": "session", "assistant_id": "assistant", "nonce": "nonce"})

    def test_wrong_chat_receipt_keeps_original_dispatch_uncertain(self):
        def wrong(*args):
            result = self.provider(*args)
            return {**result, "sessionId": "other"} if args[1] == "POST" else result
        self.assertEqual(self.execute(wrong)["result"], "uncertain")
        self.assertEqual(json.loads((self.args.output / "intent.json").read_text())["state"], "dispatched")

    def test_changed_setup_is_not_a_recovery_path(self):
        def lost(*args):
            self.provider(*args)
            if args[1] == "POST":raise TimeoutError()
            return self.session
        self.execute(lost)
        self.setup_value["nonce"] = "different"
        save(self.setup / "intent.json", self.setup_value)
        self.args.action, self.args.prompt, self.args.chat_id = "reconcile", None, "chat-id"
        with self.assertRaises(ValueError):self.execute()
        self.assertEqual(len(self.calls), 2)

    def test_oversized_prompt_is_rejected_before_network(self):
        self.prompt.write_bytes(b"a" * 16385)
        with self.assertRaises(ValueError):self.execute()
        self.assertEqual(self.calls, [])
        self.assertFalse(self.args.output.exists())

    def test_expired_setup_cannot_start_another_model_turn(self):
        self.setup_value["definition"]["expires_at"] = int(time.time()) - 1
        save(self.setup / "intent.json", self.setup_value)
        with self.assertRaisesRegex(ValueError, "expired"):
            self.execute()
        self.assertEqual(self.calls, [])
        self.assertFalse(self.args.output.exists())

    def test_setup_expiry_during_provider_inspection_prevents_post(self):
        expiry = self.setup_value["definition"]["expires_at"]
        with patch.object(chat.time, "time", return_value=expiry - 1) as clock:
            def observed(*args):
                result = self.provider(*args)
                clock.return_value = expiry
                return result
            self.assertEqual(self.execute(observed)["result"], "failed")
        self.assertEqual([method for method, _, _ in self.calls], ["GET"])

    def test_unsafe_origin_or_resource_is_rejected(self):
        for value in ["http://example.com", "https://user:key@example.com", "https://api.vapi.ai/path",
                      "https://api.vapi.ai?token=secret", "https://api.vapi.ai#selector"]:
            with self.subTest(value=value), self.assertRaises(ValueError):origin(value)
        for value in [None, "../session", "id?query", "a" * 129]:
            with self.subTest(value=value), self.assertRaises(ValueError):identifier(value)
        origin("http://127.0.0.1:1234")
        origin("https://api.vapi.ai/")


if __name__ == "__main__":
    unittest.main()
