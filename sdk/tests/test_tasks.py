"""Task wire validation and public SDK negotiation, without provider accounts."""
import json
import pathlib
import unittest
from connect_sdk import ConnectError, NativeClient
from connect_sdk.protocol import decode_response
from connect_sdk.task_operations import exchange_timeout

HERE = pathlib.Path(__file__).resolve().parent
FIXTURES = json.loads((HERE / "task_results.json").read_text())


def wrap(value):
    return json.dumps({"profile": "connect-control/1", "result": {"Ok": {"kind": "task", "data": value}}})


class TaskTests(unittest.TestCase):
    def test_typed_result_families_and_legacy_optional_fields(self):
        for name, value in FIXTURES["valid"].items():
            with self.subTest(name=name):
                self.assertEqual(decode_response(wrap(value), "task")["data"], value)

    def test_malformed_optional_fields_states_and_nested_outcomes(self):
        for name, value in FIXTURES["invalid"].items():
            with self.subTest(name=name):
                with self.assertRaises(ConnectError) as error:
                    decode_response(wrap(value), "task")
                self.assertEqual(error.exception.code, "protocol")

    def test_additional_wait_is_bounded_and_not_applied_to_mutations(self):
        operation = {"op": "task", "request": {"command": {"action": "wait", "wait_ms": 1000}}}
        self.assertEqual(exchange_timeout(1, operation), 2)
        for value, expected in [(20000, 11), (-1, 1), (True, 1), ("1000", 1)]:
            operation["request"]["command"]["wait_ms"] = value
            self.assertEqual(exchange_timeout(1, operation), expected)
        self.assertEqual(exchange_timeout(1, {"op": "health"}), 1)

    def test_native_negotiation_stable_ids_and_pending_wait(self):
        with NativeClient("tasks", executable=str(HERE / "helper_fixture.py"), timeout=.4) as client:
            with self.assertRaises(ConnectError) as error:
                client.task({"action": "get", "task_id": "task-a"})
            self.assertEqual(error.exception.code, "unsupported-profile")
            client.negotiate_tasks("connect:agent@local", "conversation-a")
            pending = client.task({"action": "wait", "task_id": "task-a", "after": None, "limit": 1, "wait_ms": 500})
            self.assertEqual(pending["kind"], "inbox")
            self.assertEqual(pending["data"]["events"], [])
            command = {"action": "reply", "request_id": "stable-id", "conversation_id": "conversation-a", "reply_to": "question-a", "body": "Answer"}
            result = client.task(command)
            self.assertEqual(result["data"]["request_id"], "stable-id")
            self.assertEqual(command["request_id"], "stable-id")

    def test_old_server_is_unsupported_and_ordinary_operations_still_work(self):
        with NativeClient("tasks-old", executable=str(HERE / "helper_fixture.py")) as client:
            with self.assertRaises(ConnectError) as error:
                client.negotiate_tasks("connect:agent@local")
            self.assertEqual(error.exception.code, "unsupported-profile")
            self.assertEqual(client.health()["schema"], 7)

    def test_malformed_extension_cannot_negotiate(self):
        with NativeClient("tasks-malformed", executable=str(HERE / "helper_fixture.py")) as client:
            with self.assertRaises(ConnectError) as error:
                client.negotiate_tasks("connect:agent@local")
            self.assertEqual(error.exception.code, "protocol")
