"""Requirement-level verdicts for an original G6-058 cell attempt.

Every contract requirement has an evaluator. A missing observation is a blocked
check, never a pass and never silently omitted. A failed programme must not erase
successful control observations from the same original attempt.
"""
import re

from .transport_matrix import REQUIRED_CHECKS, SELECTION_CHECKS


def result(passed, evidence, *, reason=None, category=None):
    return dict(status="passed" if passed else "failed", evidence=evidence,
                **({"reason": reason} if reason else {}),
                **({"category": category} if category and not passed else {}))


def blocked(reason, evidence=None, category="dependency"):
    return dict(status="blocked", reason=reason, category=category, evidence=evidence)


def evaluate_cell(cell, report, report_sha256, *, environment, revision):
    """Evaluate all 12 native / 14 browser requirements, without short circuit."""
    from .transport_matrix_execution import browser_selection_verified
    from .matrix_idle import idle_verdict

    if not re.fullmatch(r"[0-9a-f]{64}", report_sha256):
        raise ValueError("An original report digest is required")
    browser = cell["transport"] != "native-quic"
    required = REQUIRED_CHECKS | (SELECTION_CHECKS if browser else set())
    if set(cell["required_checks"]) != required:
        raise ValueError("Cell requirements differ from the full contract")
    observation = report.get("browser_observation", {}) if browser else report.get("conversation_control", {})
    checks = observation.get("admission", observation).get("checks", [])
    producer_checks = report.get("checks", [])
    guards = ("source_inputs_unchanged", "sdk_dist_inputs_unchanged",
              "executable_unchanged" if browser else "executables_unchanged")
    identity = (report.get("source_revision") == revision
        and (report.get("server_platform") if browser else report.get("host", {}).get("platform")) == cell["server"]
        and report.get("signing_algorithm") == cell["signing"]
        and (report.get("cell_id") == cell["id"] if browser else
             report.get("network_profile") == ("local" if cell["network"] == "local" else "remote")))
    integrity = identity and all(report.get(name) is True for name in guards)
    outcomes = {}

    def record(name, observed, passed, section, reason=None, category="behavior"):
        evidence = dict(report_sha256=report_sha256, section=section)
        if not observed:
            outcomes[name] = blocked(reason or "prerequisite-did-not-produce-observation", evidence)
        else:
            outcomes[name] = result(integrity and bool(passed), evidence,
                reason="candidate-or-integrity-mismatch" if not integrity else reason,
                category="integrity" if not integrity else category)

    versions = report.get("runtime_versions", {})
    record("identified-candidate-client-server-versions", bool(report.get("binary_sha256")),
           versions.get("passed") is True, "runtime_versions", "exact-client-and-server-versions-required")
    trust = observation.get("tls_trust", {})
    record("normal-tls-and-current-device-key-authority",
           bool(observation.get("authenticated_owners")) if browser else bool(report.get("actor_enrollment")),
           (observation.get("unconfigured_ca_refused") is True and trust.get("certificate_errors_ignored") is False
            and observation.get("authenticated_owners") == 7 and len(report.get("actor_bindings", {})) == 7)
           if browser else report.get("current_trust_verified") is True,
           "browser_observation.tls_trust" if browser else "current_trust_verified")
    record("selected-transport-and-authenticated-discovery", bool(checks),
           "seven-current-browser-owners-on-selected-transport" in checks if browser else
           report.get("selected_transport_verified") is True, "browser_observation.admission.checks" if browser else "checks")
    record("one-service-contract-and-original-operation-recovery", bool(checks),
           "original-browser-operation-reference-recovers-same-conversation" in checks if browser else
           {"independent-device-explicitly-activates-original-config-after-original-sdk-owner-joins",
            "independent-device-recovers-original-create-and-history-after-original-helper-close"} <= set(checks),
           "browser_observation.admission.checks" if browser else "conversation_control.checks")
    membership = {"participant-add-update-remove-rejoin-and-current-addressed-history"} if browser else {
        "scoped-native-conversation-and-nonmember-current-access-refusal",
        "four-active-participants-and-addition-does-not-expand-older-message-audience",
        "membership-update-original-replay-changed-intent-and-stale-revision-guards",
        "remove-and-readmit-preserve-participant-id-and-fence-old-epoch-history-and-replay"}
    record("participant-add-update-remove-and-current-audiences", bool(checks), membership <= set(checks), "admission.checks")
    upgrade = {"two-concurrent-accepted-sessions-upgraded-to-audio-video-and-unoffered-screen-refused"} if browser else {
        "two-current-accepted-session-intents-coexist-with-one-main-session",
        "additive-session-capability-intent-and-unoffered-screen-refusal-without-state-change"}
    record("session-upgrade-and-current-capability-refusal", bool(checks), upgrade <= set(checks), "admission.checks")
    programme = observation.get("programme", {}) if browser else report.get("movement", {})
    moves = programme.get("moves", [])
    movement_passed = (programme.get("passed") is True and len(moves) == 4
        and all(m.get("original_capture_closed") is True and m.get("source_master_continues") is True for m in moves)) if browser else (
        programme.get("original_moves_completed") == 4 and len(moves) == 4
        and all(m.get("original_reference_preserved") is True and m.get("original_retired") is True
            and m.get("camera_transfer_admitted") is True for m in moves))
    record("device-replacement-and-old-owner-retirement", bool(programme),
           movement_passed,
           "browser_observation.programme" if browser else "movement", programme.get("failure", {}).get("kind"))
    workload = observation.get("actual_connections") == 6 if browser else (
        observation.get("actual_connection_workload_observed") is True
        and observation.get("maximum_active_members_observed") == 4
        and observation.get("concurrent_session_intents_observed") == 2)
    record("declared-concurrent-workload", bool(observation), workload, "browser_observation" if browser else "conversation_control")
    capture = report.get("packet_capture", {})
    record("actual-private-packet-capture", bool(capture), capture.get("observed") is True
           and capture.get("original_joined") is True and capture.get("kernel_dropped_packets") == 0,
           "packet_capture", capture.get("failure"), "environment" if capture.get("environment_failure") else "behavior")
    decoded = observation.get("decoded_media", {}) if browser else report.get("native_av_decoded", {})
    record("independently-decoded-duplex-media-and-profile-budgets", bool(decoded),
           decoded.get("passed") is True, "browser_observation.decoded_media" if browser else "native_av_decoded")
    authority = report.get("current_authority", observation.get("current_authority", {}))
    record("current-revocation-expiry-and-conflicting-operation-refusals",
           all(name in authority for name in ("revocation", "expiry", "conflict")),
           all(authority.get(name, {}).get("passed") is True for name in ("revocation", "expiry", "conflict")),
           "current_authority", "every-current-authority-subcase-required")
    idle = report.get("idle_resources", {})
    retirement = observation.get("retirement", {})
    record("owned-drain-and-resource-reclamation", bool(idle),
           idle_verdict(idle) and retirement.get("passed") is True
           and all(s.get("original_joined") is True for s in report.get("servers", [])),
           "idle_resources")
    if browser:
        for name in SELECTION_CHECKS:
            record(name, bool(observation.get("selection")), browser_selection_verified(observation), "browser_observation.selection")
    if set(outcomes) != required:
        raise ValueError("Requirement evaluator coverage differs")
    cell["check_results"] = outcomes
    cell["status"] = ("passed" if all(r["status"] == "passed" for r in outcomes.values()) else
                      "failed" if any(r["status"] == "failed" for r in outcomes.values()) else "blocked")
    cell["reason"] = "all-required-checks-passed" if cell["status"] == "passed" else "original-attempt-requirement-results"
    cell["verifier"] = dict(format="connect/g6-058-cell-verdict/1", required=len(required),
        evaluated=len(outcomes), report_sha256=report_sha256, integrity=integrity)
    failures = [observation.get("failure", {}), *programme.get("failures", [])]
    cell["library_gaps"] = [dict(kind=f.get("kind"), problem=f.get("problem"),
        stage=f.get("stage", "device-replacement"), report_sha256=report_sha256)
        for f in failures if f.get("kind") == "unsupported"
        or (f.get("problem") or {}).get("reason") in {"unsupported_feature", "unsupported-media-profile"}]
    return cell


def summarize(matrix):
    cells = matrix["cells"]
    counts = {status: sum(c["status"] == status for c in cells)
              for status in ("passed", "failed", "blocked", "not-run", "running", "incomplete")}
    requirements = [result for cell in cells for result in cell.get("check_results", {}).values()]
    matrix["summary"] = dict(cells=counts, required_cells=32, required_checks=432,
        checks={status: sum(r["status"] == status for r in requirements)
                for status in ("passed", "failed", "blocked", "not-run", "running")})
    matrix["matrix_qualified"] = matrix["task_complete"] = (
        len(cells) == 32 and counts["passed"] == 32 and len(requirements) == 432
        and all(r["status"] == "passed" for r in requirements))
    matrix["whole_gate_qualified"] = False
    return matrix["summary"]
