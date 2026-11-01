"""Required G6-058 cells and strict separation of component/complete evidence."""
import hashlib
import itertools
import json
from pathlib import Path
import re

DIMENSIONS = {
    "server": {"macos-arm64", "ubuntu24-x86_64"},
    "network": {"local", "remote-profile-fixture"},
    "transport": {"native-quic", "chrome-webtransport", "chrome-wss-webrtc", "firefox-wss-webrtc"},
    "signing": {"EdDSA", "ES256"},
}
REQUIRED_CHECKS = {
    "identified-candidate-client-server-versions", "normal-tls-and-current-device-key-authority",
    "selected-transport-and-authenticated-discovery", "one-service-contract-and-original-operation-recovery",
    "participant-add-update-remove-and-current-audiences", "session-upgrade-and-current-capability-refusal",
    "device-replacement-and-old-owner-retirement", "declared-concurrent-workload",
    "actual-private-packet-capture", "independently-decoded-duplex-media-and-profile-budgets",
    "current-revocation-expiry-and-conflicting-operation-refusals", "owned-drain-and-resource-reclamation",
}
SELECTION_CHECKS = {
    "wt-unavailable-selects-documented-wss-fallback-with-reason",
    "neither-path-usable-refuses-without-mutation-resubmission",
}
SERVICE_INPUTS = {
    "contracts/connect-uctp.md", "contracts/connect-media-v2.md",
    "contracts/openapi-canonical-http.json", "contracts/openapi-application-http.json",
}
CONTROL_CASES = (
    "independent_http3_signed_control_and_reconnect_share_native_receipt_eddsa",
    "independent_http3_signed_control_and_reconnect_share_native_receipt_es256",
    "wrong_missing_duplicate_origin_and_wrong_mount_refuse_before_authority",
    "canonical_webtransport_discovery::configured_webtransport_discovery_is_bounded_authenticated_current_and_read_only",
    "canonical_webtransport_discovery::disabled_webtransport_and_unrelated_http_listener_do_not_invent_transport_discovery",
    "canonical_webtransport_discovery::discovery_sdk_refuses_incompatible_and_oversized_responses_during_collection",
    "canonical_websocket_control::tls_wss_signed_control_reconnect_and_native_original_receipt_eddsa",
    "canonical_websocket_control::tls_wss_signed_control_reconnect_and_native_original_receipt_es256",
    "canonical_websocket_control::tls_origin_mount_refusals_do_not_consume_canonical_slots",
    "canonical_websocket_control::physical_budget_is_shared_across_wss_http3_and_native_and_reuses_retired_slots",
    "canonical_websocket_control::dropping_host_closes_live_wss_and_releases_tcp_listener",
    "canonical_websocket_control::cancelling_host_shutdown_keeps_wss_and_pending_tls_owned",
    "canonical_websocket_control::signing_expiry_retires_wss_and_linked_native_and_http3",
    "canonical_websocket_control::production_sdk_uses_actual_tls_wss_and_recovers_original_receipt_eddsa",
    "canonical_websocket_control::production_sdk_uses_actual_tls_wss_and_recovers_original_receipt_es256",
)


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def load_contract(root):
    path = root / "contracts/gate6-transport-matrix.json"
    contract = json.loads(path.read_text())
    if contract.get("format") != "connect/gate6-transport-matrix/1" or contract.get("task") != "G6-058":
        raise ValueError("Unknown matrix contract")
    if set(contract["dimensions"]) != set(DIMENSIONS):
        raise ValueError("Matrix dimensions differ")
    for key, required in DIMENSIONS.items():
        supplied = contract["dimensions"][key]
        if len(supplied) != len(required) or set(supplied) != required:
            raise ValueError("Required matrix dimension omitted or duplicated")
    if contract["required_cell_count"] != 32:
        raise ValueError("Required matrix cell count differs")
    if len(contract["required_checks"]) != 12 or set(contract["required_checks"]) != REQUIRED_CHECKS:
        raise ValueError("Incomplete matrix behavior checklist")
    if len(contract["browser_selection_checks"]) != 2 or set(contract["browser_selection_checks"]) != SELECTION_CHECKS:
        raise ValueError("Fallback/refusal checks absent")
    if contract["workload"] != dict(participants=4, connections=6, concurrent_sessions=2):
        raise ValueError("Declared workload differs")
    if len(contract["service_contract"]) != 4 or set(contract["service_contract"]) != SERVICE_INPUTS:
        raise ValueError("Service protocol or HTTP reference omitted")
    for name in [contract["profile"], *contract["service_contract"], *contract["fixture_templates"]]:
        path = root / name
        if Path(name).is_absolute() or ".." in Path(name).parts or path.is_symlink() or not path.is_file():
            raise ValueError("Unsafe or missing matrix contract input")
    return contract


def inventory(root, revision):
    contract = load_contract(root)
    bound = ["contracts/gate6-transport-matrix.json", contract["profile"],
        *contract["service_contract"], *contract["fixture_templates"]]
    service_hashes = {name: digest(root / name) for name in contract["service_contract"]}
    identity = hashlib.sha256(json.dumps(service_hashes, sort_keys=True).encode()).hexdigest()
    cells = []
    for server, network, transport, signing in itertools.product(*(contract["dimensions"][key] for key in DIMENSIONS)):
        checks = list(contract["required_checks"])
        if transport != "native-quic":
            checks.extend(contract["browser_selection_checks"])
        cells.append(dict(id="/".join(("G6-058", server, network, transport, signing)),
            server=server, network=network, transport=transport, signing=signing,
            expected_client_platform="ubuntu24-x86_64" if network == "remote-profile-fixture" else server,
            status="not-run", reason="full-cell-executor-not-run", required_checks=checks,
            client_versions=None, server_version=None, actual_topology=None,
            packet_capture=None, decoded_media=None, outcome=None))
    if len(cells) != 32 or len({cell["id"] for cell in cells}) != 32:
        raise ValueError("Required cell inventory differs")
    return dict(format="connect/gate6-transport-matrix-report/1", task="G6-058",
        source_revision=revision, contract_inputs={name: digest(root / name) for name in bound},
        service_contract_sha256=identity, required_cells=32, cells=cells,
        supporting_control=None, matrix_qualified=False, task_complete=False,
        whole_gate_qualified=False,
        scope="All required current local/Remote-profile transport cells. No inherited component, named WAN host or media result. G6-054 remains deferred; its verification is not waived.")


def attach_control(report, control):
    """A component has no authority to qualify any complete transport cell."""
    if control.get("format") != "connect/transport-control-component/1":
        raise ValueError("Control component kind differs")
    fields = {"format", "source_revision", "service_contract_sha256", "report_sha256",
        "source_inputs_sha256", "executable_sha256", "selected_cases", "passed", "all_cases_executed"}
    if set(control) != fields:
        raise ValueError("Unexpected control reference fields")
    for name in ("report_sha256", "source_inputs_sha256", "service_contract_sha256"):
        if not isinstance(control[name], str) or not re.fullmatch("[0-9a-f]{64}", control[name]):
            raise ValueError("Control reference digest differs")
    if (type(control["passed"]) is not bool or type(control["all_cases_executed"]) is not bool
            or not isinstance(control["selected_cases"], list) or not control["selected_cases"]
            or len(control["selected_cases"]) != len(set(control["selected_cases"]))
            or not set(control["selected_cases"]) <= set(CONTROL_CASES)):
        raise ValueError("Control case selection differs")
    executable = control["executable_sha256"]
    if executable is not None and (not isinstance(executable, str) or not re.fullmatch("[0-9a-f]{64}", executable)):
        raise ValueError("Control executable digest differs")
    if control["passed"] and (not executable or not control["all_cases_executed"]):
        raise ValueError("Incomplete control result cannot pass")
    if control.get("source_revision") != report["source_revision"]:
        raise ValueError("Control component candidate differs")
    if control.get("service_contract_sha256") != report["service_contract_sha256"]:
        raise ValueError("Control component service contract differs")
    if any(cell["status"] != "not-run" for cell in report["cells"]):
        raise ValueError("Control evidence cannot adopt completed cells")
    report["supporting_control"] = control
    report["matrix_qualified"] = report["task_complete"] = report["whole_gate_qualified"] = False
    return report
