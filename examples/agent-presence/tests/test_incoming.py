"""Durable intake failure/recovery before the separate actual-host qualification."""
import copy
import json
import sqlite3
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from incoming.intake import Policy, ensure_task
from incoming.state import Ledger

POLICY = {"contacts": {"human": "connect:human@local"}, "operations": ["send", "history", "inbox", "task_cancel"], "task_ttl_seconds": 600}


def event(id="event-1", request="request-1", body="Please review the latest draft", sender="human"):
    return {"id": id, "kind": "message", "conversation_id": "shared", "cursor": id,
            "message": {"id": "message-" + id, "sender": sender, "conversation_id": "shared",
                        "body": json.dumps({"action": "contact_assistant", "request_id": request, "body": body})}}


class Client:
    """Public SDK double retaining the server acceptance before an ACK is lost."""
    def __init__(self):
        self.calls, self.record = [], None
        self.lose_ack, self.revoked = False, False

    def resolve(self, address):
        return {"id": "human"}

    def negotiate_tasks(self, *_):
        pass

    def task(self, command):
        self.calls.append(copy.deepcopy(command))
        if self.revoked:
            raise PermissionError("revoked")
        if self.record is None:
            self.record = {"id": "original-task", "owner_id": "assistant", "state": "ready", **command["definition"]}
        if self.lose_ack:
            self.lose_ack = False
            raise TimeoutError("accepted before lost ACK")
        return {"kind": "record", "data": self.record}

    def negotiate_task(self, task_id):
        if self.revoked:
            raise PermissionError("revoked")
        return self.record


class IntakeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.policy = Policy(copy.deepcopy(POLICY))
        self.binding = {"identity_id": "assistant", "policy": self.policy.value}
        self.ledger = Ledger(self.root / "state", self.binding)

    def tearDown(self):
        self.ledger.close()
        self.temp.cleanup()

    def stage(self, *events):
        self.ledger.stage({"events": list(events), "cursor": "next-page"}, self.policy.classify, 1000)

    def test_restart_replays_original_task_creation_after_lost_ack(self):
        self.stage(event())
        client = Client()
        client.lose_ack = True
        first = self.ledger.pending()
        with self.assertRaises(TimeoutError):
            ensure_task(client, self.ledger, first, self.policy)
        self.ledger.close()
        self.ledger = Ledger(self.root / "state", self.binding)
        recovered = ensure_task(client, self.ledger, self.ledger.pending(), self.policy)
        self.assertEqual(client.calls[0], client.calls[1])
        self.assertEqual(recovered["id"], "original-task")
        self.assertEqual(recovered["initiating_event_id"], "event-1")
        self.assertEqual(recovered["conversation_id"], "shared")
        self.assertEqual(recovered["scope"]["operations"], POLICY["operations"])
        self.assertEqual(self.ledger.get("cursor"), "next-page")

    def test_duplicate_and_changed_request_preserve_first_source(self):
        self.stage(event(), event("event-2"), event("event-3", body="Changed"))
        self.assertEqual(self.ledger.db.execute("SELECT COUNT(*) FROM jobs").fetchone()[0], 1)
        self.assertEqual(self.ledger.pending()["source"]["event_id"], "event-1")
        self.assertEqual([r[0] for r in self.ledger.db.execute("SELECT disposition FROM events ORDER BY rowid")], ["queued", "duplicate", "conflict"])

    def test_two_requests_share_context_without_merging_tasks(self):
        self.stage(event(), event("event-2", request="request-2"))
        commands = [json.loads(r[0]) for r in self.ledger.db.execute("SELECT command FROM jobs")]
        self.assertEqual(len(commands), 2)
        self.assertNotEqual(commands[0]["request_id"], commands[1]["request_id"])
        self.assertEqual(commands[0]["definition"]["conversation_id"], commands[1]["definition"]["conversation_id"])

    def test_page_rollback_does_not_lose_earlier_queued_work(self):
        broken = event("event-2")
        broken["message"]["conversation_id"] = "wrong"
        with self.assertRaises(ValueError):
            self.stage(event(), broken)
        self.assertIsNone(self.ledger.pending())
        self.assertIsNone(self.ledger.get("cursor"))
        self.assertEqual(self.ledger.db.execute("SELECT COUNT(*) FROM events").fetchone()[0], 0)

    def test_plain_reply_is_unassigned_and_unapproved_sender_is_denied(self):
        plain = event()
        plain["message"]["body"] = "That sounds good"
        self.stage(plain, event("event-2", sender="stranger"))
        self.assertIsNone(self.ledger.pending())
        self.assertEqual([r[0] for r in self.ledger.db.execute("SELECT disposition FROM events ORDER BY rowid")], ["unassigned", "denied"])

    def test_changed_journal_event_fails_without_advancing_cursor(self):
        self.stage(event())
        with self.assertRaises(ValueError):
            self.stage(event(body="Mutated retained event"))
        self.assertEqual(self.ledger.pending()["source"]["body"], "Please review the latest draft")

    def test_current_revocation_prevents_using_cached_task_receipt(self):
        self.stage(event())
        client = Client()
        ensure_task(client, self.ledger, self.ledger.pending(), self.policy)
        client.revoked = True
        with self.assertRaises(PermissionError):
            ensure_task(client, self.ledger, self.ledger.pending(), self.policy)
        self.assertIsNotNone(self.ledger.pending())

    def test_terminal_task_is_inspected_without_replaying_or_reopening_work(self):
        self.stage(event())
        client = Client()
        ensure_task(client, self.ledger, self.ledger.pending(), self.policy)
        client.record["state"] = "cancelled"
        current = ensure_task(client, self.ledger, self.ledger.pending(), self.policy)
        self.assertEqual(current["state"], "cancelled")
        self.assertEqual(len(client.calls), 1)

    def test_exclusive_worker_and_unchanged_policy(self):
        with self.assertRaises(BlockingIOError):
            Ledger(self.root / "state", self.binding)
        self.ledger.close()
        with self.assertRaises(ValueError):
            Ledger(self.root / "state", {"identity_id": "different"})
        self.ledger = Ledger(self.root / "state", self.binding)

    def test_symlink_and_public_directory_are_rejected(self):
        (self.root / "alias").symlink_to(self.root / "state")
        with self.assertRaises(ValueError):
            Ledger(self.root / "alias", self.binding)
        (self.root / "public").mkdir(mode=0o755)
        with self.assertRaises(ValueError):
            Ledger(self.root / "public", self.binding)

    def test_unknown_database_is_not_adopted_or_extended(self):
        directory = self.root / "unrelated"
        directory.mkdir(mode=0o700)
        database = directory / "intake.sqlite3"
        with sqlite3.connect(database) as db:
            db.execute("CREATE TABLE important(value TEXT)")
            db.execute("INSERT INTO important VALUES('preserve')")
        with self.assertRaises(ValueError):
            Ledger(directory, self.binding)
        with sqlite3.connect(database) as db:
            self.assertEqual(db.execute("SELECT value FROM important").fetchone()[0], "preserve")
            self.assertEqual(db.execute("SELECT name FROM sqlite_master WHERE type='table'").fetchall(), [("important",)])

    def test_invalid_policy_and_authority_in_contact_do_not_create_work(self):
        with self.assertRaises(ValueError):
            Policy({**POLICY, "operations": ["arbitrary"]})
        forged = event()
        body = json.loads(forged["message"]["body"])
        body["task_id"] = "other-task"
        forged["message"]["body"] = json.dumps(body)
        self.stage(forged)
        self.assertIsNone(self.ledger.pending())
