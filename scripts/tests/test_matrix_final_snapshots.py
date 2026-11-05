"""A worker exiting during a read must get a final read before publication."""
import copy
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib import transport_matrix_coordinator as coordinator


class Worker:
    def __init__(self, exited=False, exit_code=1):
        self.exited = exited
        self.exit_code = exit_code

    def poll(self):
        return self.exit_code if self.exited else None


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

    def test_exited_worker_stale_snapshot_preserves_verdicts_and_blocks_missing_evidence(self):
        host = "ubuntu24-arm64"
        evidence = dict(report_sha256="b" * 64, section="original-observation")
        finished = dict(server=host, status="failed", required_checks=["control", "media"],
            execution=dict(attempted=True, component=dict(report_sha256="b" * 64, exit_code=1)),
            check_results=dict(control=dict(status="passed", evidence=evidence),
                media=dict(status="failed", evidence=evidence)))
        running = dict(server=host, status="running",
            required_checks=["control", "media", "capture", "idle", "authority", "missing"],
            execution=dict(attempted=True, component=None),
            check_results=dict(control=dict(status="passed", evidence=evidence),
                media=dict(status="failed", evidence=evidence),
                capture=dict(status="blocked", reason="original-capture-unavailable", evidence=evidence),
                idle=dict(status="running", evidence=None), authority=dict(status="not-run", evidence=None)))
        pending = dict(server=host, status="not-run", required_checks=["media"],
            execution=dict(attempted=False, component=None))
        other_host = dict(server="macos-arm64", status="running", required_checks=["media"])
        raw = dict(cells=[finished, running, pending, other_host])
        original = json.dumps(raw, sort_keys=True)
        reports = {}

        self.assertTrue(coordinator.refresh_reports(None, "a" * 40, {host: Worker(True)},
            reports, lambda _: raw))

        cells = reports[host]["cells"]
        self.assertEqual(cells[0], finished)
        self.assertEqual(cells[3], other_host)
        self.assertEqual(json.dumps(raw, sort_keys=True), original)
        for index, previous in ((1, running), (2, pending)):
            cell = cells[index]
            self.assertEqual(cell["status"], "blocked")
            self.assertEqual(cell["reason"], "host-worker-exited-without-terminal-cell-evidence")
            self.assertEqual(cell["execution"], previous["execution"])
            self.assertEqual(cell["terminal_snapshot"], dict(host_worker_exited=True,
                original_cell_status=previous["status"]))
            self.assertEqual(set(cell["check_results"]), set(cell["required_checks"]))
        for name in ("control", "media", "capture"):
            self.assertEqual(cells[1]["check_results"][name], running["check_results"][name])
        for index, names in ((1, ("idle", "authority", "missing")), (2, ("media",))):
            for name in names:
                self.assertEqual(cells[index]["check_results"][name], dict(status="blocked",
                    reason="host-worker-exited-without-terminal-cell-evidence",
                    category="environment", evidence=None))

    def test_live_worker_valid_snapshot_keeps_running_and_pending_cells(self):
        host = "ubuntu24-arm64"
        raw = dict(cells=[dict(server=host, status=status, required_checks=["media"],
            execution=dict(attempted=False), check_results=dict(media=dict(status="not-run", evidence=None)))
            for status in ("running", "not-run")])
        reports = {}
        self.assertFalse(coordinator.refresh_reports(None, "a" * 40, {host: Worker()}, reports, lambda _: raw))
        self.assertEqual(reports[host], raw)
        self.assertEqual([cell["status"] for cell in reports[host]["cells"]], ["running", "not-run"])

    def test_zero_exit_is_not_terminal_behavior_evidence(self):
        host = "ubuntu24-arm64"
        raw = dict(cells=[dict(server=host, status="not-run", required_checks=["media"],
            execution=dict(attempted=False, component=None))])
        reports = {}
        self.assertTrue(coordinator.refresh_reports(None, "a" * 40,
            {host: Worker(True, exit_code=0)}, reports, lambda _: raw))
        cell = reports[host]["cells"][0]
        self.assertEqual(cell["status"], "blocked")
        self.assertFalse(cell["execution"]["attempted"])
        self.assertIsNone(cell["check_results"]["media"]["evidence"])


if __name__ == "__main__":
    unittest.main()
