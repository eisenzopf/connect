"""Execute available matrix workloads without mistaking a component for a cell.

One failed cell cannot stop selection of another. Missing producers are explicit
work items, not skipped passes. No result crosses platform, network or key cohorts.
"""
import copy
import re

from .transport_matrix import DIMENSIONS


COMPLETION_GAPS = [
    "full-cell-current-expiry-and-conflict-observer-not-connected",
    "full-cell-ten-second-idle-resource-observer-not-connected",
    "full-cell-requirement-verifier-not-connected",
]

AV_CHECK = "independently-decoded-duplex-media-and-profile-budgets"


def browser_selection_verified(observation):
    """Require original recovery, real unavailable paths and joined owners."""
    selected = observation.get("selection", {})
    sockets = observation.get("unavailable_transports", {})
    commands = selected.get("commands", [])
    operation = selected.get("original_operation_id")
    return (selected.get("format") == "connect/browser-matrix-selection/1"
        and selected.get("passed") is True and selected.get("selected_transport") == "wss-webrtc"
        and selected.get("fallback_reason") in {"preferred-transport-timeout", "preferred-transport-failed"}
        and selected.get("refusal", {}).get("certainty") == "not-submitted"
        and selected.get("refusal", {}).get("kind") in {"transport-uncertain", "deadline"}
        and selected.get("attempts") == [dict(phase=phase, transport=transport)
            for phase in ("fallback", "neither") for transport in ("webtransport", "wss-webrtc")]
        and isinstance(operation, str) and bool(operation)
        and selected.get("original_reference_recovered") == 2
        and selected.get("application_mutations_submitted") == 0
        and isinstance(commands, list) and 2 <= len(commands) <= 32
        and {command.get("phase") for command in commands} == {"fallback", "recovery-after-refusal"}
        and all(command.get("type") == "operation.get" and command.get("operation_id") == operation for command in commands)
        and selected.get("original_physical_owners") == 4 and selected.get("original_physical_owners_joined") is True
        and type(selected.get("cleanup_elapsed_ms")) in (int, float) and 0 <= selected["cleanup_elapsed_ms"] <= 5000
        and sockets.get("original_sockets_joined") is True
        and type(sockets.get("udp_datagrams")) is int and 0 < sockets["udp_datagrams"] <= 256
        and type(sockets.get("tcp_connections")) is int and 0 < sockets["tcp_connections"] <= 16
        and sockets.get("errors") == 0 and sockets.get("overflow") is False)


def configure_execution(matrix, environment, stage="media"):
    if stage not in {"full", "media", "admission", "programme", "quality", "non-av"}:
        raise ValueError("Unknown execution stage")
    if environment["server"] not in DIMENSIONS["server"]:
        raise ValueError("Unapproved execution platform")
    matrix["execution_environment"] = environment
    matrix["requested_stage"] = stage
    if stage == "full":
        return configure_full_execution(matrix, environment)
    for cell in matrix["cells"]:
        blockers = []
        non_av = stage == "non-av"
        admission = (stage == "full" and cell["transport"] != "native-quic") or stage in {"admission", "programme", "quality"} or (non_av and cell["transport"] != "native-quic")
        actual_stage = ("quality" if admission else "media") if stage == "full" else "admission" if non_av and admission else "data" if non_av else stage
        if cell["server"] != environment["server"]:
            blockers.append("requires-other-local-arm-host")
        if admission:
            if (cell["server"] != "ubuntu24-arm64" or cell["network"] != "local"
                    or cell["transport"] not in {"chrome-webtransport", "chrome-wss-webrtc"}):
                blockers.append("browser-admission-producer-unavailable-for-this-cell")
            if actual_stage in {"programme", "quality"} and cell["transport"] != "chrome-webtransport":
                blockers.append("browser-replacement-producer-not-implemented-for-this-transport")
            if not environment.get("browser_runtime_image_id"):
                blockers.append("exact-local-arm-browser-image-not-verified")
        elif cell["transport"] != "native-quic":
            blockers.append("full-browser-movement-media-producer-not-connected")
            if cell["network"] == "local" and cell["server"] == environment["server"]:
                browser = "firefox" if cell["transport"].startswith("firefox") else "chrome"
                if not environment["browsers"][browser]["matches_frozen_version"]:
                    blockers.append("frozen-" + browser + "-runtime-unavailable-in-this-environment")
        if cell["network"] != "local":
            if cell["server"] == "macos-arm64":
                blockers.append("mac-server-linux-client-full-workload-producer-not-connected")
            elif cell["server"] == environment["server"] and not environment["owned_systemd_vm"]:
                blockers.append("owned-ubuntu-systemd-and-client-netns-environment-unavailable")
        if cell["server"] == environment["server"]:
            blockers.extend(environment["execution_blockers"])
        cell["execution"] = dict(
            producer="browser-" + actual_stage if admission else ("native-data" if non_av else "native-media") if cell["transport"] == "native-quic" else None,
            stage=actual_stage,
            runnable=not blockers, blockers=blockers,
            completion_blockers=list(COMPLETION_GAPS) + ([("browser-independent-decoded-quality-not-qualified" if actual_stage == "quality" else "browser-independent-decoded-quality-and-packet-capture-not-connected" if actual_stage == "programme" else "full-browser-movement-media-producer-not-connected"),
                "browser-fallback-and-neither-path-refusal-not-executed"] if admission else []), attempted=False,
            component=None,
        )
        cell["check_results"] = {name: {"status": "not-run", "evidence": None}
                                 for name in cell["required_checks"]}
        if non_av:
            cell["check_results"][AV_CHECK].update(status="not-run", reason="outside-selected-non-av-stage; AV qualification tracked separately")
            cell["non_av_scope"] = dict(required_checks=[name for name in cell["required_checks"] if name != AV_CHECK],
                qualified=False, all_required_checks_executed=False,
                note="Admission/data stages execute available checks only. Replacement, expiry and idle gaps remain required; no AV or full-cell qualification.")
        cell["reason"] = "execution-preflight-blocked" if blockers else "required-workload-ready-not-run"
    return matrix


def full_producer_gaps(cell):
    """Declared producer paths, independent of scheduling and behavior results.

    The Mac remote topology/clock bridge remains a separate integration. Keep
    its missing pieces explicit until both executable paths are connected; a
    scheduled command or a verifier function is not an implemented producer.
    """
    if cell["server"] == "macos-arm64" and cell["network"] != "local":
        return ["macos-server-linux-client-remote-producer-not-connected",
                "cross-host-ten-second-idle-observer-not-connected"]
    return []


def configure_full_execution(matrix, environment):
    """Schedule all original attempts, retaining actual producer gaps."""
    for cell in matrix["cells"]:
        browser = cell["transport"] != "native-quic"
        local_host = cell["server"] == environment["server"]
        blockers = list(environment["execution_blockers"]) if local_host else ["assigned-to-other-local-arm-host"]
        cell["execution"] = dict(producer="browser-quality" if browser else "native-media",
            stage="quality" if browser else "media", runnable=not blockers,
            blockers=blockers, completion_blockers=full_producer_gaps(cell), attempted=False, component=None,
            executor="scripts/rehearse-gate6-browser-matrix.py" if browser else "scripts/check-transport-fixtures.py",
            verifier="scripts/lib/transport_matrix_verdict.py")
        cell["check_results"] = {name: dict(status="not-run", evidence=None) for name in cell["required_checks"]}
        cell["reason"] = "assigned-to-local-arm-worker" if local_host else "assigned-to-other-local-arm-host"
    return matrix


def full_run_preflight(matrix):
    """Owner policy: no selected-cell or partial-stage acceptance runs.

    Record environment problems without cancelling independent cells. Behavior
    failures and library refusals are evaluated during the original execution.
    """
    ids = [cell["id"] for cell in matrix["cells"]]
    if len(ids) != 32 or len(set(ids)) != 32 or matrix.get("requested_stage") != "full":
        raise ValueError("Every G6-058 run requires all 32 complete workloads")
    missing = []
    for cell in matrix["cells"]:
        execution = cell["execution"]
        gaps = [b for b in execution["blockers"] if b != "assigned-to-other-local-arm-host"]
        gaps.extend(gap for gap in execution["completion_blockers"]
                    if gap.endswith("-not-connected") or gap.endswith("-not-implemented"))
        if gaps:
            missing.append(dict(cell_id=cell["id"], gaps=sorted(set(gaps))))
    return dict(format="connect/g6-058-full-run-preflight/1", required_cells=32,
                selected_cells=ids, all_workloads_connected=not missing,
                missing=missing, tests_executed=False,
                scope="Wiring and environment preflight only; never behavior qualification.")


def select_cells(matrix, ids):
    if not ids or len(ids) != len(set(ids)):
        raise ValueError("Select distinct explicit matrix cell IDs")
    by_id = {c["id"]: c for c in matrix["cells"]}
    if not set(ids) <= set(by_id):
        raise ValueError("Unknown matrix cell ID")
    return [by_id[name] for name in ids]


def record_native_result(matrix, cell, report, report_sha256, exit_code):
    """Bind negative/partial observations; this producer cannot qualify a cell."""
    if cell not in matrix["cells"] or cell["transport"] != "native-quic":
        raise ValueError("Native evidence cannot stand in for browser evidence")
    if cell["execution"]["component"] is not None:
        raise ValueError("Original cell attempt already recorded")
    if not re.fullmatch(r"[0-9a-f]{64}", report_sha256):
        raise ValueError("Missing original report digest")
    if (report.get("format") != "connect/transport-fixture-rehearsal/1"
            or report.get("source_revision") != matrix["source_revision"]
            or report.get("host", {}).get("platform") != cell["server"]
            or report.get("network_profile") != ("local" if cell["network"] == "local" else "remote")
            or report.get("signing_algorithm") != cell["signing"]):
        raise ValueError("Component candidate/platform/network/signing differs from selected cell")
    if type(report.get("passed")) is not bool or type(exit_code) is not int:
        raise ValueError("Missing terminal component outcome")
    guards = {key: report.get(key) is True for key in (
        "source_inputs_unchanged", "executables_unchanged", "sdk_dist_inputs_unchanged")}
    passed = report["passed"] and exit_code == 0 and all(guards.values())
    # Copy a small allowlist. Reports contain private paths and generated actors.
    component = dict(kind=cell["execution"]["producer"], report_sha256=report_sha256,
                     passed=passed, exit_code=exit_code, integrity=guards,
                     source_revision=report["source_revision"],
                     binary_sha256=report.get("binary_sha256"),
                     peer_binary_sha256=report.get("peer_binary_sha256"),
                     failure=report.get("failure"))
    cell["execution"].update(attempted=True, component=component)
    cell.update(status="incomplete" if passed else "failed",
                reason="required-workload-passed-full-checks-missing" if passed else "required-workload-failed")
    if cell["execution"]["producer"] == "native-data":
        record_native_data_checks(cell, report, report_sha256, passed)
    matrix["matrix_qualified"] = matrix["task_complete"] = matrix["whole_gate_qualified"] = False
    return cell


def record_native_data_checks(cell, report, digest, passed):
    """Promote only observed control clauses; data is not AV/movement/idle proof."""
    observed = report.get("conversation_control", {})
    checks = observed.get("checks", [])
    valid = (passed and observed.get("passed") is True
        and observed.get("all_observed_helpers_joined") is True
        and observed.get("workload_mode") == "six-native-data-connections"
        and observed.get("maximum_active_members_observed") == 4
        and observed.get("concurrent_session_intents_observed") == 2
        and observed.get("actual_connection_workload_observed") is True
        and observed.get("actual_topology_verified") is True)
    requirements = {
        "participant-add-update-remove-and-current-audiences": {
            "scoped-native-conversation-and-nonmember-current-access-refusal",
            "four-active-participants-and-addition-does-not-expand-older-message-audience",
            "membership-update-original-replay-changed-intent-and-stale-revision-guards",
            "remove-and-readmit-preserve-participant-id-and-fence-old-epoch-history-and-replay",
        },
        "session-upgrade-and-current-capability-refusal": {
            "two-current-accepted-session-intents-coexist-with-one-main-session",
            "additive-session-capability-intent-and-unoffered-video-refusal-without-state-change",
        },
    }
    for name, required in requirements.items():
        if valid and required <= set(checks):
            cell["check_results"][name] = dict(status="passed", evidence=digest)
    # Expose the executed subcases without claiming the composite expiry,
    # four-move or ten-second resource requirements from partial observations.
    cell["non_av_observations"] = dict(report_sha256=digest, valid=valid,
        checks=checks, producer_checks=report.get("checks", []),
        missing=["four-device-moves-and-old-owner-retirement", "current-expiry-refusal",
                 "ten-second-idle-resource-restoration"])


def record_browser_result(matrix, cell, report, report_sha256, exit_code):
    """An actual admission observation is a partial stage, never cell evidence."""
    if (cell not in matrix["cells"] or cell["execution"].get("producer") not in {"browser-admission", "browser-programme", "browser-quality"}
            or cell["execution"]["component"] is not None):
        raise ValueError("Browser admission must match an unrecorded selected stage")
    if not re.fullmatch(r"[0-9a-f]{64}", report_sha256):
        raise ValueError("Missing original browser report digest")
    if (report.get("format") != "connect/browser-matrix-component/1" or report.get("stage") != cell["execution"].get("stage")
            or report.get("source_revision") != matrix["source_revision"] or report.get("cell_id") != cell["id"]
            or report.get("server_platform") != cell["server"] or report.get("client_platform") != "ubuntu24-arm64"
            or cell["network"] != "local" or report.get("transport") != cell["transport"]
            or report.get("signing_algorithm") != cell["signing"]
            or report.get("browser_image_id") != matrix["execution_environment"].get("browser_runtime_image_id")):
        raise ValueError("Browser stage candidate/platform/network/transport/signing/image differs")
    if (type(report.get("passed")) is not bool or type(exit_code) is not int
            or report.get("cell_qualified") is not False or report.get("full_media_programme_executed") is not False):
        raise ValueError("Missing scoped terminal browser outcome")
    guards = {key: report.get(key) is True for key in (
        "source_inputs_unchanged", "executable_unchanged", "sdk_dist_inputs_unchanged", "browser_owner_joined")}
    passed = report["passed"] and exit_code == 0 and all(guards.values())
    observation = report.get("browser_observation", {})
    if passed and (observation.get("source_revision") != matrix["source_revision"]
            or observation.get("transport") != cell["transport"] or observation.get("signing_algorithm") != cell["signing"]
            or observation.get("authenticated_owners") != 7 or observation.get("actual_connections") != 6
            or observation.get("passed") is not True):
        raise ValueError("Successful admission lacks its actual browser workload")
    if passed and report["stage"] in {"programme", "quality"}:
        programme = observation.get("programme", {})
        moves = programme.get("moves", [])
        if (programme.get("passed") is not True or len(moves) != 4
                or sorted((move.get("source_index"), move.get("ordinal"), move.get("scheduled_start_ms")) for move in moves)
                != [(0, 0, 15000), (0, 1, 40000), (4, 0, 15000), (4, 1, 40000)]
                or not all(move.get("original_capture_closed") is True and move.get("source_master_continues") is True for move in moves)
                or programme.get("source_programme", {}).get("durationMs") != 62000
                or not 62000 <= programme.get("finished_ms", -1) < 67000):
            raise ValueError("Browser programme lacks the four original moves and fixed terminal")
    if passed and report["stage"] == "quality":
        decoded = observation.get("decoded_media", {})
        capture = report.get("packet_capture", {})
        if (decoded.get("passed") is not True or decoded.get("window_ms") != 60000
                or len(decoded.get("routes", [])) != 6
                or not all(route.get("passed") is True for route in decoded["routes"])
                or capture.get("observed") is not True or capture.get("original_joined") is not True):
            raise ValueError("Browser quality stage lacks six measured routes or actual owned capture")
    cell["execution"].update(attempted=True, component=dict(kind=cell["execution"]["producer"], passed=passed,
        exit_code=exit_code, report_sha256=report_sha256, integrity=guards, source_revision=report["source_revision"],
        binary_sha256=report.get("binary_sha256"), browser=observation.get("browser"),
        authenticated_owners=observation.get("authenticated_owners"), actual_connections=observation.get("actual_connections"),
        failure=report.get("failure")))
    cell.update(status="incomplete" if passed else "failed",
                reason=cell["execution"]["producer"] + ("-passed-full-checks-missing" if passed else "-failed"))
    record_browser_control_checks(cell, observation, report_sha256, passed)
    if observation.get("selection"):
        selection_passed = (all(guards.values()) and browser_selection_verified(observation)
            and observation.get("source_revision") == matrix["source_revision"]
            and observation.get("transport") == cell["transport"]
            and observation.get("signing_algorithm") == cell["signing"])
        from .transport_matrix import SELECTION_CHECKS
        for name in SELECTION_CHECKS:
            cell["check_results"][name] = dict(status="passed" if selection_passed else "failed", evidence=report_sha256)
        if selection_passed:
            cell["execution"]["completion_blockers"] = [gap for gap in cell["execution"]["completion_blockers"]
                if gap != "browser-fallback-and-neither-path-refusal-not-executed"]
    if report.get("idle_resources") is not None:
        from .matrix_idle import idle_verdict
        idle = report["idle_resources"]
        local = observation.get("idle_resources", {})
        retirement = observation.get("retirement", {})
        browser = observation.get("trusted_browser", {})
        audio = report.get("owned_audio", {})
        reclaimed = (report["stage"] == "quality" and all(guards.values())
            and observation.get("source_revision") == matrix["source_revision"]
            and observation.get("transport") == cell["transport"] and observation.get("signing_algorithm") == cell["signing"]
            and idle.get("passed") is True and idle_verdict(idle)
            and local.get("passed") is True and local.get("localOwnersJoined") is True
            and 4 <= local.get("media_peers", 0) <= 7 and local.get("renderer", {}).get("joined") is True
            and local.get("renderer", {}).get("owners") == 10
            and local.get("active_bindings") == 0 and local.get("queued_packets") == 0 and local.get("queued_bytes") == 0
            and retirement.get("passed") is True and 0 <= retirement.get("allLocalOwnersElapsedMs", -1) <= 5000
            and retirement.get("originalEndsCompleted") == 6 and retirement.get("originalClientsClosed") == 7
            and retirement.get("originalSessionsEnded") == 2
            and browser.get("original_process_joined") is True and browser.get("forced_kill") is False
            and audio.get("original_process_joined") is True and audio.get("forced_kill", False) is False
            and len(report.get("servers", [])) == 2 and all(s.get("original_joined") is True for s in report["servers"]))
        cell["check_results"]["owned-drain-and-resource-reclamation"] = dict(
            status="passed" if reclaimed else "failed",
            evidence=dict(report_sha256=report_sha256, source_revision=report["source_revision"], section="idle_resources"))
        gaps = cell["execution"]["completion_blockers"]
        gap = "full-cell-ten-second-idle-resource-observer-not-connected"
        if gap in gaps:
            gaps.remove(gap)
        if not reclaimed:
            gaps.append("owned-drain-and-ten-second-idle-not-qualified")
    matrix["matrix_qualified"] = matrix["task_complete"] = matrix["whole_gate_qualified"] = False
    return cell


def record_browser_control_checks(cell, observation, digest, passed):
    """Record completed admission clauses without borrowing media/expiry proof."""
    retirement = observation.get("retirement", {})
    valid = (passed and observation.get("passed") is True
        and observation.get("authenticated_owners") == 7 and observation.get("actual_connections") == 6
        and retirement.get("passed") is True and retirement.get("originalEndsCompleted") == 6
        and retirement.get("originalSessionsEnded") == 2 and retirement.get("originalClientsClosed") == 7)
    checks = observation.get("admission", {}).get("checks", [])
    requirements = {
        "participant-add-update-remove-and-current-audiences":
            "participant-add-update-remove-rejoin-and-current-addressed-history",
        "session-upgrade-and-current-capability-refusal":
            "two-concurrent-accepted-sessions-upgraded-to-audio-video-and-unoffered-screen-refused",
    }
    for name, check in requirements.items():
        if valid and check in checks:
            cell["check_results"][name] = dict(status="passed", evidence=digest)
    if "non_av_scope" in cell:
        cell["non_av_observations"] = dict(report_sha256=digest, valid=valid, checks=checks,
            missing=["four-device-moves-and-old-owner-retirement", "current-revocation-and-expiry-refusal",
                     "ten-second-idle-resource-restoration"])


def merge_hosts(base, reports):
    """Exactly one current report per host; never overwrite another attempt."""
    merged = copy.deepcopy(base)
    hosts = set()
    by_id = {cell["id"]: cell for cell in merged["cells"]}
    for report in reports:
        host = report.get("execution_environment", {}).get("server")
        if host not in DIMENSIONS["server"] or host in hosts:
            raise ValueError("Missing or duplicate host report")
        if (report.get("source_revision") != merged["source_revision"]
                or report.get("contract_inputs") != merged["contract_inputs"]
                or report.get("service_contract_sha256") != merged["service_contract_sha256"]):
            raise ValueError("Host candidate or contract differs")
        cells = report.get("cells", [])
        if len(cells) != 32 or {c["id"] for c in cells} != set(by_id):
            raise ValueError("Incomplete or duplicate host cell inventory")
        for cell in cells:
            original = by_id[cell["id"]]
            if any(cell.get(k) != original[k] for k in DIMENSIONS):
                raise ValueError("Cell dimensions changed")
            if cell["required_checks"] != original["required_checks"]:
                raise ValueError("Cell requirements changed")
            if cell["status"] not in {"not-run", "running", "failed", "incomplete", "blocked", "passed"}:
                raise ValueError("Unknown cell outcome")
            if cell["status"] == "passed" and (cell.get("verifier", {}).get("format") != "connect/g6-058-cell-verdict/1"
                    or set(cell.get("check_results", {})) != set(original["required_checks"])
                    or any(r.get("status") != "passed" or not r.get("evidence") for r in cell["check_results"].values())
                    or cell["verifier"].get("integrity") is not True):
                raise ValueError("Complete cell lacks all current requirement verdicts")
            if cell["server"] == host:
                original.update(copy.deepcopy(cell))
        hosts.add(host)
    merged["host_reports_received"] = sorted(hosts)
    merged["missing_host_reports"] = sorted(DIMENSIONS["server"] - hosts)
    from .transport_matrix_verdict import summarize
    summarize(merged)
    return merged
