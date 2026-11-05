"""Later A/V failures cannot erase earlier original native control evidence."""
import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.transport_matrix import REQUIRED_CHECKS
from lib.transport_matrix_verdict import evaluate_cell

RECOVERY = "one-service-contract-and-original-operation-recovery"
UPGRADE = "session-upgrade-and-current-capability-refusal"
AV = "independently-decoded-duplex-media-and-profile-budgets"


class NativeWitnesses(unittest.TestCase):
    def setUp(self):
        self.cell = dict(id="G6-058/macos-arm64/local/native-quic/EdDSA", server="macos-arm64",
            network="local", transport="native-quic", signing="EdDSA", required_checks=sorted(REQUIRED_CHECKS))
        self.report = dict(source_revision="a" * 40, host=dict(platform="macos-arm64"),
            network_profile="local", signing_algorithm="EdDSA", source_inputs_unchanged=True,
            executables_unchanged=True, sdk_dist_inputs_unchanged=True, passed=False, checks=[],
            native_av_decoded=dict(passed=False), conversation_control=dict(checks=[
                "independent-device-explicitly-activates-original-config-after-original-sdk-owner-joins",
                "independent-device-recovers-original-create-and-history-after-original-helper-close",
                "two-current-accepted-session-intents-coexist-with-one-main-session",
                "additive-session-capability-intent-and-unoffered-screen-refusal-without-state-change"]))

    def verdict(self, report=None):
        cell = copy.deepcopy(self.cell)
        evaluate_cell(cell, self.report if report is None else report, "b" * 64,
                      environment={}, revision="a" * 40)
        return cell["check_results"]

    def test_full_native_control_witnesses_survive_later_av_failure(self):
        outcomes = self.verdict()
        self.assertEqual(outcomes[RECOVERY]["status"], "passed")
        self.assertEqual(outcomes[RECOVERY]["evidence"]["section"], "conversation_control.checks")
        self.assertEqual(outcomes[UPGRADE]["status"], "passed")
        self.assertEqual(outcomes[AV]["status"], "failed")

    def test_each_original_control_witness_is_required(self):
        for index, name in enumerate(self.report["conversation_control"]["checks"]):
            with self.subTest(missing=name):
                report = copy.deepcopy(self.report)
                report["conversation_control"]["checks"].remove(name)
                self.assertEqual(self.verdict(report)[RECOVERY if index < 2 else UPGRADE]["status"], "failed")

    def test_old_aggregate_marker_cannot_substitute_for_current_recovery_witnesses(self):
        report = copy.deepcopy(self.report)
        report["conversation_control"]["checks"] = report["conversation_control"]["checks"][2:]
        report["checks"] = ["public-sdk-membership-audiences-two-session-intents-and-independent-device-original-recovery"]
        self.assertEqual(self.verdict(report)[RECOVERY]["status"], "failed")

    def test_wrong_candidate_never_qualifies_current_controls(self):
        report = copy.deepcopy(self.report)
        report["source_revision"] = "c" * 40
        outcomes = self.verdict(report)
        self.assertEqual(outcomes[RECOVERY]["status"], "failed")
        self.assertEqual(outcomes[UPGRADE]["status"], "failed")


if __name__ == "__main__":
    unittest.main()
