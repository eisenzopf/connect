"""A worker exiting during a read must get a final read before publication."""
import copy
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib import transport_matrix_coordinator as coordinator


class Worker:
    def __init__(self, exited=False):
        self.exited = exited

    def poll(self):
        return 1 if self.exited else None


class FinalSnapshots(unittest.TestCase):
    def test_worker_exiting_after_snapshot_is_read_gets_a_new_final_read(self):
        workers = {"macos-arm64": Worker(True), "ubuntu24-arm64": Worker()}
        reports = {}
        reads = []

        def read(host):
            reads.append(host)
            status = "failed" if workers[host].exited else "running"
            # Original worker publishes its final result and exits just after
            # the coordinator captured its previous running snapshot.
            workers[host].exited = True
            return {"status": status}

        self.assertFalse(coordinator.refresh_reports(None, "a" * 40, workers, reports, read))
        self.assertEqual(reports["ubuntu24-arm64"]["status"], "running")
        self.assertTrue(coordinator.refresh_reports(None, "a" * 40, workers, reports, read))
        self.assertEqual(reports["ubuntu24-arm64"]["status"], "failed")
        self.assertEqual(reads.count("ubuntu24-arm64"), 2)

    def test_unreadable_final_snapshot_never_promotes_cached_results(self):
        host = "ubuntu24-arm64"
        workers = {host: Worker(True)}
        reports = {host: {"status": "passed"}}
        base = {"cells": [{"server": host, "required_checks": ["media"], "status": "not-run"}]}
        def read(_):
            raise OSError("final worker artifact is unavailable")
        with patch.object(coordinator, "inventory", return_value=base), \
                patch.object(coordinator, "configure_execution", side_effect=lambda report, *_: copy.deepcopy(report)):
            self.assertTrue(coordinator.refresh_reports(None, "a" * 40, workers, reports, read))
        cell = reports[host]["cells"][0]
        self.assertEqual(cell["status"], "blocked")
        self.assertEqual(cell["reason"], "host-final-snapshot-unavailable")
        self.assertEqual(cell["check_results"]["media"]["status"], "blocked")

    def test_temporary_read_failure_keeps_live_worker_pending(self):
        host = "ubuntu24-arm64"
        workers = {host: Worker()}
        reports = {host: {"status": "running"}}
        def read(_):
            raise OSError("temporary read failure")
        self.assertFalse(coordinator.refresh_reports(None, "a" * 40, workers, reports, read))
        self.assertEqual(reports[host], {"status": "running"})


if __name__ == "__main__":
    unittest.main()
