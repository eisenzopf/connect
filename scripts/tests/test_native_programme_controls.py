"""Ordinary failures retain independent late controls; unsafe cleanup never enters."""
import copy
import subprocess
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from lib.native_programme_controls import ordinary_native_check, run_late_native_controls, original_native_owners_joined, wait_original_control_child, OriginalControlOwnerUnjoined, terminate_original_control_child


class LateNativeControls(unittest.TestCase):
    def fixture(self, *, fail_before=False, fail_revoke=False):
        owners = {"controller-1", "controller-2", "participant-2", "participant-3", "participant-4", "replacement-1"}
        expected = {"cid_0", "cid_1", "cid_2"}; calls = []
        rows = [dict(id=cid, policy="persistent") for cid in sorted(expected)]
        def before(owner):
            calls.append(("before", owner))
            if fail_before and owner == "controller-1": raise ValueError("private response body")
            return copy.deepcopy(rows)
        def revoke():
            calls.append(("revoke", None))
            if fail_revoke: raise OSError("private credential/path")
            return True
        def refused(): calls.append(("primary-refusal", None)); return True
        def after(owner): calls.append(("after", owner)); return copy.deepcopy(rows)
        def configs(): calls.append(("configs", None)); return True
        return dict(owners=owners, expected=expected, before_read=before, revoke_primary=revoke,
            primary_refused=refused, after_read=after, configs_unchanged=configs), calls

    def test_quality_failure_still_runs_all_original_independent_late_controls(self):
        args, calls = self.fixture(); report = dict(passed=True)
        self.assertFalse(ordinary_native_check(report, "original-quality-failure", False))
        original = copy.deepcopy(report["failure"])
        late = run_late_native_controls(report, owners_joined=True, **args)
        self.assertEqual(late["status"], "passed")
        self.assertTrue(late["all_required_calls_attempted"])
        self.assertEqual(len(calls), 15)
        self.assertEqual([phase for phase, _ in calls], ["before"] * 6 + ["revoke", "primary-refusal"] + ["after"] * 6 + ["configs"])
        self.assertEqual(report["failure"], original)
        self.assertFalse(report["passed"], "Independent control success cannot repair original quality failure")

    def test_unsafe_original_cleanup_leaves_late_controls_explicitly_not_run(self):
        args, calls = self.fixture(); report = dict(passed=False)
        for unknown in (False, None, 1):
            with self.subTest(owners_joined=unknown):
                late = run_late_native_controls(report, owners_joined=unknown, **args)
                self.assertEqual(late["status"], "not-run")
                self.assertFalse(late["attempted"])
                self.assertEqual(late["reason"], "original-programme-owners-not-joined")
                self.assertEqual(calls, [])

    def test_quality_success_cannot_stand_in_for_original_worker_retirement(self):
        report = dict(passed=True, conversation_control=dict(all_observed_helpers_joined=True))
        self.assertFalse(original_native_owners_joined(report, True, 3))
        self.assertFalse(original_native_owners_joined(report, True, 1))
        report["conversation_control"]["all_observed_helpers_joined"] = False
        self.assertFalse(original_native_owners_joined(report, False, 1))

        # Genuine receipt shape consumed by the actual Worker/retirement
        # validators, matching the original three-cycle native producer.
        from lib.native_programme_profile import PROFILE
        controls, resources, receipts = [], [], []
        for index in (1, 2, 3):
            owner = dict(profile=PROFILE, original_node_pid=42, logical_worker_thread_id=index,
                created_monotonic_seconds=index * 40., started_monotonic_seconds=index * 40. + .1,
                original_global_deadline_monotonic_seconds=index * 40. + 300)
            terminal = index * 40. + 20
            worker = dict(owner=owner, original_terminal_monotonic_seconds=terminal,
                original_exit_observed=True, exit_code=0, forced=False,
                exit_monotonic_seconds=terminal + .2, joined_monotonic_seconds=terminal + .25,
                stdio_joined=True, report_digest_verified=True, report_sha256=str(index) * 64,
                report_bytes=1000, failure=None)
            retirement = dict(original_terminal_monotonic_seconds=terminal,
                programme_worker=copy.deepcopy(worker), passed=True, observed_helpers=8,
                original_helpers_joined=True, original_clients_closed=True, all_local_owners_elapsed_ms=250)
            controls.append(dict(index=index, programme_worker=copy.deepcopy(worker), retirement=retirement))
            resources.append(dict(index=index, original_programme_owner=copy.deepcopy(owner),
                original_worker_receipt=copy.deepcopy(worker)))
            receipts.append(dict(index=index, programme_worker=copy.deepcopy(worker)))
        valid = dict(conversation_control=dict(all_observed_helpers_joined=True, resource_cycles=receipts),
            native_cycle_controls=controls, idle_resources=dict(cycles=resources))
        self.assertTrue(original_native_owners_joined(valid, True, 3))
        for mismatch in ("index", "receipt", "owner", "unjoined", "missing-cycle"):
            with self.subTest(mismatch=mismatch):
                bad = copy.deepcopy(valid)
                if mismatch == "index": bad["idle_resources"]["cycles"][1]["index"] = 99
                elif mismatch == "receipt": bad["conversation_control"]["resource_cycles"][1]["programme_worker"]["report_sha256"] = "f" * 64
                elif mismatch == "owner": bad["idle_resources"]["cycles"][1]["original_programme_owner"]["logical_worker_thread_id"] = 99
                elif mismatch == "unjoined":
                    bad["native_cycle_controls"][1]["programme_worker"]["forced"] = True
                    bad["native_cycle_controls"][1]["retirement"]["programme_worker"]["forced"] = True
                    bad["conversation_control"]["resource_cycles"][1]["programme_worker"]["forced"] = True
                    bad["idle_resources"]["cycles"][1]["original_worker_receipt"]["forced"] = True
                else: bad["idle_resources"]["cycles"].pop()
                self.assertFalse(original_native_owners_joined(bad, True, 3))
        single = dict(conversation_control=dict(all_observed_helpers_joined=True,
            programme_worker=copy.deepcopy(controls[0]["programme_worker"]),
            retirement=copy.deepcopy(controls[0]["retirement"])),
            idle_resources=dict(programme_owner=copy.deepcopy(resources[0]["original_programme_owner"]),
                retirement=copy.deepcopy(controls[0]["retirement"])))
        self.assertTrue(original_native_owners_joined(single, True, 1))
        single["idle_resources"]["programme_owner"]["logical_worker_thread_id"] = 99
        self.assertFalse(original_native_owners_joined(single, True, 1))

    def test_late_failures_preserve_first_failure_and_join_every_independent_call(self):
        args, calls = self.fixture(fail_before=True, fail_revoke=True); report = dict(passed=True)
        ordinary_native_check(report, "original-quality-failure", False)
        original = copy.deepcopy(report["failure"])
        late = run_late_native_controls(report, owners_joined=True, **args)
        self.assertEqual(late["status"], "failed")
        self.assertTrue(late["all_required_calls_attempted"])
        self.assertEqual(len(calls), 15)
        self.assertEqual(sum(phase == "revoke" for phase, _ in calls), 1, "No mutation retry after uncertainty")
        self.assertEqual(sum(phase == "after" for phase, _ in calls), 6)
        self.assertEqual(report["failure"], original)
        self.assertFalse(report["passed"])
        self.assertFalse(late["all_before_exact"])
        self.assertFalse(late["primary_revoked"])
        self.assertFalse(late["all_after_same"], "Missing before receipt cannot be invented")
        self.assertEqual([x["phase"] for x in report["additional_failures"]][:3],
            ["original-quality-failure", "controller-1-before-revocation", "revoke-primary-device"])
        import json
        self.assertNotIn("private response", json.dumps(late))
        self.assertNotIn("private credential", json.dumps(late))

    def test_joined_command_timeout_continues_but_unknown_cleanup_stops_and_blocks_custody(self):
        from lib.vm_workload_lease import cleanup_evidence
        class Child:
            pid = 4242
            def __init__(self, outcomes): self.outcomes = list(outcomes); self.waits = []
            def wait(self, timeout):
                self.waits.append(timeout)
                value = self.outcomes.pop(0)
                if isinstance(value, BaseException): raise value
                return value
        for cleanup_error in (None, subprocess.TimeoutExpired("owned", 5), PermissionError("private")):
            with self.subTest(cleanup_error=type(cleanup_error).__name__):
                args, calls = self.fixture(); report = dict(passed=False)
                ordinary_native_check(report, "original-quality-failure", False)
                original = copy.deepcopy(report["failure"])
                child = Child([subprocess.TimeoutExpired("owned", 30), 0 if cleanup_error is None else cleanup_error])
                killed = []
                def before(owner):
                    calls.append(("before", owner))
                    return wait_original_control_child(child, lambda: killed.append(True), report, "owned-before")
                args["before_read"] = before
                # Only the first command is delayed; remaining original reads are normal.
                original_before = self.fixture()[0]["before_read"]
                def first_then_normal(owner):
                    if not killed: return before(owner)
                    return original_before(owner)
                args["before_read"] = first_then_normal
                if cleanup_error is None:
                    late = run_late_native_controls(report, owners_joined=True, **args)
                    self.assertTrue(late["all_required_calls_attempted"])
                    self.assertTrue(report["public_control_owner_cleanup"]["confirmed"])
                    self.assertTrue(cleanup_evidence(report)["public_commands_joined"])
                    self.assertIn(("revoke", None), calls)
                else:
                    with self.assertRaises(OriginalControlOwnerUnjoined):
                        run_late_native_controls(report, owners_joined=True, **args)
                    self.assertEqual(len(calls), 1)
                    self.assertEqual(report["late_native_controls"]["status"], "incomplete")
                    self.assertFalse(report["late_native_controls"]["all_required_calls_attempted"])
                    self.assertEqual(report["late_native_controls"]["steps"][0]["status"], "owner-unconfirmed")
                    self.assertFalse(report["public_control_owner_cleanup"]["confirmed"])
                    self.assertFalse(cleanup_evidence(report)["confirmed"])
                    self.assertFalse(cleanup_evidence(report)["public_commands_joined"])
                self.assertEqual(child.waits, [30, 5])
                self.assertEqual(killed, [True])
                self.assertEqual(report["failure"], original)
                self.assertFalse(report["passed"])
        # Failed kill still attempts the original wait, and remains fatal even if
        # the leader subsequently exits; whole-group termination was unconfirmed.
        child = Child([subprocess.TimeoutExpired("owned", 30), 0]); report = {}
        def failed_kill(): raise PermissionError("private")
        with self.assertRaises(OriginalControlOwnerUnjoined):
            wait_original_control_child(child, failed_kill, report, "owned-before")
        self.assertEqual(child.waits, [30, 5])
        self.assertFalse(cleanup_evidence(report)["public_commands_joined"])

        # Exercise the actual termination helper's permission fallback; a failed
        # sudo kill cannot be hidden by a subsequently exited leader.
        from unittest.mock import patch
        from types import SimpleNamespace
        child = Child([subprocess.TimeoutExpired("owned", 30), 0]); report = {}
        namespace = SimpleNamespace(client_forced=False)
        with patch("lib.native_programme_controls.os.killpg", side_effect=PermissionError), \
             patch("lib.native_programme_controls.subprocess.run", side_effect=subprocess.CalledProcessError(1, "owned-kill")) as sudo:
            with self.assertRaises(OriginalControlOwnerUnjoined):
                wait_original_control_child(child,
                    lambda: terminate_original_control_child(child.pid, namespace=namespace, env={}), report, "owned-before")
        self.assertTrue(sudo.call_args.kwargs["check"])
        self.assertEqual(sudo.call_args.kwargs["timeout"], 5)
        self.assertTrue(namespace.client_forced)
        self.assertEqual(child.waits, [30, 5])
        self.assertFalse(cleanup_evidence(report)["public_commands_joined"])

        self.assertTrue(cleanup_evidence({})["public_commands_joined"])
        good = dict(confirmed=True, owners_started=2, owners_joined=2)
        self.assertTrue(cleanup_evidence(dict(public_control_owner_cleanup=good))["public_commands_joined"])
        for bad in (None, False, {}, dict(confirmed=True, owners_started=2, owners_joined=1),
                dict(confirmed=1, owners_started=2, owners_joined=2),
                dict(confirmed=True, owners_started=True, owners_joined=1),
                dict(confirmed=True, owners_started=2, owners_joined=2, unconfirmed_owner={})):
            self.assertFalse(cleanup_evidence(dict(public_control_owner_cleanup=bad))["public_commands_joined"])


if __name__ == "__main__": unittest.main()
