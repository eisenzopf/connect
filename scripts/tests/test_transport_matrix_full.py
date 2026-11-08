"""Full scheduling and independent verdicts, including negative evidence."""
import copy
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from lib.transport_matrix import inventory, load_contract
from lib.transport_matrix_execution import configure_execution, merge_hosts
from lib.transport_matrix_verdict import evaluate_cell, summarize

loader = importlib.util.spec_from_file_location("matrix_full_driver", ROOT / "scripts/run-gate6-transport-matrix.py")
driver = importlib.util.module_from_spec(loader)
loader.loader.exec_module(driver)


class FullMatrix(unittest.TestCase):
    def configured(self, host):
        return configure_execution(inventory(ROOT, "a" * 40),
            dict(server=host, execution_blockers=[], browsers={}), "full")

    def test_browser_overrides_belong_to_actual_client_host_not_remote_mac_server(self):
        for host in ("macos-arm64", "ubuntu24-arm64"):
            args = SimpleNamespace(chrome=Path("/host-specific/chrome"), firefox=Path("/host-specific/firefox"))
            expected = ["--chrome", str(args.chrome), "--firefox", str(args.firefox)]
            for cell in self.configured(host)["cells"]:
                if cell["server"] != host or cell["transport"] == "native-quic":
                    continue
                with self.subTest(cell=cell["id"]):
                    remote_mac = host == "macos-arm64" and cell["network"] == "remote-profile-fixture"
                    self.assertEqual(driver.browser_runtime_arguments(cell, args), [] if remote_mac else expected)
                    self.assertEqual(driver.browser_runtime_arguments(cell, SimpleNamespace(chrome=None, firefox=None)), [])

    def test_every_transport_and_network_gets_a_real_executor_on_its_host(self):
        all_ids = set()
        for host in ("macos-arm64", "ubuntu24-arm64"):
            matrix = self.configured(host)
            cells = [c for c in matrix["cells"] if c["execution"]["runnable"]]
            self.assertEqual(len(cells), 16)
            self.assertEqual({c["transport"] for c in cells},
                {"native-quic", "chrome-webtransport", "chrome-wss-webrtc", "firefox-wss-webrtc"})
            self.assertTrue(all(c["execution"]["executor"] and c["execution"]["verifier"] for c in cells))
            self.assertFalse(all_ids & {c["id"] for c in cells})
            all_ids.update(c["id"] for c in cells)
        self.assertEqual(len(all_ids), 32)

    def test_first_exception_does_not_stop_any_other_cell_and_no_check_disappears(self):
        reports = []
        visited = []
        for host in ("macos-arm64", "ubuntu24-arm64"):
            matrix = self.configured(host)
            def execute(cell):
                visited.append(cell["id"])
                if len(visited) == 1:
                    raise RuntimeError("original failed fixture")
                return {}, "b" * 64, 1
            with tempfile.TemporaryDirectory() as temp:
                driver.execute_selected(matrix, [c["id"] for c in matrix["cells"]], Path(temp), execute)
            reports.append(matrix)
        merged = merge_hosts(inventory(ROOT, "a" * 40), reports)
        self.assertEqual(len(visited), len(set(visited)))
        self.assertEqual(len(visited), 32)
        self.assertEqual(sum(len(c["check_results"]) for c in merged["cells"]), 432)
        self.assertEqual(merged["summary"]["checks"]["not-run"], 0)
        self.assertEqual(merged["summary"]["checks"]["passed"], 0)
        self.assertFalse(merged["matrix_qualified"])

    def test_prospective_media_quality_requires_explicit_current_metric_scope(self):
        matrix=self.configured("ubuntu24-arm64")
        cell=next(c for c in matrix["cells"] if c["execution"]["runnable"] and c["transport"]=="chrome-webtransport")
        audio=dict(quality_profile="connect/decoded-audio-quality/3",metric_scope="measurable-decoded-quality-source-quanta-diagnostic",
            source_quantum_scope="diagnostic-only-source-clock-attribution",original_source_sample_coverage_proved=False)
        decoded=dict(passed=True,quality_profile=audio['quality_profile'],metric_scope=audio['metric_scope'],
            original_source_sample_coverage_proved=False,routes=[dict(passed=True,audio=dict(audio)) for _ in range(6)])
        report=dict(source_revision="a"*40,server_platform=cell["server"],cell_id=cell["id"],signing_algorithm=cell["signing"],
            source_inputs_unchanged=True,sdk_dist_inputs_unchanged=True,executable_unchanged=True,
            browser_observation=dict(decoded_media=decoded))
        name="independently-decoded-duplex-media-and-profile-budgets"
        evaluate_cell(cell,report,"b"*64,environment=matrix["execution_environment"],revision="a"*40)
        self.assertEqual(cell["check_results"][name]["status"],"passed")
        for field,value in [('quality_profile','historical'),('metric_scope','original-samples-proved'),('original_source_sample_coverage_proved',True)]:
            changed=copy.deepcopy(report);changed['browser_observation']['decoded_media'][field]=value
            evaluate_cell(cell,changed,"b"*64,environment=matrix["execution_environment"],revision="a"*40)
            with self.subTest(field=field):self.assertEqual(cell["check_results"][name]["status"],"failed")
        changed=copy.deepcopy(report);del changed['browser_observation']['decoded_media']['routes'][0]['audio']['source_quantum_scope']
        evaluate_cell(cell,changed,"b"*64,environment=matrix["execution_environment"],revision="a"*40)
        self.assertEqual(cell["check_results"][name]["status"],"failed")
        import json
        original=Path.read_text
        contract=json.loads((ROOT/'contracts/gate6-transport-matrix.json').read_text())
        for field,value in [('quality_profile','historical'),('metric_scope','original-samples-proved'),('owner_amendment',None)]:
            bad=copy.deepcopy(contract);bad['media_quality_profile'][field]=value
            def read(path,*args,**kwargs):
                return json.dumps(bad) if path==ROOT/'contracts/gate6-transport-matrix.json' else original(path,*args,**kwargs)
            with self.subTest(contract_field=field),patch.object(Path,'read_text',read),self.assertRaises(ValueError):
                load_contract(ROOT)

    def test_later_av_failure_preserves_current_control_passes(self):
        matrix = self.configured("ubuntu24-arm64")
        cell = next(c for c in matrix["cells"] if c["execution"]["runnable"] and c["transport"] == "chrome-webtransport")
        report = dict(source_revision="a" * 40, server_platform=cell["server"], cell_id=cell["id"],
            signing_algorithm=cell["signing"], source_inputs_unchanged=True, executable_unchanged=True,
            sdk_dist_inputs_unchanged=True, passed=False, browser_observation=dict(passed=False,
                admission=dict(checks=["participant-add-update-remove-rejoin-and-current-addressed-history",
                    "two-concurrent-accepted-sessions-upgraded-to-audio-video-and-unoffered-screen-refused"]),
                decoded_media=dict(passed=False)))
        evaluate_cell(cell, report, "b" * 64, environment=matrix["execution_environment"], revision="a" * 40)
        self.assertEqual(cell["check_results"]["participant-add-update-remove-and-current-audiences"]["status"], "passed")
        self.assertEqual(cell["check_results"]["independently-decoded-duplex-media-and-profile-budgets"]["status"], "failed")
        self.assertEqual(cell["status"], "failed")
        changed = copy.deepcopy(report)
        changed["signing_algorithm"] = "wrong"
        evaluate_cell(cell, changed, "c" * 64, environment=matrix["execution_environment"], revision="a" * 40)
        self.assertEqual(cell["check_results"]["participant-add-update-remove-and-current-audiences"]["status"], "failed")

    def test_environment_block_is_not_a_failed_or_executed_behavior_check(self):
        matrix = configure_execution(inventory(ROOT, "a" * 40),
            dict(server="ubuntu24-arm64", execution_blockers=["missing-tool-cargo"], browsers={}), "full")
        with tempfile.TemporaryDirectory() as temp:
            driver.execute_selected(matrix, [c["id"] for c in matrix["cells"]], Path(temp),
                                    lambda _: self.fail("Blocked environment launched a workload"))
        summary = summarize(matrix)
        self.assertEqual(summary["cells"]["blocked"], 16)
        self.assertEqual(summary["checks"]["failed"], 0)
        self.assertTrue(all(not c["execution"]["attempted"] for c in matrix["cells"]))

    def test_missing_macos_capture_does_not_gate_independent_requirements(self):
        def version(command):
            return {"rustc": "rustc 1.97.1 (fixture)", "node": "v22.16.0"}.get(command[0], "1.0")
        with patch.object(driver.platform, "system", return_value="Darwin"), \
                patch.object(driver.platform, "machine", return_value="arm64"), \
                patch.object(driver, "command_version", side_effect=version), \
                patch.object(driver.shutil, "which", side_effect=lambda name: None if name == "tcpdump" else "/fixture/" + name), \
                patch.object(driver.shutil, "disk_usage", return_value=SimpleNamespace(free=10 * 1024**3)), \
                patch.object(driver.os, "access", return_value=False):
            environment = driver.probe()
        self.assertEqual(environment["execution_blockers"], [])
        self.assertEqual(environment["capture_blockers"],
                         ["missing-tool-tcpdump", "macos-bpf-capture-permission-unavailable"])
        matrix = configure_execution(inventory(ROOT, "a" * 40), environment, "full")
        self.assertEqual(sum(c["execution"]["runnable"] for c in matrix["cells"]), 16)
        cell = next(c for c in matrix["cells"] if c["execution"]["runnable"])
        report = dict(source_revision="a" * 40, host=dict(platform=cell["server"]),
            network_profile="local", signing_algorithm=cell["signing"],
            source_inputs_unchanged=True, sdk_dist_inputs_unchanged=True, executables_unchanged=True,
            packet_capture=dict(observed=False, environment_failure=True, failure="PermissionError"),
            conversation_control=dict(actual_connection_workload_observed=True,
                maximum_active_members_observed=4, concurrent_session_intents_observed=2))
        evaluate_cell(cell, report, "b" * 64, environment=environment, revision="a" * 40)
        self.assertEqual(cell["check_results"]["declared-concurrent-workload"]["status"], "passed")
        self.assertEqual(cell["check_results"]["actual-private-packet-capture"]["status"], "failed")
        self.assertEqual(cell["check_results"]["actual-private-packet-capture"]["category"], "environment")
        self.assertEqual(cell["status"], "failed")

    def test_early_native_setup_failure_keeps_sealed_identity_independent(self):
        matrix = self.configured("macos-arm64")
        cell = next(c for c in matrix["cells"] if c["execution"]["runnable"]
                    and c["network"] == "remote-profile-fixture" and c["transport"] == "native-quic")
        report = dict(source_revision="a" * 40, host=dict(platform=cell["server"]),
            network_profile="remote", signing_algorithm=cell["signing"],
            source_inputs_unchanged=True, sdk_dist_inputs_unchanged=True, executables_unchanged=True,
            binary_sha256="c" * 64, runtime_versions=dict(passed=True),
            failure=dict(phase="seed-create", name="AssertionError"), native_programme_started=False)
        evaluate_cell(cell, report, "b" * 64, environment=matrix["execution_environment"], revision="a" * 40)
        self.assertEqual(cell["check_results"]["identified-candidate-client-server-versions"]["status"], "passed")
        self.assertEqual(cell["check_results"]["declared-concurrent-workload"]["status"], "blocked")
        self.assertNotEqual(cell["status"], "passed")
        report["sdk_dist_inputs_unchanged"] = False
        evaluate_cell(cell, report, "d" * 64, environment=matrix["execution_environment"], revision="a" * 40)
        self.assertEqual(cell["check_results"]["identified-candidate-client-server-versions"]["status"], "failed")

    def test_native_data_replay_and_prior_controls_cannot_substitute_original_helper_recovery(self):
        matrix = self.configured("macos-arm64")
        cell = next(c for c in matrix["cells"] if c["execution"]["runnable"]
                    and c["network"] == "local" and c["transport"] == "native-quic")
        recovery = ["independent-device-explicitly-activates-original-config-after-original-sdk-owner-joins",
                    "independent-device-recovers-original-create-and-history-after-original-helper-close"]
        controls = ["scoped-native-conversation-and-nonmember-current-access-refusal",
                    "four-active-participants-and-addition-does-not-expand-older-message-audience",
                    "membership-update-original-replay-changed-intent-and-stale-revision-guards",
                    "remove-and-readmit-preserve-participant-id-and-fence-old-epoch-history-and-replay",
                    "two-current-accepted-session-intents-coexist-with-one-main-session",
                    "additive-session-capability-intent-and-unoffered-screen-refusal-without-state-change",
                    "native-data-original-replay-recovers-receipt-without-second-delivery"]
        original = dict(source_revision="a" * 40, host=dict(platform=cell["server"]), network_profile="local",
            signing_algorithm=cell["signing"], source_inputs_unchanged=True,
            sdk_dist_inputs_unchanged=True, executables_unchanged=True, passed=False,
            conversation_control=dict(passed=False, checks=controls),
            movement=dict(passed=False, original_moves_completed=0, moves=[]), native_av_decoded=dict(passed=False))
        for absent in [None, *recovery]:
            report = copy.deepcopy(original)
            report["conversation_control"]["checks"] += [] if absent is None else [v for v in recovery if v != absent]
            evaluate_cell(cell, report, "b" * 64, environment=matrix["execution_environment"], revision="a" * 40)
            self.assertEqual(cell["check_results"]["one-service-contract-and-original-operation-recovery"]["status"], "failed")
            self.assertEqual(cell["check_results"]["participant-add-update-remove-and-current-audiences"]["status"], "passed")
            self.assertEqual(cell["check_results"]["session-upgrade-and-current-capability-refusal"]["status"], "passed")
            self.assertNotEqual(cell["status"], "passed")
        report = copy.deepcopy(original)
        report["conversation_control"]["checks"] += recovery
        evaluate_cell(cell, report, "b" * 64, environment=matrix["execution_environment"], revision="a" * 40)
        self.assertEqual(cell["check_results"]["one-service-contract-and-original-operation-recovery"]["status"], "passed")
        self.assertEqual(cell["check_results"]["device-replacement-and-old-owner-retirement"]["status"], "failed")
        self.assertEqual(cell["check_results"]["independently-decoded-duplex-media-and-profile-budgets"]["status"], "failed")
        self.assertNotEqual(cell["check_results"]["current-revocation-expiry-and-conflicting-operation-refusals"]["status"], "passed")
        self.assertEqual(cell["status"], "failed", "Successful original control recovery never qualifies later AV failure")
        report["sdk_dist_inputs_unchanged"] = False
        evaluate_cell(cell, report, "c" * 64, environment=matrix["execution_environment"], revision="a" * 40)
        self.assertEqual(cell["check_results"]["one-service-contract-and-original-operation-recovery"]["status"], "failed")
        self.assertEqual(cell["check_results"]["one-service-contract-and-original-operation-recovery"]["category"], "integrity")


if __name__ == "__main__":
    unittest.main()
