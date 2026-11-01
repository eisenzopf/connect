"""Read-only independent checks of source-bound transport component evidence.

This reads curated ZIP members only. It never extracts or executes artifact code
and never promotes a component report to a complete transport cell.
"""
import ast
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import runpy
import stat
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[2]
COMPONENTS = {"materializer", "actors", "conversation", "data", "media", "control"}
REFUSALS = {"unsupported-init-signing", "existing-init-signing-refusal",
    "actor-cannot-provision", "revoked-primary-native-refusal"}


def verify_command_outcomes(commands, component):
    expected = REFUSALS | ({"participant-cannot-verify-endpoint"} if component in {"data", "media"} else set())
    if component in {"data", "media"}:
        require(expected <= {command["name"] for command in commands}, "data-authority-refusal-missing")
    for command in commands:
        require(type(command["exit_code"]) is int and
            ((command["exit_code"] != 0) == (command["name"] in expected)), "unexpected-command-outcome")


def require(condition, reason):
    if not condition:
        raise ValueError(reason)


def decode(raw):
    def unique(pairs):
        value = {}
        for key, item in pairs:
            require(key not in value, "duplicate-json-member")
            value[key] = item
        return value
    def nonfinite(_):
        raise ValueError("nonfinite-json-number")
    return json.loads(raw, object_pairs_hook=unique, parse_constant=nonfinite)


def read_members(path, expected_sha256):
    require(re.fullmatch(r"[0-9a-f]{64}", expected_sha256) is not None, "artifact-digest-required")
    with Path(path).open("rb") as stream:
        require(hashlib.file_digest(stream, "sha256").hexdigest() == expected_sha256, "artifact-digest-mismatch")
    with zipfile.ZipFile(path) as archive:
        infos = archive.infolist()
        require(0 < len(infos) <= 32, "artifact-member-count")
        require(len({item.filename for item in infos}) == len(infos), "duplicate-artifact-member")
        require(sum(item.file_size for item in infos) <= 64 * 1024 * 1024, "artifact-size-bound")
        members = {}
        for item in infos:
            name = PurePosixPath(item.filename)
            require(not name.is_absolute() and ".." not in name.parts and "\\" not in item.filename,
                "unsafe-artifact-member")
            require(not item.is_dir() and not item.flag_bits & 1 and
                stat.S_IFMT(item.external_attr >> 16) in {0, stat.S_IFREG}, "nonordinary-artifact-member")
            require(item.file_size <= 16 * 1024 * 1024 and name.suffix == ".json", "artifact-json-bound")
            members[item.filename] = archive.read(item)
        return members


def verify_prepared_inventory(inputs, revision, prepared):
    """Do not let an omitted prepared file disappear behind a valid Git map."""
    lock = decode(subprocess.check_output(["git", "show", revision + ":dependencies/source-lock.json"], cwd=ROOT))
    expected = {".deps/.archives/" + name + "-" + source["version"] + ".crate"
        for name, source in lock["sources"].items() if source.get("kind") == "crate"}
    for path in Path(prepared).rglob("*"):
        if ".git" in path.parts or path.is_symlink() or not path.is_file():
            continue
        expected.add(".deps/" + str(path.relative_to(prepared)))
    actual = {name for name in inputs if name.startswith(".deps/")}
    require(actual == expected, "prepared-source-inventory-mismatch")


def verify_sdk(inputs, directory):
    require(isinstance(inputs, dict) and "node_sdk.js" in inputs and "browser_client.js" in inputs,
        "sdk-runtime-inventory-required")
    directory = Path(directory)
    require(directory.is_dir() and not directory.is_symlink(), "ordinary-sdk-directory-required")
    actual = {}
    for path in directory.rglob("*"):
        require(not path.is_symlink(), "nonordinary-sdk-input")
        if path.is_file():
            with path.open("rb") as stream:
                actual[str(path.relative_to(directory))] = hashlib.file_digest(stream, "sha256").hexdigest()
    require(actual == inputs, "sdk-runtime-bytes-or-inventory-mismatch")
    return len(actual)


def verify_control(report):
    require(report["format"] == "connect/transport-control-component/1", "wrong-control-format")
    # Read the exact case inventory from the reported revision, not today's list.
    source = subprocess.check_output(["git", "show", report["source_revision"] + ":scripts/lib/transport_matrix.py"], cwd=ROOT)
    # Parse the literal tuple without importing/executing historical code.
    tree = ast.parse(source)
    declarations = [node for node in tree.body if isinstance(node, ast.Assign) and
        any(isinstance(target, ast.Name) and target.id == "CONTROL_CASES" for target in node.targets)]
    require(len(declarations) == 1, "control-case-contract-missing")
    contract = ast.literal_eval(declarations[0].value)
    selected, cases = report["selected_cases"], report["cases"]
    require(isinstance(selected, list) and selected and len(selected) == len(set(selected)) and
        set(selected) <= set(contract), "unknown-or-duplicate-control-selection")
    require([case["name"] for case in cases] == selected, "control-case-results-mismatch")
    for case in cases:
        require(case["status"] in {"not-run", "passed", "failed"}, "unknown-control-outcome")
        if case["status"] == "not-run":
            continue
        results = case["test_results"]
        require(type(case["exit_code"]) is int, "control-exit-code-required")
        passed = case["exit_code"] == 0 and not case["compiler_errors"] and not case["failure_locations"] and len(results) == 1 and re.fullmatch(
            r"test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; \d+ filtered out; finished in .+", results[0]) is not None
        require((case["status"] == "passed") == passed, "contradictory-control-case-outcome")
    executed = all(case["status"] != "not-run" for case in cases)
    require(report["all_cases_executed"] is executed, "contradictory-control-execution")
    passed = executed and all(case["status"] == "passed" for case in cases)
    require(report["passed"] is passed, "contradictory-control-component-outcome")
    if passed:
        require(any(step["name"] == "compile" and step["exit_code"] == 0 and not step["compiler_errors"]
            for step in report["steps"]), "successful-control-compilation-missing")
    return dict(selected_cases=len(selected), executed_cases=sum(case["status"] != "not-run" for case in cases),
        passed_cases=sum(case["status"] == "passed" for case in cases), complete_control_selection=set(selected) == set(contract))


def verify_rehearsal(report, component, members):
    require(report["format"] == "connect/transport-fixture-rehearsal/1", "wrong-rehearsal-format")
    servers = report["servers"]
    for server in servers:
        require(server["running_binary_sha256"] == report["binary_sha256"], "server-executable-binding-mismatch")
    require(len({command["name"] for command in report["commands"]}) == len(report["commands"]), "duplicate-command-outcome")
    if report["passed"]:
        require(report["compile_exit_code"] == 0 and len(servers) == 2 and
            all(server["original_joined"] is True for server in servers), "positive-rehearsal-owner-or-build-missing")
        verify_command_outcomes(report["commands"], component)
        require(not report.get("failure") and not report.get("failure_cleanup", {}).get("forced", False), "positive-rehearsal-has-failure")
        required = {"production-restore-and-private-config-preserve-original-trust-validity-and-credential",
            "occupied-instance-refuses-without-overwrite", "wrong-signing-seed-refuses-before-state-publication",
            "incomplete-browser-seed-refuses-without-native-only-downgrade",
            "normal-server-and-native-client-read-original-state-through-materialized-config",
            "seed-unchanged-and-original-server-groups-joined"}
        if component != "materializer":
            required.update({"four-identities-five-independent-scoped-devices-enrolled-through-production-cli",
                "explicit-original-enrollment-resume-preserves-all-ids-keys-credentials-and-expiry",
                "all-five-enrolled-devices-perform-signed-native-read-under-their-own-authority",
                "participant-credential-cannot-provision-an-identity",
                "changed-bound-issuer-refuses-before-public-calls-and-preserves-original-plan",
                "revoked-primary-device-refuses-while-independent-replacement-and-other-devices-remain-usable"})
        require(required <= set(report["checks"]), "positive-rehearsal-checks-missing")
    result = dict(commands_observed=len(report["commands"]), original_servers_joined=sum(v["original_joined"] is True for v in servers))
    if component not in {"conversation", "data", "media"}:
        return result
    raw = members.get(component + "/conversation.json")
    require(raw is not None and hashlib.sha256(raw).hexdigest() == report["conversation_report_sha256"], "conversation-report-binding-mismatch")
    child = decode(raw)
    require(child["format"] == "connect/transport-conversation-control/1", "wrong-conversation-format")
    for field in ("source_revision", "binary_sha256", "signing_algorithm"):
        require(child[field] == report[field], "conversation-runtime-binding-mismatch")
    for field in ("cell_qualified", "task_complete", "whole_gate_qualified"):
        require(child[field] is False, "component-cannot-promote-cell")
    require(type(child["passed"]) is bool, "conversation-result-required")
    if report["passed"]:
        require(child["passed"] and report["conversation_exit_code"] == 0 and not child.get("failure") and
            child["observed_helpers"] == 5 and child["all_observed_helpers_joined"] is True, "positive-conversation-work-or-join-missing")
        require(child["maximum_active_members_observed"] == 4 and child["concurrent_session_intents_observed"] == 2,
            "positive-conversation-workload-missing")
    result.update(conversation_report_sha256=hashlib.sha256(raw).hexdigest(), conversation_passed=child["passed"],
        helpers_observed=child["observed_helpers"], helpers_joined=child["all_observed_helpers_joined"])
    if component in {"data", "media"}:
        if report["passed"]:
            workload = child["data_workload"]
            require(child["actual_connection_workload_observed"] is True and child["actual_topology_verified"] is True and
                child["native_offers_completed"] == child["native_data_submissions"] == child["native_data_completions"] == child["native_data_receives"] == 6 and
                workload["connections"] == 6 and workload["participants"] == 4 and workload["sessions"] == 2 and
                workload["connection_counts_by_session"] == [4, 2] and workload["bytes_per_payload"] == 65536 and
                workload["verified_payloads"] == 6 and workload["original_connections_retired"] == 6 and
                workload["total_received_bytes"] == 6 * 65536, "positive-data-workload-missing")
            capture = report["packet_capture"]
            require(capture["observed"] is True and capture["original_joined"] is True and capture["forced"] is False and
                capture["kernel_dropped_packets"] == 0 and capture["counts"]["to_server_udp_bytes"] >= 6 * 65536 and
                capture["counts"]["from_server_udp_bytes"] >= 6 * 65536, "positive-data-capture-missing")
        result.update(native_offers_completed=child.get("native_offers_completed", 0),
            native_data_submissions=child.get("native_data_submissions", 0),
            native_data_completions=child.get("native_data_completions", 0), native_data_receives=child.get("native_data_receives", 0))
    if component == "media" and report["passed"]:
        decoded = report["native_av_decoded"]
        require(decoded["format"] == "connect/native-av-decoded-component/1" and decoded["passed"] is True and
            decoded["connections"] == 6 and decoded["participants"] == 4 and decoded["sessions"] == 2 and
            decoded["observation_seconds"] == 60 and decoded["warmup_seconds"] == 2 and len(decoded["routes"]) == 6,
            "positive-media-workload-missing")
        require(all(decoded[k] is False for k in ("task_complete", "cell_qualified", "matrix_qualified", "whole_gate_qualified")),
            "media-component-cannot-promote-cell")
        expected_peers = [3, 0, 1, 2, 5, 4]
        require([route["index"] for route in decoded["routes"]] == list(range(6)) and
            [route["peer_index"] for route in decoded["routes"]] == expected_peers,
            "decoded-original-route-set-mismatch")
        source_digest = hashlib.sha256((json.dumps(decoded["source_manifest"], indent=2) + "\n").encode()).hexdigest()
        require(source_digest == report["native_av_source_manifest_sha256"] == child["native_av"]["source_manifest_sha256"] and
            report["endpoint_verification"]["full_media_endpoints_verified"] == 5 and
            report["endpoint_verification"]["unchanged_unverified_issuance_resumes"] == 5,
            "media-source-or-endpoint-binding-missing")
        for route in decoded["routes"]:
            video, audio = route["video"], route["audio"]
            require(route["passed"] is True and audio["all_blocks_identified"] is True and audio["bad_identity_blocks"] == 0 and
                audio["decoded_seconds"] >= 59.9 and route["maximum_audio_arrival_gap_ms"] <= 200 and
                (video["width"], video["height"]) == (640, 360) and video["decoded_window_frames"] >= 1440 and
                video["changing_window_frames"] >= 1440 and len(video["frames_by_observation_second"]) == 60 and
                min(video["frames_by_observation_second"]) >= 24 and video["wrong_source_pixel_frames"] == 0 and
                route["received_clocks"]["audio"] > 0 and route["received_clocks"]["video"] > 0,
                "positive-decoded-media-observation-missing")
        native = child["native_av"]
        require(native["original_receive_tasks_joined"] is True and native["observation_seconds"] == 60 and
            report["packet_capture"]["media_workload"] is True and report["packet_capture"]["packet_limit"] == 200000,
            "media-work-or-capture-owner-missing")
        timings = child["data_workload"].get("media_end_timings")
        if timings is not None:
            require([v["index"] for v in timings] == list(range(6)) and all(v["limit_ms"] == 5000 and
                0 <= v["end_receipt_ms"] <= v["original_retirement_observed_ms"] <= 5000 for v in timings),
                "original-media-end-timing-incomplete")
        result.update(native_av_decoded_routes=6, native_end_timings_verified=0 if timings is None else 6,
            full_media_profile_qualified=False)
    return result


def verify(path, expected_sha256, revision, component, target, signing, network, sdk, prepared, archives, expected_passed):
    require(re.fullmatch(r"[0-9a-f]{40}", revision) is not None, "full-source-revision-required")
    require(component in COMPONENTS, "unknown-component")
    members = read_members(path, expected_sha256)
    report_name = component + ("/control.json" if component == "control" else "/rehearsal.json")
    source_name = component + "/source-inputs.json"
    require(report_name in members and source_name in members, "component-report-or-source-inputs-missing")
    report, inputs = decode(members[report_name]), decode(members[source_name])
    require(report["source_revision"] == revision, "wrong-source-revision")
    require(type(report["passed"]) is bool and report["passed"] is expected_passed, "unexpected-component-result")
    require(hashlib.sha256(members[source_name]).hexdigest() == report["source_inputs_sha256"] and
        report["source_count"] == len(inputs), "source-report-binding-mismatch")
    for field in ("task_complete", "matrix_qualified", "whole_gate_qualified"):
        require(report[field] is False, "component-cannot-promote-matrix")
    for field in ("source_inputs_unchanged", "executables_unchanged"):
        require(report[field] is True, "runtime-input-integrity-unconfirmed")
    verify_prepared_inventory(inputs, revision, prepared)
    functions = runpy.run_path(str(ROOT / "scripts/verify-linux-installation.py"))
    counts, _ = functions["verify_sources"](inputs, revision, Path(prepared), Path(archives) if archives else None)
    contract = decode(subprocess.check_output(["git", "show", revision + ":contracts/gate6-transport-matrix.json"], cwd=ROOT))
    service_hashes = {name: inputs[name]["sha256"] for name in contract["service_contract"]}
    contract_sha = hashlib.sha256(json.dumps(service_hashes, sort_keys=True).encode()).hexdigest()
    for name, raw in members.items():
        if name.endswith("/matrix.json"):
            matrix = decode(raw)
            require(matrix["source_revision"] == revision and matrix["service_contract_sha256"] == contract_sha,
                "matrix-source-or-service-contract-mismatch")
            require(all(matrix[field] is False for field in ("task_complete", "matrix_qualified", "whole_gate_qualified")) and
                matrix["required_cells"] == len(matrix["cells"]) == 32 and
                all(cell["status"] == "not-run" for cell in matrix["cells"]), "component-matrix-cannot-claim-cell-passes")
    host = report.get("host", {})
    if component == "control":
        require(report["service_contract_sha256"] == contract_sha, "control-service-contract-mismatch")
        require((host.get("system"), host.get("machine")) ==
            ({"ubuntu24-x86_64": ("Linux", "x86_64"), "macos-arm64": ("Darwin", "arm64")}[target]), "wrong-runtime-platform")
        require(network == "local" and signing == "both", "control-signing-or-network-scope")
        outcomes = verify_control(report)
        require(outcomes["complete_control_selection"], "complete-control-selection-required")
        binary = report["executable_sha256"]
    else:
        require(host.get("platform") == target, "runtime-platform-observation-required")
        require(report["signing_algorithm"] == signing, "wrong-runtime-signing")
        # Early local reports predate this field; absent remote proof cannot be
        # accepted as a Remote result. The materialized cell remains binding.
        require(report.get("network_profile", "local") == ("local" if network == "local" else "remote"), "wrong-runtime-network")
        materialized = report["materialization"]
        require(materialized["candidate_source_revision"] == revision and
            materialized["candidate_binary_sha256"] == report["binary_sha256"] and
            materialized["service_contract_sha256"] == contract_sha, "materialized-source-or-binary-mismatch")
        require(materialized["cell_id"] == "G6-058/" + target + "/" + network + "/native-quic/" + signing,
            "wrong-materialized-cell")
        if network != "local" and report["passed"]:
            owned = report["network_namespace"]
            require(owned["passed"] is True and owned["forced_cleanup"] is False and owned["systemd_servers_verified"] == 2 and
                owned["original_namespace_removed"] is True and owned["original_veth_removed"] is True and
                owned["remaining_client_processes"] == 0 and not owned["failures"] and
                all(v.get("managed_by_systemd") is True for v in report["servers"]), "remote-owned-runtime-or-cleanup-missing")
        outcomes = verify_rehearsal(report, component, members)
        binary = report["binary_sha256"]
    require(re.fullmatch(r"[0-9a-f]{64}", binary) is not None, "runtime-binary-digest-missing")
    sdk_count = 0
    if component in {"control", "conversation", "data", "media"}:
        require(report["sdk_dist_inputs_unchanged"] is True, "sdk-integrity-unconfirmed")
        sdk_count = verify_sdk(report["sdk_dist_inputs"], sdk)
    return dict(format="connect/transport-component-independent-verification/1", evidence_verified=True,
        component=component, component_passed=report["passed"], source_revision=revision, artifact_sha256=expected_sha256,
        report_sha256=hashlib.sha256(members[report_name]).hexdigest(), source_inputs_sha256=report["source_inputs_sha256"],
        sources_verified=counts, sdk_dist_files_verified=sdk_count, target=target, signing=signing, network=network,
        reported_runtime_binary_sha256=binary, binary_bytes_in_artifact=False, outcomes=outcomes,
        full_matrix_cells_qualified=0, task_complete=False, whole_gate_qualified=False)
