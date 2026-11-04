"""Completed browser controls never qualify missing AV, expiry or idle work."""
import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.transport_matrix import REQUIRED_CHECKS
from lib.transport_matrix_execution import record_browser_control_checks


class BrowserControls(unittest.TestCase):
    def fixture(self):
        cell = dict(check_results={name: dict(status="not-run") for name in REQUIRED_CHECKS},
                    non_av_scope=dict(qualified=False))
        observed = dict(passed=True, authenticated_owners=7, actual_connections=6,
            retirement=dict(passed=True, originalEndsCompleted=6, originalSessionsEnded=2, originalClientsClosed=7),
            admission=dict(checks=["participant-add-update-remove-rejoin-and-current-addressed-history",
                "two-concurrent-accepted-sessions-upgraded-to-audio-video-and-unoffered-screen-refused"]))
        return cell, observed

    def test_completed_admission_promotes_only_two_complete_control_clauses(self):
        cell, observed = self.fixture()
        record_browser_control_checks(cell, observed, "a" * 64, True)
        self.assertEqual({k for k, v in cell["check_results"].items() if v["status"] == "passed"},
            {"participant-add-update-remove-and-current-audiences", "session-upgrade-and-current-capability-refusal"})
        self.assertFalse(cell["non_av_scope"]["qualified"])
        self.assertIn("current-revocation-and-expiry-refusal", cell["non_av_observations"]["missing"])

    def test_failed_integrity_missing_control_or_unjoined_owner_cannot_promote(self):
        for change, passed in [(lambda r: None, False),
            (lambda r: r.update(passed=False), True),
            (lambda r: r.update(actual_connections=5), True),
            (lambda r: r["retirement"].update(originalSessionsEnded=1), True),
            (lambda r: r["retirement"].update(originalClientsClosed=6), True),
            (lambda r: r["admission"].update(checks=[]), True)]:
            cell, observed = self.fixture(); change(observed)
            record_browser_control_checks(cell, observed, "a" * 64, passed)
            self.assertFalse(any(v["status"] == "passed" for v in cell["check_results"].values()))


if __name__ == "__main__":
    unittest.main()
