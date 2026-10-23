#!/usr/bin/env python3
"""Validate captured controlled Store DTMF summaries; not physical/signature conformance."""
import argparse
import json
import pathlib

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("samples", type=pathlib.Path)
    parser.add_argument("--require-count", type=int, default=5)
    args = parser.parse_args()
    raw = args.samples.read_bytes()
    assert len(raw) <= 1024 * 1024, "sample file exceeds the controlled profile"
    samples = [json.loads(line) for line in raw.splitlines() if line.strip()]
    assert len(samples) == args.require_count
    contract = json.loads((ROOT / "contracts/openapi-operations.json").read_text())
    validator = Draft202012Validator(contract["components"]["schemas"]["NativeDtmfSubmission"])
    states = set()
    mixed = False
    for record in samples:
        state = record["state"]
        states.add(state)
        signal = record["result"] if state == "completed" else record["problem"]["signal"]
        validator.validate(signal)
        assert signal["signal_source"] == "uctp-native"
        assert signal["source_observation_id"] == "actual-native-observation"
        assert signal["local_submission_only"] and signal["local_work_complete"]
        recipients = signal["recipients"]
        mixed |= len(recipients) == 2
        if state == "completed":
            assert all(r["disposition"] == "local_submitted" for r in recipients)
        elif state == "unknown":
            assert any(r["disposition"] == "outcome_unknown" for r in recipients)
        if state == "cancelled":
            assert all(r["disposition"] == "not_started" and r["frame_id"] is None for r in recipients)
    assert states == {"completed", "failed", "unknown", "cancelled"}
    assert mixed, "missing actual mixed native/SIP Store aggregate"
    print(json.dumps({"passed": True, "controlled_store_samples": len(samples), "states": sorted(states),
                      "physical_delivery_qualified": False, "signature_conformance_qualified": False}))


if __name__ == "__main__":
    main()
