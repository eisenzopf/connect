"""Matrix scheduling must preserve failed, missing and mismatched cell evidence."""
import copy
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from lib.transport_matrix import inventory
from lib.transport_matrix_execution import configure_execution, merge_hosts, record_native_result, select_cells

loader = importlib.util.spec_from_file_location("matrix_driver", ROOT / "scripts/run-gate6-transport-matrix.py")
driver = importlib.util.module_from_spec(loader)
loader.loader.exec_module(driver)


class MatrixExecution(unittest.TestCase):
    def matrix(self, host="ubuntu24-arm64"):
        return configure_execution(inventory(ROOT, "a" * 40), dict(server=host,
            browsers={name: dict(matches_frozen_version=True) for name in ("chrome", "firefox")},
            owned_systemd_vm=False, execution_blockers=[]))

    def result(self, cell, passed=True):
        return dict(format="connect/transport-fixture-rehearsal/1", source_revision="a" * 40,
                    host=dict(platform=cell["server"]), signing_algorithm=cell["signing"],
                    network_profile="local", passed=passed, source_inputs_unchanged=True,
                    executables_unchanged=True, sdk_dist_inputs_unchanged=True)

    def local_cells(self, matrix):
        return [c for c in matrix["cells"] if c["execution"]["runnable"]]

    def test_all_32_cells_remain_visible_when_only_two_workloads_are_runnable(self):
        matrix = self.matrix()
        self.assertEqual(len(matrix["cells"]), 32)
        ready = self.local_cells(matrix)
        self.assertEqual(len(ready), 2)
        self.assertEqual({c["signing"] for c in ready}, {"EdDSA", "ES256"})
        self.assertTrue(all(c["transport"] == "native-quic" and c["network"] == "local" for c in ready))
        self.assertTrue(all(c["execution"]["completion_blockers"] for c in matrix["cells"]))
        self.assertFalse(matrix["matrix_qualified"])

    def test_first_failure_does_not_prevent_second_cell_execution_or_qualify_it(self):
        for first_raises in (False, True):
            matrix = self.matrix()
            cells = self.local_cells(matrix)
            visited = []

            def execute(cell):
                visited.append(cell["id"])
                if len(visited) == 1 and first_raises:
                    raise RuntimeError("fixture refused before report")
                passed = len(visited) == 2
                return self.result(cell, passed), "b" * 64, 0 if passed else 1

            with tempfile.TemporaryDirectory() as temp:
                driver.execute_selected(matrix, [c["id"] for c in cells], Path(temp), execute)
                saved = json.loads((Path(temp) / "matrix.json").read_text())
                self.assertEqual(len(saved["cells"]), 32)
                self.assertFalse(saved["matrix_qualified"])
            self.assertEqual(visited, [c["id"] for c in cells])
            self.assertEqual([c["status"] for c in cells], ["failed", "incomplete"])
            self.assertTrue(all(r["status"] == "not-run" for c in cells for r in c["check_results"].values()))

    def test_native_evidence_cannot_cross_candidate_signing_platform_or_network(self):
        for field, value in (("source_revision", "c" * 40), ("signing_algorithm", "ES256"),
                             ("host", {"platform": "macos-arm64"}), ("network_profile", "remote")):
            matrix = self.matrix()
            cell = next(c for c in self.local_cells(matrix) if c["signing"] == "EdDSA")
            result = self.result(cell)
            result[field] = value
            with self.assertRaises(ValueError):
                record_native_result(matrix, cell, result, "b" * 64, 0)
            self.assertIsNone(cell["execution"]["component"])
        cell = next(c for c in matrix["cells"] if c["transport"] == "chrome-webtransport")
        with self.assertRaises(ValueError):
            record_native_result(matrix, cell, self.result(cell), "b" * 64, 0)

    def test_broken_integrity_and_terminal_exit_cannot_be_hidden_by_passed_flag(self):
        for field in ("source_inputs_unchanged", "executables_unchanged", "sdk_dist_inputs_unchanged", "exit_code"):
            matrix = self.matrix()
            cell = self.local_cells(matrix)[0]
            result = self.result(cell)
            result[field] = False
            record_native_result(matrix, cell, result, "b" * 64, 1 if field == "exit_code" else 0)
            self.assertEqual(cell["status"], "failed")
            with self.assertRaises(ValueError):
                record_native_result(matrix, cell, self.result(cell), "b" * 64, 0)

    def test_merge_keeps_hosts_distinct_and_rejects_stale_duplicate_or_omitted_rows(self):
        base = inventory(ROOT, "a" * 40)
        mac, linux = self.matrix("macos-arm64"), self.matrix()
        cell = self.local_cells(linux)[0]
        record_native_result(linux, cell, self.result(cell, False), "b" * 64, 1)
        merged = merge_hosts(base, [mac, linux])
        self.assertEqual(len(merged["cells"]), 32)
        self.assertEqual([c["id"] for c in merged["cells"] if c["status"] == "failed"], [cell["id"]])
        self.assertFalse(merged["matrix_qualified"])
        self.assertEqual(merged["missing_host_reports"], [])
        with self.assertRaises(ValueError):
            merge_hosts(base, [mac, mac])
        for mutation in (lambda r: r.update(source_revision="d" * 40),
                         lambda r: r["cells"].pop(),
                         lambda r: r["cells"][0].update(status="passed"),
                         lambda r: r["cells"][0]["required_checks"].pop()):
            invalid = copy.deepcopy(linux)
            mutation(invalid)
            with self.assertRaises(ValueError):
                merge_hosts(base, [mac, invalid])

    def test_no_implicit_all_or_duplicate_attempt_selection(self):
        matrix = self.matrix()
        name = self.local_cells(matrix)[0]["id"]
        for ids in ([], [name, name], ["not-a-cell"]):
            with self.assertRaises(ValueError):
                select_cells(matrix, ids)


if __name__ == "__main__":
    unittest.main()
