"""Full scheduling and independent verdicts, including negative evidence."""
import copy
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from lib.transport_matrix import inventory
from lib.transport_matrix_execution import configure_execution, merge_hosts
from lib.transport_matrix_verdict import evaluate_cell, summarize

loader = importlib.util.spec_from_file_location("matrix_full_driver", ROOT / "scripts/run-gate6-transport-matrix.py")
driver = importlib.util.module_from_spec(loader)
loader.loader.exec_module(driver)


class FullMatrix(unittest.TestCase):
    def configured(self, host):
        return configure_execution(inventory(ROOT, "a" * 40),
            dict(server=host, execution_blockers=[], browsers={}), "full")

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
            dict(server="ubuntu24-arm64", execution_blockers=["missing-tool-tcpdump"], browsers={}), "full")
        with tempfile.TemporaryDirectory() as temp:
            driver.execute_selected(matrix, [c["id"] for c in matrix["cells"]], Path(temp),
                                    lambda _: self.fail("Blocked environment launched a workload"))
        summary = summarize(matrix)
        self.assertEqual(summary["cells"]["blocked"], 16)
        self.assertEqual(summary["checks"]["failed"], 0)
        self.assertTrue(all(not c["execution"]["attempted"] for c in matrix["cells"]))


if __name__ == "__main__":
    unittest.main()
