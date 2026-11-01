"""A component, omitted browser, stale candidate or private field cannot qualify a matrix."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts/lib"))
import transport_matrix as matrix


class MatrixBoundary(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.path = self.root / "contracts/gate6-transport-matrix.json"
        self.contract = json.loads((ROOT / "contracts/gate6-transport-matrix.json").read_text())
        for name in [self.contract["profile"], *self.contract["service_contract"], *self.contract["fixture_templates"]]:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("isolated test input\n")
        self.save()

    def save(self):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.path.write_text(json.dumps(self.contract))

    def reference(self, report):
        return dict(format="connect/transport-control-component/1", source_revision=report["source_revision"],
            service_contract_sha256=report["service_contract_sha256"], report_sha256="a" * 64,
            source_inputs_sha256="b" * 64, executable_sha256="c" * 64,
            selected_cases=list(matrix.CONTROL_CASES), passed=True, all_cases_executed=True)

    def test_successful_component_never_qualifies_any_of_32_required_cells(self):
        report = matrix.inventory(self.root, "d" * 40)
        result = matrix.attach_control(report, self.reference(report))
        self.assertEqual(len(result["cells"]), 32)
        self.assertEqual(len({c["id"] for c in result["cells"]}), 32)
        self.assertTrue(result["supporting_control"]["passed"])
        self.assertFalse(result["matrix_qualified"])
        self.assertFalse(result["task_complete"])
        self.assertTrue(all(c["status"] == "not-run" and c["decoded_media"] is None for c in result["cells"]))

    def test_omitted_or_duplicated_browser_or_algorithm_cannot_shrink_matrix(self):
        original = copy.deepcopy(self.contract)
        for key, value in [("transport", ["native-quic", "chrome-webtransport", "chrome-wss-webrtc"]),
                ("signing", ["EdDSA", "EdDSA"])]:
            with self.subTest(dimension=key):
                self.contract = copy.deepcopy(original)
                self.contract["dimensions"][key] = value
                self.save()
                with self.assertRaisesRegex(ValueError, "omitted or duplicated"):
                    matrix.inventory(self.root, "d" * 40)

    def test_media_or_movement_requirement_cannot_be_replaced_with_a_component_check(self):
        for name in ("independently-decoded-duplex-media-and-profile-budgets", "device-replacement-and-old-owner-retirement"):
            with self.subTest(requirement=name):
                original = self.contract["required_checks"].copy()
                self.contract["required_checks"][original.index(name)] = "control-component-passed"
                self.save()
                with self.assertRaisesRegex(ValueError, "behavior checklist"):
                    matrix.inventory(self.root, "d" * 40)
                self.contract["required_checks"] = original

    def test_wrong_candidate_contract_or_unexpected_private_field_refuses(self):
        report = matrix.inventory(self.root, "d" * 40)
        for key, value in [("source_revision", "e" * 40), ("service_contract_sha256", "f" * 64),
                ("credential", "private-must-not-be-adopted"), ("selected_cases", ["media::unapproved"]),
                ("all_cases_executed", False)]:
            with self.subTest(field=key):
                reference = self.reference(report)
                reference[key] = value
                with self.assertRaises(ValueError):
                    matrix.attach_control(report, reference)
        self.assertIsNone(report["supporting_control"])

    def test_control_cannot_adopt_prepopulated_cell_passes(self):
        report = matrix.inventory(self.root, "d" * 40)
        report["cells"][0]["status"] = "passed"
        with self.assertRaisesRegex(ValueError, "adopt completed cells"):
            matrix.attach_control(report, self.reference(report))

    def test_http_reference_cannot_be_omitted_from_shared_contract(self):
        self.contract["service_contract"].remove("contracts/openapi-canonical-http.json")
        self.save()
        with self.assertRaisesRegex(ValueError, "HTTP reference omitted"):
            matrix.inventory(self.root, "d" * 40)


if __name__ == "__main__":
    unittest.main()
