"""Projected record references retain source boundaries across SDK decoding."""
import json
import unittest
from connect_sdk import ConnectError, ContextRecord
from connect_sdk.protocol import decode_response


class RecordContextTests(unittest.TestCase):
    def responses(self, record):
        body = '{"uuid":"record-one"}' if record["state"] == "available" else None
        common = dict(source_id="record-one", kind="record_ready", actor_id="owner", record=record)
        yield "context_view", dict(conversation_id="conversation-one", watermark=4,
            items=[dict(common, sequence=4, body=body or "")], omitted=0,
            deliveries=[], deliveries_omitted=0)
        yield "task", dict(kind="recall", data=dict(sources=[dict(common,
            created_at=1792000000, provenance="conversation-journal:event-one;current-authorized-record-projection",
            state=record["state"], body=body)], next=None, omitted=0))

    def decode(self, kind, data):
        return decode_response(json.dumps(dict(profile="connect-control/1", result=dict(Ok=dict(kind=kind, data=data)))), kind)

    def reference(self):
        return ContextRecord(uuid="record-one", session_id="session-one", revision=1,
            state="available", content_hash="sha512-projection", valid_until=1793000000)

    def test_available_unavailable_and_deleted_keep_truthful_references(self):
        for state in ("available", "unavailable", "deleted"):
            record = dict(self.reference(), state=state)
            if state != "available":
                record["content_hash"] = None
            for kind, data in self.responses(record):
                with self.subTest(state=state, kind=kind):
                    self.assertEqual(self.decode(kind, data)["data"], data)

    def test_private_fingerprints_and_foreign_identity_are_rejected(self):
        for fields in (
            dict(body_hash="private-whole-record-hash"),
            dict(manifest_hash="private-manifest-hash"),
            dict(uuid="foreign-record"), dict(revision=0), dict(revision=True),
            dict(revision=9007199254740992), dict(state="deleted"),
            dict(content_hash=None), dict(valid_until="tomorrow"), dict(valid_until=-1),
        ):
            for kind, data in self.responses(dict(self.reference(), **fields)):
                with self.subTest(fields=fields, kind=kind):
                    with self.assertRaises(ConnectError) as caught:
                        self.decode(kind, data)
                    self.assertEqual(caught.exception.code, "protocol")
