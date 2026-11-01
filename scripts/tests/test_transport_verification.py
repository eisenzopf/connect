"""Adversarial evidence checks; no artifact code or transport is executed."""
import copy
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
import warnings
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.transport_verification import decode, read_members, verify_control, verify_rehearsal, verify_sdk


class EvidenceVerification(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.directory = Path(temp.name)

    def archive(self, entries):
        path = self.directory / "artifact.zip"
        with warnings.catch_warnings(), zipfile.ZipFile(path, "w") as archive:
            warnings.simplefilter("ignore", UserWarning)
            for name, raw in entries:
                archive.writestr(name, raw)
        return path, hashlib.sha256(path.read_bytes()).hexdigest()

    def test_changed_zip_is_refused_before_json_is_read(self):
        path, digest = self.archive([("data/rehearsal.json", b"not even JSON")])
        with path.open("ab") as stream:
            stream.write(b"changed archive")
        with self.assertRaisesRegex(ValueError, "artifact-digest-mismatch"):
            read_members(path, digest)

    def test_duplicate_and_unsafe_members_cannot_shadow_a_curated_report(self):
        for entries in [[("data/rehearsal.json", b"{}"), ("data/rehearsal.json", b"{}")],
                [("../rehearsal.json", b"{}")], [("/data/rehearsal.json", b"{}")]]:
            with self.subTest(entries=entries):
                path, digest = self.archive(entries)
                with self.assertRaises(ValueError):
                    read_members(path, digest)

    def test_duplicate_result_or_nonfinite_number_is_not_last_member_wins(self):
        for raw in [b'{"passed":false,"passed":true}', b'{"value":NaN}', b'{"value":Infinity}']:
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                decode(raw)

    def test_reported_control_pass_cannot_hide_failed_or_unexecuted_functions(self):
        case = dict(name="exact_case", status="passed", exit_code=0,
            test_results=["test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out; finished in 0.01s"],
            compiler_errors=[], failure_locations=[])
        report = dict(format="connect/transport-control-component/1", source_revision="a" * 40,
            selected_cases=["exact_case"], cases=[case], all_cases_executed=True, passed=True,
            steps=[dict(name="compile", exit_code=0, compiler_errors=[])])
        # Historical Python is parsed as data, not executed; this sentinel must
        # never run even though the literal case list is available.
        source = b'raise RuntimeError("artifact code must not execute")\nCONTROL_CASES = ("exact_case",)\n'
        with patch("lib.transport_verification.subprocess.check_output", return_value=source):
            self.assertEqual(verify_control(report)["passed_cases"], 1)
            for changed in [dict(exit_code=1), dict(status="not-run"),
                    dict(test_results=["test result: ok. 0 passed; 0 failed; 0 ignored; 4 filtered out; finished in 0.00s"])]:
                bad = copy.deepcopy(report); bad["cases"][0].update(changed)
                with self.subTest(changed=changed), self.assertRaises(ValueError):
                    verify_control(bad)

    def test_partial_sdk_inventory_and_changed_generated_bytes_refuse(self):
        first, second = self.directory / "node_sdk.js", self.directory / "browser_client.js"
        first.write_bytes(b"first"); second.write_bytes(b"second")
        inputs = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in [first, second]}
        self.assertEqual(verify_sdk(inputs, self.directory), 2)
        second.write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "sdk-runtime-bytes-or-inventory-mismatch"):
            verify_sdk(inputs, self.directory)
        second.write_bytes(b"second"); (self.directory / "omitted.js").write_bytes(b"unreported runtime")
        with self.assertRaisesRegex(ValueError, "sdk-runtime-bytes-or-inventory-mismatch"):
            verify_sdk(inputs, self.directory)

    def test_negative_conversation_is_a_verified_failure_and_never_a_payload_pass(self):
        child = dict(format="connect/transport-conversation-control/1", passed=False,
            source_revision="a" * 40, binary_sha256="b" * 64, signing_algorithm="EdDSA",
            cell_qualified=False, task_complete=False, whole_gate_qualified=False,
            observed_helpers=5, all_observed_helpers_joined=True,
            native_offers_completed=6, native_data_submissions=0, native_data_completions=0, native_data_receives=0)
        raw = json.dumps(child).encode()
        parent = dict(format="connect/transport-fixture-rehearsal/1", passed=False,
            source_revision="a" * 40, binary_sha256="b" * 64, signing_algorithm="EdDSA", servers=[], commands=[],
            conversation_report_sha256=hashlib.sha256(raw).hexdigest())
        result = verify_rehearsal(parent, "data", {"data/conversation.json": raw})
        self.assertFalse(result["conversation_passed"])
        self.assertEqual(result["native_data_receives"], 0)
        # Changing only the child's result leaves the parent's recorded digest
        # inconsistent; it cannot be laundered into a passing component.
        child["passed"] = True
        with self.assertRaisesRegex(ValueError, "conversation-report-binding-mismatch"):
            verify_rehearsal(parent, "data", {"data/conversation.json": json.dumps(child).encode()})


if __name__ == "__main__":
    unittest.main()
