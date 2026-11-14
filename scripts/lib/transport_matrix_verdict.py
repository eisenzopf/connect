"""Requirement-level verdicts for an original G6-058 cell attempt.

Every contract requirement has an evaluator. A missing observation is a blocked
check, never a pass and never silently omitted. A failed programme must not erase
successful control observations from the same original attempt.
"""
import re

from .transport_matrix import REQUIRED_CHECKS, SELECTION_CHECKS, media_quality_bound
from .rtc_source_frame_witness import report_bound as rtc_source_frame_report_bound


def result(passed, evidence, *, reason=None, category=None):
    return dict(status="passed" if passed else "failed", evidence=evidence,
                **({"reason": reason} if reason else {}),
                **({"category": category} if category and not passed else {}))


def blocked(reason, evidence=None, category="dependency"):
    return dict(status="blocked", reason=reason, category=category, evidence=evidence)


def native_cycle_join_verified(control, resource):
    from .native_programme_profile import worker_join_verified
    try:
        retirement = control["retirement"]
        elapsed = retirement["all_local_owners_elapsed_ms"]
        return (retirement.get("passed") is True and retirement.get("original_helpers_joined") is True
            and retirement.get("original_clients_closed") is True
            and type(retirement.get("observed_helpers")) is int and retirement["observed_helpers"] >= 1
            and type(elapsed) in (int, float) and 0 <= elapsed <= 10000
            and worker_join_verified(control.get("programme_worker"), resource.get("original_programme_owner"),
                                     retirement.get("original_terminal_monotonic_seconds")))
    except (KeyError, TypeError, ValueError):
        return False



def original_producer_outcome(cell, report, report_sha256):
    """Whole-cell admission is separate from its independently retained checks."""
    execution = cell.get("execution", {})
    component = execution.get("component")
    component = component if isinstance(component, dict) else {}
    code = component.get("exit_code")
    declared = report.get("passed")
    digest = component.get("report_sha256")
    failed = (declared is False or component.get("passed") is False
        or type(code) is int and code != 0
        or report.get("failure") is not None or component.get("failure_phase") is not None
        or component.get("failure_kind") is not None
        or digest is not None and digest != report_sha256)
    complete = (execution.get("attempted") is True and declared is True
        and component.get("passed", True) is True and type(code) is int and code == 0 and digest == report_sha256)
    return dict(status="failed" if failed else "passed" if complete else "blocked",
        reason="original-producer-failed" if failed else
            "original-producer-completed" if complete else "original-producer-outcome-missing",
        report_passed=declared if type(declared) is bool else None,
        exit_code=code if type(code) is int else None,
        report_sha256=report_sha256, report_digest_matches=digest == report_sha256,
        attempted=execution.get("attempted") is True)


def original_producer_verified(cell):
    """Recheck copied terminal metadata at host merge and final summarization."""
    execution = cell.get("execution", {})
    component = execution.get("component")
    verifier = cell.get("verifier", {})
    outcome = verifier.get("producer_outcome", {})
    return (isinstance(component, dict) and execution.get("attempted") is True
        and component.get("passed", True) is True and type(component.get("exit_code")) is int and component["exit_code"] == 0
        and component.get("failure_phase") is None and component.get("failure_kind") is None
        and verifier.get("format") == "connect/g6-058-cell-verdict/1" and verifier.get("integrity") is True
        and outcome.get("status") == "passed" and outcome.get("report_passed") is True
        and outcome.get("attempted") is True and type(outcome.get("exit_code")) is int
        and outcome["exit_code"] == 0
        and outcome.get("report_digest_matches") is True
        and component.get("report_sha256") == verifier.get("report_sha256") == outcome.get("report_sha256")
        and isinstance(outcome.get("report_sha256"), str)
        and re.fullmatch(r"[0-9a-f]{64}", outcome["report_sha256"]) is not None)



def native_late_controls_verified(report):
    late = report.get("late_native_controls", {})
    owners = sorted(("controller-1", "controller-2", "participant-2", "participant-3", "participant-4", "replacement-1"))
    phases = ([owner + "-before-revocation" for owner in owners]
        + ["revoke-primary-device", "revoked-primary-native-refusal"]
        + [owner + "-after-revocation" for owner in owners] + ["original-actor-configs-unchanged"])
    steps = late.get("steps")
    return (late.get("attempted") is True and late.get("status") == "passed"
        and all(late.get(name) is True for name in ("all_required_calls_attempted", "all_before_exact",
            "primary_revoked", "revoked_primary_refused", "all_after_same", "configs_unchanged"))
        and isinstance(steps, list) and len(steps) == len(phases)
        and all(isinstance(step, dict) and step.get("phase") == phase and step.get("status") == "passed"
            for step, phase in zip(steps, phases))
        and "revoked-primary-device-refuses-while-independent-replacement-and-other-devices-remain-usable" in report.get("checks", []))


def evaluate_cell(cell, report, report_sha256, *, environment, revision):
    """Evaluate all 12 native / 14 browser requirements, without short circuit."""
    from .transport_matrix_execution import browser_selection_verified, browser_resource_plateau_verified
    from .matrix_idle import idle_verdict
    from .native_matrix_idle import native_idle_verdict
    from .matrix_plateau import plateau_verdict
    from .native_programme_profile import PROFILE
    from .native_matrix_authority import authority_verdict

    if not re.fullmatch(r"[0-9a-f]{64}", report_sha256):
        raise ValueError("An original report digest is required")
    browser = cell["transport"] != "native-quic"
    required = REQUIRED_CHECKS | (SELECTION_CHECKS if browser else set())
    if set(cell["required_checks"]) != required:
        raise ValueError("Cell requirements differ from the full contract")
    observation = report.get("browser_observation", {}) if browser else report.get("conversation_control", {})
    checks = observation.get("admission", observation).get("checks", [])
    plateau = report.get("idle_resources", {}).get("format") == "connect/matrix-plateau-observation/1"
    cycles = report.get("cycle_observations", []) if browser and plateau else [observation]
    complete_cycles = not plateau or not browser or len(cycles) == 3
    if browser and plateau:
        checks = sorted(set.intersection(*(set(c.get("admission", {}).get("checks", [])) for c in cycles))) if len(cycles) == 3 else []
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
            and observation.get("authenticated_owners") == 7 and len(report.get("actor_bindings", {})) == 7
            and (not plateau or complete_cycles and all(c.get("authenticated_owners") == 7 for c in cycles)))
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
    if browser and plateau:
        movement_passed = complete_cycles and all(c.get("programme", {}).get("passed") is True
            and len(c["programme"].get("moves", [])) == 4
            and all(m.get("original_capture_closed") is True and m.get("source_master_continues") is True for m in c["programme"]["moves"])
            for c in cycles)
    record("device-replacement-and-old-owner-retirement", bool(programme),
           movement_passed,
           "browser_observation.programme" if browser else "movement", programme.get("failure", {}).get("kind"))
    workload = observation.get("actual_connections") == 6 if browser else (
        observation.get("actual_connection_workload_observed") is True
        and observation.get("maximum_active_members_observed") == 4
        and observation.get("concurrent_session_intents_observed") == 2)
    if browser and plateau:
        workload = complete_cycles and all(c.get("actual_connections") == 6
            and c.get("programme", {}).get("source_programme", {}).get("durationMs") == 62000
            and 62000 <= c.get("programme", {}).get("finished_ms", -1) < 67000
            and c.get("original_cycle_budget", {}).get("budget_ms") == 210000
            and abs(c["original_cycle_budget"].get("deadline_ms", -1) - c["original_cycle_budget"].get("started_ms", -1) - 210000) <= .001
            and c["original_cycle_budget"].get("started_ms", -1) < c["original_cycle_budget"].get("finished_ms", -1) < c["original_cycle_budget"].get("deadline_ms", -1)
            for c in cycles)
    record("declared-concurrent-workload", bool(observation), workload, "browser_observation" if browser else "conversation_control")
    capture = report.get("packet_capture", {})
    record("actual-private-packet-capture", bool(capture), capture.get("observed") is True
           and capture.get("original_joined") is True and capture.get("kernel_dropped_packets") == 0,
           "packet_capture", capture.get("failure"), "environment" if capture.get("environment_failure") else "behavior")
    decoded = observation.get("decoded_media", {}) if browser else report.get("native_av_decoded", {})
    record("independently-decoded-duplex-media-and-profile-budgets", bool(decoded),
           rtc_source_frame_report_bound(report) and ((complete_cycles and all(c.get("decoded_media", {}).get("passed") is True
                and media_quality_bound(c.get("decoded_media")) for c in cycles))
           if browser and plateau else decoded.get("passed") is True and media_quality_bound(decoded)), "browser_observation.decoded_media" if browser else "native_av_decoded")
    authority = report.get("current_authority", observation.get("current_authority", {}))
    record("current-revocation-expiry-and-conflicting-operation-refusals",
           all(name in authority for name in ("revocation", "expiry", "conflict"))
           and (browser or all(authority[name].get("executed") is True for name in ("revocation", "expiry")))
           and (browser or report.get("late_native_controls", {}).get("attempted") is True),
           all(authority.get(name, {}).get("passed") is True for name in ("revocation", "expiry", "conflict"))
           and (not (browser and plateau) or complete_cycles and all(
               all(c.get("current_authority", {}).get(name, {}).get("passed") is True for name in ("revocation", "expiry", "conflict")) for c in cycles))
           and (browser or all(authority_verdict(authority[name], name) for name in ("revocation", "expiry")))
           and (browser or native_late_controls_verified(report)),
           "current_authority" if browser else "current_authority+late_native_controls",
           "every-current-authority-subcase-and-original-late-control-required")
    idle = report.get("idle_resources", {})
    retirement = observation.get("retirement", {})
    if plateau:
        if browser:
            resource_passed = browser_resource_plateau_verified(report)
        else:
            receipts = observation.get("resource_cycles", [])
            resource_passed = (plateau_verdict(idle) and observation.get("matrix_resource_cycles") == 3
                and observation.get("programme_profile") == PROFILE
                and report.get("native_cycle_mutation_inventory", {}).get("passed") is True
                and len(receipts) == 3 and len(report.get("native_cycle_controls", [])) == 3
                and all(receipt.get("index") == index
                    and receipt.get("programme_worker") == resource.get("original_worker_receipt")
                    and receipt.get("original_node_pid") == idle.get("persistent_owners", {}).get("helper", {}).get("programme_node_pid")
                    and receipt.get("programme_worker", {}).get("original_exit_observed") is True
                    and receipt.get("programme_worker", {}).get("forced") is False
                    and control.get("index") == index and native_cycle_join_verified(control, resource)
                    and control.get("programme_worker") == receipt.get("programme_worker")
                    for index, (receipt, resource, control) in enumerate(zip(receipts, idle.get("cycles", []), report.get("native_cycle_controls", [])), 1)))
    else:
        resource_passed = ((idle_verdict(idle) if browser else native_idle_verdict(idle)) and retirement.get("passed") is True
            and (browser or (observation.get("programme_profile") == PROFILE
                and observation.get("programme_worker") == idle.get("retirement", {}).get("programme_worker")
                and retirement == idle.get("retirement"))))
    record("owned-drain-and-resource-reclamation", bool(idle), resource_passed
           and len(report.get("servers", [])) == 2
           and all(s.get("original_joined") is True for s in report.get("servers", [])), "idle_resources")
    if browser:
        for name in SELECTION_CHECKS:
            record(name, bool(observation.get("selection")),
                   (complete_cycles and all(browser_selection_verified({**observation, **c,
                       "unavailable_transports": {**observation.get("unavailable_transports", {}), **c.get("transport_probe_delta", {})}}) for c in cycles)) if plateau else browser_selection_verified(observation), "browser_observation.selection")
    if set(outcomes) != required:
        raise ValueError("Requirement evaluator coverage differs")
    cell["check_results"] = outcomes
    producer = original_producer_outcome(cell, report, report_sha256)
    cell["status"] = ("passed" if producer["status"] == "passed"
                          and all(r["status"] == "passed" for r in outcomes.values()) else
                      "failed" if producer["status"] == "failed"
                          or any(r["status"] == "failed" for r in outcomes.values()) else "blocked")
    cell["reason"] = ("all-required-checks-and-original-producer-passed" if cell["status"] == "passed" else
        producer["reason"] if producer["status"] != "passed" else "original-attempt-requirement-results")
    cell["verifier"] = dict(format="connect/g6-058-cell-verdict/1", required=len(required),
        evaluated=len(outcomes), report_sha256=report_sha256, integrity=integrity,
        producer_outcome=producer)
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
        and all(r["status"] == "passed" for r in requirements)
        and all(original_producer_verified(cell) for cell in cells))
    matrix["whole_gate_qualified"] = False
    return matrix["summary"]
