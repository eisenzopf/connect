#!/usr/bin/env python3
"""Check actual local native-to-initial-SIP observations against the HTTP contract."""
import argparse
import datetime
import json
import pathlib

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("samples", type=pathlib.Path)
    args = parser.parse_args()
    raw = args.samples.read_bytes()
    assert len(raw) <= 1024 * 1024
    rows = [json.loads(line) for line in raw.splitlines() if line.strip()]
    assert len(rows) == 4
    assert {(r["algorithm"], r["method"]) for r in rows} == {
        (algorithm, method)
        for algorithm in ("EdDSA", "ES256")
        for method in ("info", "rfc4733")
    }
    contract = json.loads((ROOT / "contracts/openapi-operations.json").read_text())
    validator = Draft202012Validator(contract["components"]["schemas"]["NativeDtmfSubmission"])
    observations, schedules = set(), set()
    for row in rows:
        record = row["completed"]
        assert record["state"] == "completed"
        assert record["operation_id"] == "native-sip-" + row["method"]
        result = record["result"]
        validator.validate(result)
        assert result["signal_source"] == "uctp-native"
        assert result["connection_id"] == "initial-native"
        assert result["local_submission_only"] and result["local_work_complete"]
        assert result["resources_changed"] is False
        received = datetime.datetime.fromisoformat(result["received_at"].replace("Z", "+00:00"))
        assert received.tzinfo is not None
        assert len(result["recipients"]) == row["sip_calls"] == 1
        recipient = result["recipients"][0]
        assert recipient["connection_id"] != result["connection_id"]
        assert recipient["disposition"] == "local_submitted"
        assert recipient["frame_id"].startswith("dtmfg_")
        assert recipient["frame_id"] not in schedules
        schedules.add(recipient["frame_id"])
        assert result["source_observation_id"].startswith("env_")
        assert result["source_observation_id"] not in observations
        observations.add(result["source_observation_id"])
        if row["method"] == "info":
            assert row["info_packets"] >= 3 and row["rtp_events"] == {}
        else:
            assert row["info_packets"] == 0
            assert len(row["rtp_events"]) == 3
            assert {event for event, count in row["rtp_events"].values()} == {5, 8, 11}
            assert all(count >= 3 for event, count in row["rtp_events"].values())
    print(json.dumps({
        "passed": True, "actual_sdk_summaries": len(rows),
        "algorithms": ["EdDSA", "ES256"], "methods": ["info", "rfc4733"],
        "scope": "Local macOS arm64 native-to-initial-SIP positive summaries and actual UDP observations.",
        "signature_verification": "The producing SDK verifies signatures; this validator checks recorded DTOs and HTTP/OpenAPI schema.",
        "full_dtmf_clause_qualification": False,
    }))


if __name__ == "__main__":
    main()
