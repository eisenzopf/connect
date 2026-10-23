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
    parser.add_argument("--failure-samples", type=pathlib.Path)
    parser.add_argument("--stop-samples", type=pathlib.Path)
    parser.add_argument("--maximum", action="store_true", help="Require the exact 30-second RFC4733 profile")
    args = parser.parse_args()
    raw = args.samples.read_bytes()
    assert len(raw) <= 1024 * 1024
    rows = [json.loads(line) for line in raw.splitlines() if line.strip()]
    assert len(rows) == (2 if args.maximum else 4)
    assert {(r["algorithm"], r["method"]) for r in rows} == {
        (algorithm, method)
        for algorithm in ("EdDSA", "ES256")
        for method in (("rfc4733",) if args.maximum else ("info", "rfc4733"))
    }
    contract = json.loads((ROOT / "contracts/openapi-operations.json").read_text())
    validator = Draft202012Validator(contract["components"]["schemas"]["NativeDtmfSubmission"])
    observations, schedules = set(), set()
    for row in rows:
        record = row["completed"]
        assert record["state"] == "completed"
        assert record["operation_id"] == "native-sip-" + row.get("profile", row["method"])
        if args.maximum:
            assert row["profile"] == "rfc4733-maximum"
            assert row["digits"] == "12345" and row["duration_ms"] == 5944
            assert len(row["digits"]) * row["duration_ms"] + 70 * (len(row["digits"]) - 1) == 30_000
            assert 30_000 <= row["elapsed_ms"] < 35_000
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
            assert len(row["rtp_events"]) == (5 if args.maximum else 3)
            assert {event for event, count in row["rtp_events"].values()} == ({1, 2, 3, 4, 5} if args.maximum else {5, 8, 11})
            assert all(count >= 3 for event, count in row["rtp_events"].values())
    failures = []
    if args.failure_samples:
        raw = args.failure_samples.read_bytes()
        assert len(raw) <= 1024 * 1024
        failures = [json.loads(line) for line in raw.splitlines() if line.strip()]
        assert len(failures) == 2
        assert {r["algorithm"] for r in failures} == {"EdDSA", "ES256"}
        for row in failures:
            record = row["stopped"]
            assert record["state"] == "unknown"
            assert record["operation_id"] == "native-sip-info-refusal"
            signal = record["problem"]["signal"]
            validator.validate(signal)
            assert signal["local_work_complete"] and signal["local_submission_only"]
            assert signal["signal_source"] == "uctp-native"
            assert signal["connection_id"] == "initial-native"
            assert row["sip_calls"] == row["info_packets"] == 1
            assert len(signal["recipients"]) == 1
            recipient = signal["recipients"][0]
            assert recipient["disposition"] == "outcome_unknown"
            assert recipient["frame_id"].startswith("dtmfg_")
            assert recipient["frame_id"] not in schedules
            schedules.add(recipient["frame_id"])
    stops = []
    if args.stop_samples:
        raw = args.stop_samples.read_bytes()
        assert len(raw) <= 1024 * 1024
        stops = [json.loads(line) for line in raw.splitlines() if line.strip()]
        assert len(stops) == 4
        assert {(r["algorithm"], r["mode"]) for r in stops} == {
            (algorithm, mode)
            for algorithm in ("EdDSA", "ES256")
            for mode in ("info-cancel", "info-source-revoke")
        }
        for row in stops:
            record = row["stopped"]
            assert record["state"] == "unknown"
            assert record["operation_id"] == "native-sip-" + row["mode"]
            signal = record["problem"]["signal"]
            validator.validate(signal)
            assert signal["local_work_complete"] and signal["local_submission_only"]
            assert signal["signal_source"] == "uctp-native"
            assert row["sip_calls"] == row["info_packets"] == 1
            assert len(signal["recipients"]) == 1
            recipient = signal["recipients"][0]
            assert recipient["disposition"] == "outcome_unknown"
            assert recipient["frame_id"].startswith("dtmfg_")
            assert recipient["frame_id"] not in schedules
            schedules.add(recipient["frame_id"])
    print(json.dumps({
        "passed": True, "actual_sdk_summaries": len(rows),
        "actual_uncertain_sdk_summaries": len(failures),
        "actual_cancelled_or_revoked_summaries": len(stops),
        "maximum_sequence_profile": args.maximum,
        "algorithms": ["EdDSA", "ES256"], "methods": ["info", "rfc4733"],
        "scope": "Local macOS arm64 native-to-initial-SIP positive summaries and actual UDP observations.",
        "signature_verification": "The producing SDK verifies signatures; this validator checks recorded DTOs and HTTP/OpenAPI schema.",
        "full_dtmf_clause_qualification": False,
    }))


if __name__ == "__main__":
    main()
