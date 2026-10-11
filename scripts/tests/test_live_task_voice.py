"""Offline rejection/recovery checks; never contact a provider or model account."""
import hashlib
import importlib.util
import io
import json
import pathlib
import sys
import tempfile
import unittest
import wave
from types import SimpleNamespace
from unittest.mock import MagicMock, patch

LIVE = pathlib.Path(__file__).resolve().parents[1] / "live"
sys.path.insert(0, str(LIVE))
from task_voice import TaskVoice, require_task_config
from voice_runtime import load, save
from voice_scenario import load_scenario


class Client:
    def __init__(self, response):
        self.response = response
        self.calls = []
        self.conversation = "conversation"

    def negotiate_task(self, task):
        self.calls.append(("negotiate", task))
        return {"conversation_id": self.conversation}

    def task(self, command):
        self.calls.append(command)
        return self.response


def complete(**session):
    return {"kind": "tool", "data": {"status": "completed", "task_id": "task",
            "outcome": {"kind": "session", "data": {"id": "session",
            "conversation_id": "conversation", "participant_id": "peer",
            "state": "accepted", "media": ["audio"], **session}}}}


class TaskVoiceTests(unittest.TestCase):
    def setUp(self):
        self.intent = {"inspect": "inspect-once", "end": "end-once"}
        self.voice = TaskVoice("task", "conversation", "session", "peer", self.intent)

    def test_scoped_inspection_reuses_intent_and_never_invites(self):
        client = Client(complete())
        self.assertEqual(self.voice.bind(client)["id"], "session")
        self.voice.bind(client)
        commands = [call for call in client.calls if isinstance(call, dict)]
        self.assertEqual(commands[0], commands[1])
        self.assertEqual(commands[0]["operation"]["op"], "session_get")
        self.assertEqual(commands[0]["task_id"], "task")
        self.assertEqual(commands[0]["request_id"], self.intent["inspect"])

    def test_other_conversation_does_not_inspect_a_session(self):
        client = Client(complete())
        client.conversation = "other"
        with self.assertRaises(ValueError):
            self.voice.bind(client)
        self.assertEqual(client.calls, [("negotiate", "task")])

    def test_pending_wrong_resource_and_running_leg_fail_closed(self):
        cases = [complete(id="other"), complete(participant_id="other"),
                 complete(conversation_id="other"), complete(state="ready"), complete(media=["video"]),
                 {"kind": "tool", "data": {"status": "pending", "task_id": "task"}}]
        for response in cases:
            with self.subTest(response=response), self.assertRaises(ValueError):
                self.voice.bind(Client(response))

    def test_uncertain_end_is_not_retried_or_followed_by_recall(self):
        client = Client({"kind": "tool", "data": {"status": "uncertain", "task_id": "task"}})
        with self.assertRaises(ValueError):
            self.voice.finish(client)
        self.assertEqual(len(client.calls), 2)
        command = client.calls[1]
        self.assertEqual(command["request_id"], command["operation"]["request_id"])
        self.assertEqual(command["request_id"], "end-once")

    def test_end_must_confirm_the_original_session_has_ended(self):
        for response in [complete(), complete(id="other", state="ended")]:
            client = Client(response)
            with self.subTest(response=response), self.assertRaises(ValueError):
                self.voice.finish(client)
            self.assertEqual(len(client.calls), 2)
        client = Client(complete(state="ended"))
        self.voice.finish(client)
        self.assertEqual(client.calls[-1], {"action": "recall", "task_id": "task", "limit": 100})

    def test_backing_credentials_public_files_and_symlinks_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "client.toml"
            path.write_text('credential_id = "operator"\n')
            path.chmod(0o600)
            with self.assertRaises(ValueError):
                require_task_config(path)
            path.write_text('credential_id = "connect-task-grant:00000000-0000-0000-0000-000000000001"\n')
            require_task_config(path)
            path.chmod(0o644)
            with self.assertRaises(ValueError):
                require_task_config(path)
            path.chmod(0o600)
            link = path.with_name("link.toml")
            link.symlink_to(path)
            with self.assertRaises(OSError):
                require_task_config(link)

    def test_failed_record_publication_preserves_original_intent(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "intent.json"
            save(path, self.intent)
            with patch("voice_runtime.os.replace", side_effect=OSError("interrupted")):
                with self.assertRaises(OSError):
                    save(path, {"end": "must-not-publish"})
            self.assertEqual(json.loads(path.read_text()), self.intent)
            self.assertEqual(path.stat().st_mode & 0o077, 0)
            self.assertTrue(path.with_name("intent.json.tmp").exists())

    def test_scenario_bounds_are_checked_before_encoding(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            with wave.open(str(root / "audio.wav"), "wb") as wav:
                wav.setparams((1, 2, 8000, 0, "NONE", "not compressed"))
                wav.writeframes(b"\0\0" * 160)
            path = root / "scenario.json"
            for turns in [[], [{"wav": "audio.wav", "wait_seconds": True}],
                          [{"wav": "audio.wav", "wait_seconds": float("nan")}],
                          [{"wav": "audio.wav", "wait_for_reply": "true"}],
                          [{"wav": "audio.wav"}] * 7]:
                path.write_text(json.dumps({"turns": turns}))
                with self.subTest(turns=turns), self.assertRaises(ValueError):
                    load_scenario(path, lambda _: self.fail("must reject before encoding"))
            path.write_text(json.dumps({"turns": [{"wav": "audio.wav", "wait_seconds": 0.1}]}))
            prompts = load_scenario(path, lambda raw: b"\xff" * (len(raw) // 2))
            self.assertEqual(prompts, [([b"\xff" * 160], 5, False)])
            path.write_text(json.dumps({"turns": [{"wav": "audio.wav", "wait_seconds": 20}] * 6}))
            with self.assertRaisesRegex(ValueError, "120-second"):
                load_scenario(path, lambda raw: b"\xff" * (len(raw) // 2))

    def test_installed_tampering_fails_before_sdk_import(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            names = ["bin/connect", "sdk/python/connect_sdk/__init__.py", "examples/conversation-client/pcmu.py"]
            for name in names:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("raise AssertionError('must not import')\n")
            manifest = {"files": {name: hashlib.sha256((root / name).read_bytes()).hexdigest() for name in names}}
            (root / "manifest.json").write_text(json.dumps(manifest))
            (root / "bin/connect").write_text("tampered")
            with self.assertRaisesRegex(ValueError, "differs from its manifest"):
                load(root, root, None)

    def test_recording_failure_still_ends_the_session_and_records_failure(self):
        specification = importlib.util.spec_from_file_location("voice_driver", LIVE / "native-voice.py")
        driver = importlib.util.module_from_spec(specification)
        specification.loader.exec_module(driver)
        client = MagicMock()
        client.__enter__.return_value = client
        client.session_invite.return_value = {"id": "original-session"}
        client.context_view.return_value = {"messages": []}
        runtime = SimpleNamespace(client=MagicMock(return_value=client),
                                  audio=MagicMock(side_effect=RuntimeError("media unavailable")),
                                  encode=lambda _: b"", decode=lambda _: b"", binary="unused",
                                  evidence={"runtime": "unit-fixture"})
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory).resolve()
            output = root / ".local/capture"
            argv = ["native-voice.py", "--allow-live-provider", "--config", "human.toml",
                    "--assistant-config", "peer.toml", "--conversation", "conversation",
                    "--assistant", "peer", "--scenario", "unused.json", "--output", str(output)]
            # main applies a private umask; restore the test runner's own setting.
            import os
            previous = os.umask(0o077)
            try:
                with patch.object(driver, "ROOT", root), patch.object(driver, "load", return_value=runtime), \
                     patch.object(driver, "load_scenario", return_value=[]), \
                     patch.object(driver.wave, "open", side_effect=OSError("disk write failed")), \
                     patch.object(sys, "argv", argv), patch.object(sys, "stdout", io.StringIO()):
                    self.assertEqual(driver.main(), 1)
            finally:
                os.umask(previous)
            client.session_end.assert_called_once()
            self.assertEqual(client.session_end.call_args.args[2], "original-session")
            result = json.loads((output / "result.json").read_text())
            self.assertEqual(result["result"], "failed")
            self.assertEqual(result["recording_error_class"], "OSError")
            self.assertTrue((output / "context.json").exists())


if __name__ == "__main__":
    unittest.main()
