"""Non-AV scheduling must neither stop at media nor invent missing coverage."""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from lib.transport_matrix import inventory
from lib.transport_matrix_execution import AV_CHECK, configure_execution, record_native_result

spec = importlib.util.spec_from_file_location("driver", ROOT / "scripts/run-gate6-transport-matrix.py")
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)


class NonAvMatrix(unittest.TestCase):
    def matrix(self):
        return configure_execution(inventory(ROOT, "a" * 40), dict(server="ubuntu24-arm64",
            browsers={name: dict(matches_frozen_version=True) for name in ("chrome", "firefox")},
            owned_systemd_vm=True, execution_blockers=[], browser_runtime_image_id="sha256:" + "b" * 64), "non-av")

    def report(self, cell):
        return dict(format="connect/transport-fixture-rehearsal/1", source_revision="a" * 40,
            host=dict(platform=cell["server"]), signing_algorithm=cell["signing"],
            network_profile="local" if cell["network"] == "local" else "remote",
            passed=True, source_inputs_unchanged=True, executables_unchanged=True, sdk_dist_inputs_unchanged=True,
            conversation_control=dict(passed=True, all_observed_helpers_joined=True,
                workload_mode="six-native-data-connections", maximum_active_members_observed=4,
                concurrent_session_intents_observed=2, actual_connection_workload_observed=True,
                actual_topology_verified=True, checks=[
                    "scoped-native-conversation-and-nonmember-current-access-refusal",
                    "four-active-participants-and-addition-does-not-expand-older-message-audience",
                    "membership-update-original-replay-changed-intent-and-stale-revision-guards",
                    "remove-and-readmit-preserve-participant-id-and-fence-old-epoch-history-and-replay",
                    "two-current-accepted-session-intents-coexist-with-one-main-session",
                    "additive-session-capability-intent-and-unoffered-video-refusal-without-state-change"]))

    def test_all_cells_visible_and_available_native_browser_cohorts_selected(self):
        matrix = self.matrix()
        self.assertEqual(len(matrix["cells"]), 32)
        ready = [c for c in matrix["cells"] if c["execution"]["runnable"]]
        self.assertEqual(len(ready), 8)
        self.assertEqual(sum(c["execution"]["producer"] == "native-data" for c in ready), 4)
        self.assertEqual(sum(c["execution"]["producer"] == "browser-admission" for c in ready), 4)
        for cell in matrix["cells"]:
            self.assertEqual(cell["check_results"][AV_CHECK]["status"], "not-run")
            self.assertIn("outside-selected-non-av-stage", cell["check_results"][AV_CHECK]["reason"])
            self.assertEqual(set(cell["non_av_scope"]["required_checks"]), set(cell["required_checks"]) - {AV_CHECK})
            self.assertFalse(cell["non_av_scope"]["qualified"])

    def test_first_native_failure_does_not_stop_other_signing_or_remote_cohorts(self):
        matrix = self.matrix()
        cells = [c for c in matrix["cells"] if c["execution"]["runnable"] and c["transport"] == "native-quic"]
        visited = []

        def execute(cell):
            visited.append(cell["id"])
            if len(visited) == 1:
                raise RuntimeError("original fixture failure")
            return self.report(cell), "c" * 64, 0

        with tempfile.TemporaryDirectory() as directory:
            driver.execute_selected(matrix, [c["id"] for c in cells], Path(directory), execute)
        self.assertEqual(len(visited), 4)
        self.assertEqual([c["status"] for c in cells], ["failed", "incomplete", "incomplete", "incomplete"])
        self.assertFalse(matrix["matrix_qualified"])

    def test_only_complete_observed_control_clauses_pass(self):
        matrix = self.matrix()
        cell = next(c for c in matrix["cells"] if c["execution"]["runnable"] and c["transport"] == "native-quic")
        record_native_result(matrix, cell, self.report(cell), "c" * 64, 0)
        passed = {name for name, result in cell["check_results"].items() if result["status"] == "passed"}
        self.assertEqual(passed, {"participant-add-update-remove-and-current-audiences", "session-upgrade-and-current-capability-refusal"})
        self.assertFalse(cell["non_av_scope"]["qualified"] or matrix["task_complete"])
        self.assertEqual(cell["execution"]["component"]["kind"], "native-data")

    def test_wrong_workload_missing_observation_and_broken_guard_never_promote(self):
        for change in (lambda r: r.update(executables_unchanged=False),
                       lambda r: r["conversation_control"].update(workload_mode="control-only"),
                       lambda r: r["conversation_control"].update(actual_topology_verified=False),
                       lambda r: r["conversation_control"].update(checks=[])):
            matrix = self.matrix()
            cell = next(c for c in matrix["cells"] if c["execution"]["runnable"] and c["transport"] == "native-quic")
            report = self.report(cell)
            change(report)
            record_native_result(matrix, cell, report, "c" * 64, 0)
            self.assertFalse(any(r["status"] == "passed" for r in cell["check_results"].values()))


if __name__ == "__main__":
    unittest.main()
