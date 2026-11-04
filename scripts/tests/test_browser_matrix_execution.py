"""Keep browser stage outcomes inside their actual matrix cell and scope."""
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from lib.transport_matrix import inventory
from lib.transport_matrix_execution import configure_execution, record_browser_result


class BrowserMatrixExecution(unittest.TestCase):
    def matrix(self, image="sha256:" + "b" * 64):
        return configure_execution(inventory(ROOT, "a" * 40), dict(server="ubuntu24-arm64",
            browsers={name: dict(matches_frozen_version=True) for name in ("chrome", "firefox")},
            owned_systemd_vm=True, execution_blockers=[], browser_runtime_image_id=image), "admission")

    def result(self, matrix, cell):
        return dict(format="connect/browser-matrix-component/1", stage="admission", cell_id=cell["id"],
            source_revision="a" * 40, server_platform="ubuntu24-arm64", client_platform="ubuntu24-arm64",
            transport=cell["transport"], signing_algorithm=cell["signing"],
            browser_image_id=matrix["execution_environment"]["browser_runtime_image_id"],
            passed=True, cell_qualified=False, full_media_programme_executed=False,
            source_inputs_unchanged=True, sdk_dist_inputs_unchanged=True, executable_unchanged=True,
            browser_owner_joined=True, browser_observation=dict(source_revision="a" * 40,
                transport=cell["transport"], signing_algorithm=cell["signing"],
                passed=True, authenticated_owners=7, actual_connections=6))

    def test_admission_selection_cannot_launch_native_remote_or_unimplemented_firefox_cells(self):
        matrix = self.matrix()
        ready = [c for c in matrix["cells"] if c["execution"]["runnable"]]
        self.assertEqual(len(ready), 4)
        self.assertEqual({(c["transport"], c["signing"]) for c in ready}, {
            (transport, signing) for transport in ("chrome-webtransport", "chrome-wss-webrtc")
            for signing in ("EdDSA", "ES256")})
        self.assertTrue(all(c["server"] == "ubuntu24-arm64" and c["network"] == "local" for c in ready))
        self.assertTrue(all(c["execution"]["completion_blockers"] for c in ready))
        self.assertEqual(len(matrix["cells"]), 32)
        self.assertFalse(any(c["execution"]["runnable"] for c in self.matrix(None)["cells"]))

    def test_browser_admission_success_leaves_every_full_cell_requirement_unverified(self):
        matrix = self.matrix()
        cell = next(c for c in matrix["cells"] if c["execution"]["runnable"])
        report = self.result(matrix, cell)
        record_browser_result(matrix, cell, report, "c" * 64, 0)
        self.assertEqual(cell["status"], "incomplete")
        self.assertTrue(cell["execution"]["component"]["passed"])
        self.assertTrue(all(r["status"] == "not-run" for r in cell["check_results"].values()))
        self.assertFalse(matrix["task_complete"] or matrix["matrix_qualified"])
        with self.assertRaises(ValueError):
            record_browser_result(matrix, cell, report, "c" * 64, 0)

    def test_browser_evidence_cannot_cross_source_image_transport_signing_or_scope(self):
        for field, value in (("source_revision", "d" * 40), ("browser_image_id", "sha256:" + "e" * 64),
                ("transport", "native-quic"), ("signing_algorithm", "wrong"), ("cell_id", "other"),
                ("stage", "media"), ("full_media_programme_executed", True), ("cell_qualified", True)):
            matrix = self.matrix()
            cell = next(c for c in matrix["cells"] if c["execution"]["runnable"])
            report = self.result(matrix, cell)
            report[field] = value
            with self.assertRaises(ValueError):
                record_browser_result(matrix, cell, report, "c" * 64, 0)
            self.assertIsNone(cell["execution"]["component"])

    def test_unjoined_owner_changed_inputs_or_original_exit_failure_cannot_be_green(self):
        for field in ("source_inputs_unchanged", "sdk_dist_inputs_unchanged", "executable_unchanged", "browser_owner_joined", "exit"):
            matrix = self.matrix()
            cell = next(c for c in matrix["cells"] if c["execution"]["runnable"])
            report = self.result(matrix, cell)
            report[field] = False
            record_browser_result(matrix, cell, report, "c" * 64, 1 if field == "exit" else 0)
            self.assertEqual(cell["status"], "failed")
            self.assertFalse(cell["execution"]["component"]["passed"])


if __name__ == "__main__":
    unittest.main()
