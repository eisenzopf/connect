"""A failed original producer cannot become a qualified cell from partial witnesses."""
import copy
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from lib.transport_matrix import inventory
from lib.transport_matrix_execution import configure_execution, merge_hosts
from lib.transport_matrix_verdict import evaluate_cell, summarize
from lib.native_programme_profile import PROFILE

REVISION = "a" * 40
DIGEST = "b" * 64


class OriginalProducer(unittest.TestCase):
    def case(self):
        matrix = configure_execution(inventory(ROOT, REVISION),
            dict(server="ubuntu24-arm64", execution_blockers=[], browsers={}), "full")
        cell = next(c for c in matrix["cells"] if c["server"] == "ubuntu24-arm64"
            and c["transport"] == "native-quic" and c["network"] == "remote-profile-fixture")
        cell["execution"].update(attempted=True, component=dict(report_sha256=DIGEST,
            exit_code=0, failure_phase=None, failure_kind=None))
        controls = ["independent-device-explicitly-activates-original-config-after-original-sdk-owner-joins",
            "independent-device-recovers-original-create-and-history-after-original-helper-close",
            "scoped-native-conversation-and-nonmember-current-access-refusal",
            "four-active-participants-and-addition-does-not-expand-older-message-audience",
            "membership-update-original-replay-changed-intent-and-stale-revision-guards",
            "remove-and-readmit-preserve-participant-id-and-fence-old-epoch-history-and-replay",
            "two-current-accepted-session-intents-coexist-with-one-main-session",
            "additive-session-capability-intent-and-unoffered-screen-refusal-without-state-change"]
        worker = dict(unit_fixture_only=True)
        retirement = dict(passed=True, programme_worker=worker)
        quality = dict(quality_profile="connect/decoded-audio-quality/3",
            metric_scope="measurable-decoded-quality-source-quanta-diagnostic",
            original_source_sample_coverage_proved=False)
        audio = dict(quality, source_quantum_scope="diagnostic-only-source-clock-attribution")
        report = dict(passed=True, source_revision=REVISION, host=dict(platform="ubuntu24-arm64"),
            network_profile="remote", signing_algorithm=cell["signing"],
            source_inputs_unchanged=True, sdk_dist_inputs_unchanged=True, executables_unchanged=True,
            binary_sha256="c" * 64, runtime_versions=dict(passed=True), actor_enrollment=dict(unit_fixture_only=True),
            current_trust_verified=True, selected_transport_verified=True,
            conversation_control=dict(checks=controls, actual_connection_workload_observed=True,
                maximum_active_members_observed=4, concurrent_session_intents_observed=2,
                programme_profile=PROFILE, programme_worker=worker, retirement=retirement),
            movement=dict(original_moves_completed=4, moves=[dict(original_reference_preserved=True,
                original_retired=True, camera_transfer_admitted=True) for _ in range(4)]),
            packet_capture=dict(observed=True, original_joined=True, kernel_dropped_packets=0),
            native_av_decoded=dict(quality, passed=True,
                routes=[dict(passed=True, audio=dict(audio)) for _ in range(6)]),
            current_authority={name:dict(passed=True, executed=True) for name in ("revocation", "expiry", "conflict")},
            idle_resources=dict(retirement=retirement, programme_worker=worker),
            servers=[dict(original_joined=True), dict(original_joined=True)])
        owners = sorted(("controller-1", "controller-2", "participant-2", "participant-3", "participant-4", "replacement-1"))
        phases = ([owner + "-before-revocation" for owner in owners]
            + ["revoke-primary-device", "revoked-primary-native-refusal"]
            + [owner + "-after-revocation" for owner in owners] + ["original-actor-configs-unchanged"])
        report["late_native_controls"] = dict(attempted=True, status="passed", all_required_calls_attempted=True,
            all_before_exact=True, primary_revoked=True, revoked_primary_refused=True, all_after_same=True,
            configs_unchanged=True, steps=[dict(phase=phase, status="passed") for phase in phases])
        report["checks"] = ["revoked-primary-device-refuses-while-independent-replacement-and-other-devices-remain-usable"]
        return matrix, cell, report

    def evaluate(self, cell, report):
        # This suite isolates aggregate producer admission. Existing independent
        # authority/idle suites own the underlying physical witness validation.
        with patch("lib.native_matrix_idle.native_idle_verdict", return_value=True), \
                patch("lib.native_matrix_authority.authority_verdict", return_value=True):
            return evaluate_cell(cell, report, DIGEST, environment={}, revision=REVISION)

    def assert_partial_witnesses(self, cell):
        self.assertEqual(len(cell["check_results"]), 12)
        self.assertTrue(all(row["status"] == "passed" for row in cell["check_results"].values()))
        self.assertTrue(all(row["evidence"]["report_sha256"] == DIGEST
            for row in cell["check_results"].values()))

    def test_all_twelve_witnesses_require_original_successful_producer(self):
        for mode in ("exit", "report", "component", "contradictory-failure", "digest"):
            with self.subTest(mode=mode):
                _, cell, report = self.case()
                if mode == "exit": cell["execution"]["component"]["exit_code"] = 1
                elif mode == "report": report["passed"] = False
                elif mode == "component": cell["execution"]["component"]["passed"] = False
                elif mode == "contradictory-failure":
                    report["failure"] = dict(phase="replacement-1-before-revocation", name="AssertionError", line=810)
                else: cell["execution"]["component"]["report_sha256"] = "d" * 64
                self.evaluate(cell, report)
                self.assert_partial_witnesses(cell)
                self.assertEqual(cell["status"], "failed")
                self.assertEqual(cell["verifier"]["producer_outcome"]["status"], "failed")
        _, cell, report = self.case()
        self.evaluate(cell, report)
        self.assert_partial_witnesses(cell)
        self.assertEqual(cell["status"], "passed")
        self.assertEqual(cell["verifier"]["producer_outcome"]["status"], "passed")

    def test_unknown_original_outcome_is_blocked_without_erasing_twelve_checks(self):
        for mode in ("exit", "boolean-exit", "attempt", "report", "digest"):
            with self.subTest(mode=mode):
                _, cell, report = self.case()
                if mode == "exit": del cell["execution"]["component"]["exit_code"]
                elif mode == "boolean-exit": cell["execution"]["component"]["exit_code"] = False
                elif mode == "attempt": cell["execution"]["attempted"] = False
                elif mode == "report": del report["passed"]
                else: del cell["execution"]["component"]["report_sha256"]
                self.evaluate(cell, report)
                self.assert_partial_witnesses(cell)
                self.assertEqual(cell["status"], "blocked")
                self.assertEqual(cell["verifier"]["producer_outcome"]["status"], "blocked")

    def test_host_merge_rejects_failed_or_unknown_producer_behind_claimed_pass(self):
        for mode in ("exit", "outcome", "report", "digest"):
            with self.subTest(mode=mode):
                matrix, cell, report = self.case()
                self.evaluate(cell, report)
                if mode == "exit": cell["execution"]["component"]["exit_code"] = 1
                elif mode == "outcome": del cell["verifier"]["producer_outcome"]
                elif mode == "report": cell["verifier"]["producer_outcome"]["report_passed"] = False
                else: cell["execution"]["component"]["report_sha256"] = "d" * 64
                self.assertEqual(cell["status"], "passed")
                with self.assertRaisesRegex(ValueError, "Complete cell lacks"):
                    merge_hosts(inventory(ROOT, REVISION), [matrix])

    def test_earlier_authority_cases_cannot_substitute_unexecuted_or_failed_late_controls(self):
        authority = "current-revocation-expiry-and-conflicting-operation-refusals"
        for mode in ("missing", "not-run", "failed", "missing-owner-step", "wrong-order", "missing-marker"):
            with self.subTest(mode=mode):
                _, cell, report = self.case()
                if mode == "missing": del report["late_native_controls"]
                elif mode == "not-run": report["late_native_controls"] = dict(attempted=False, status="not-run")
                elif mode == "failed": report["late_native_controls"]["status"] = "failed"
                elif mode == "missing-owner-step": report["late_native_controls"]["steps"].pop()
                elif mode == "wrong-order": report["late_native_controls"]["steps"].reverse()
                else: report["checks"] = []
                self.evaluate(cell, report)
                self.assertEqual(cell["check_results"][authority]["status"],
                    "blocked" if mode in ("missing", "not-run") else "failed")
                self.assertEqual(sum(r["status"] == "passed" for r in cell["check_results"].values()), 11)
                self.assertNotEqual(cell["status"], "passed")

    def test_whole_matrix_requires_original_success_without_adding_requirement_rows(self):
        matrix, cell, report = self.case()
        self.evaluate(cell, report)
        witness = copy.deepcopy(cell["verifier"])
        for current in matrix["cells"]:
            current["status"] = "passed"
            current["execution"].update(attempted=True, component=dict(report_sha256=DIGEST,
                exit_code=0, failure_phase=None, failure_kind=None))
            current["verifier"] = copy.deepcopy(witness)
            current["verifier"]["required"] = current["verifier"]["evaluated"] = len(current["required_checks"])
            current["check_results"] = {name:dict(status="passed", evidence=dict(report_sha256=DIGEST))
                for name in current["required_checks"]}
        self.assertEqual(summarize(matrix)["required_checks"], 432)
        self.assertTrue(matrix["matrix_qualified"])
        matrix["cells"][0]["execution"]["component"]["exit_code"] = 1
        self.assertEqual(summarize(matrix)["checks"]["passed"], 432)
        self.assertFalse(matrix["matrix_qualified"])
        self.assertFalse(matrix["task_complete"])


if __name__ == "__main__":
    unittest.main()
