#!/usr/bin/env python3
"""Validate actual local moved-SIP summaries; no remote playback claim."""
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
    assert len(rows) == 6
    assert {(r["algorithm"], r["method"]) for r in rows} == {(a,m) for a in ("EdDSA","ES256") for m in ("info","rfc4733","missing-receive")}
    contract = json.loads((ROOT / "contracts/openapi-operations.json").read_text())
    validator = Draft202012Validator(contract["components"]["schemas"]["NativeDtmfSubmission"])
    schedules, observations = set(), set()
    for row in rows:
        assert row["sip_calls"] == 1
        if row["method"] == "missing-receive":
            assert row["refused"]["state"] == "failed" and row["info_packets"] == 0
            continue
        done = row["completed"]
        assert done["state"] == "completed"
        assert done["operation_id"] == "native-to-moved-sip-" + row["method"]
        result = done["result"]
        validator.validate(result)
        assert result["signal_source"] == "uctp-native" and result["connection_id"] == "codec-source"
        assert result["resources_changed"] is False and result["local_submission_only"] and result["local_work_complete"]
        assert datetime.datetime.fromisoformat(result["received_at"].replace("Z", "+00:00")).tzinfo is not None
        assert len(result["recipients"]) == 1
        recipient = result["recipients"][0]
        target = row["target"]
        assert recipient["connection_id"] == target["connection_id"]
        assert recipient["generation"] == target["generation"] == 2
        assert recipient["disposition"] == "local_submitted"
        assert recipient["frame_id"].startswith("dtmfg_") and recipient["frame_id"] not in schedules
        schedules.add(recipient["frame_id"])
        assert result["source_observation_id"].startswith("env_") and result["source_observation_id"] not in observations
        observations.add(result["source_observation_id"])
        assert target["sip_call_id"] and target["route_credential_id"] and target["route_owner_id"] != target["identity_id"]
        assert not {"endpoint_credential_id","key_id","signing_key_id","endpoint_revision"}.intersection(target)
        if row["method"] == "info":
            assert row["info_packets"] >= 3 and row["rtp_events"] == {}
        else:
            assert len(row["rtp_events"]) == 3
            assert {event for event, count in row["rtp_events"].values()} == {5,8,11}
            assert all(count >= 3 for event,count in row["rtp_events"].values())
    print(json.dumps({"passed":True,"signing_scenes":2,"actual_completed_schedules":4,"named_receive_refusals":2,"http_summaries_validated":4,"remote_playback_qualified":False}))

if __name__ == "__main__":
    main()
