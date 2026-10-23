#!/usr/bin/env python3
"""Check actual signed subscription completion/replay frames from the local fixture.

The producing SDK verifies signatures. This checks captured schema, scope,
correlation and immutable replay payloads, not independent cryptography.
"""
import argparse
import json
from pathlib import Path

from jsonschema import Draft202012Validator, FormatChecker

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("observations", type=Path)
parser.add_argument("--baseline", type=Path)
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
wire = Draft202012Validator(json.loads(
    (root / "contracts/schemas/uctp-envelope-2.schema.json").read_text()),
    format_checker=FormatChecker())
rows = [json.loads(line) for line in args.observations.read_text().splitlines()]
assert len(rows) == 36
requests, scenes, ids = {}, {}, set()
for row in rows:
    assert row["command"] == "stream.subscribe" and row["expected_revision"] == 1
    assert row["payload"] == {"subscriptions": [{"strm_id": "telephone-audio"}]}
    assert row["operation_id"] == "initial-select-phone"
    frame = row["frame"]
    wire.validate(frame)
    assert frame["type"] in ("operation.updated", "ack")
    assert frame["operation_id"] == row["operation_id"]
    assert frame["sid"] == "native-session" and frame["connid"] == "initial-native"
    algorithm = frame["signature"]["alg"]
    assert algorithm in ("EdDSA", "ES256")
    assert frame["id"] not in ids
    ids.add(frame["id"])
    key = algorithm, frame["cid"], frame["in_reply_to"]
    group = requests.setdefault(key, {})
    assert frame["type"] not in group
    group[frame["type"]] = frame
    if frame["type"] == "operation.updated":
        payload = frame["payload"]
        assert payload["state"] == "completed" and payload["revision"] == 3
        assert payload["result"] == {
            "connection_id": "initial-native", "resources_changed": False,
            "stream_revision": 2,
        }
        scenes.setdefault((algorithm, frame["cid"]), []).append(payload)
    else:
        assert frame["payload"]["details"]["reply_complete"] is True
assert len(requests) == 18 and len(scenes) == 10
assert all(set(group) == {"operation.updated", "ack"} for group in requests.values())
for algorithm in ("EdDSA", "ES256"):
    groups = [group for (alg, _), group in scenes.items() if alg == algorithm]
    assert len(groups) == 5
    assert sorted(map(len, groups)) == [1, 2, 2, 2, 2]
    assert all(all(payload == group[0] for payload in group) for group in groups)
if args.baseline:
    before = [json.loads(line) for line in args.baseline.read_text().splitlines()]
    assert len(before) == 1
    frame = before[0]["frame"]
    wire.validate(frame)
    assert frame["type"] == "operation.updated" and frame["payload"]["state"] == "unknown"
print(json.dumps({
    "passed": True, "actual_frames": len(rows), "completed_responses": 18,
    "signed_request_reply_pairs": len(requests), "scenes": len(scenes),
    "algorithms": ["EdDSA", "ES256"], "immutable_replay_scenes": 8,
    "scope": "Local signed native subscription to an initial SIP source under controlled binding contention and bounded setup/replay rehearsals.",
    "independent_signature_verification": False,
    "full_subscription_or_reliability_qualification": False,
}))
