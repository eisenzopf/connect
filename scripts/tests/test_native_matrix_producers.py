"""Real owner cleanup plus independent native authority/idle verdict regressions."""
import copy
import json
import os
from pathlib import Path
import signal
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

from lib.matrix_idle import POLICY
from lib.native_matrix_authority import authority_verdict
from lib.native_matrix_idle import NativeMatrixIdle, NativeMatrixPlateau, native_cycle_retirement_verified, native_idle_verdict, restoration
from lib.native_matrix_process import NativeOwner
from lib.native_process_owner import live_workers
from lib.native_programme_profile import PROFILE
from lib.transport_matrix import inventory
from lib.transport_matrix_execution import configure_execution, full_run_preflight
from lib.transport_matrix_verdict import evaluate_cell

ROOT = next(path for path in Path(__file__).resolve().parents if (path / "contracts/gate6-transport-matrix.json").is_file())


def sample(at, stage):
    group = dict(root_state="available", unavailable_processes=0, open_resource_unavailable_processes=0,
        live_processes=1, sampled_threads=2, sampled_open_descriptors=5, sampled_rss_bytes=4096,
        observed_cpu_seconds=.1)
    return dict(monotonic_seconds=at, stage=stage, usage={"physical_owners": 0, "owned_tasks": 2, "pool:quic": 0},
        groups={"server": dict(group), "helper": dict(group)})


def programme_owner():
    return dict(profile=PROFILE, original_node_pid=42, logical_worker_thread_id=1,
        created_monotonic_seconds=.1, started_monotonic_seconds=.2, original_global_deadline_monotonic_seconds=300)


def worker_receipt(terminal=20):
    return dict(owner=programme_owner(), original_terminal_monotonic_seconds=terminal,
        original_exit_observed=True, exit_code=0, forced=False, exit_monotonic_seconds=terminal+.2,
        joined_monotonic_seconds=terminal+.25, stdio_joined=True, report_digest_verified=True,
        report_sha256="a"*64, report_bytes=1000, failure=None)


def retirement(terminal=20):
    return dict(original_terminal_monotonic_seconds=terminal, programme_worker=worker_receipt(terminal), passed=True, observed_helpers=8, original_helpers_joined=True, original_clients_closed=True,
        all_local_owners_elapsed_ms=250)


def idle_result():
    return dict(format="connect/native-matrix-idle-observation/2", policy=copy.deepcopy(POLICY),
        programme_owner=programme_owner(), original_baseline_monotonic_seconds=1, original_terminal_monotonic_seconds=20,
        terminal_monotonic_seconds=20, baseline=sample(2, "baseline"), retirement=retirement(),
        samples=[sample(1, "baseline"), sample(2, "baseline"), sample(28, "idle"), sample(29, "idle"), sample(30.01, "idle-final")])


def authority_result(kind):
    return dict(passed=True, executed=True, kind=kind, selected_transport="quic", mutation_submissions=1,
        current_original_read_before_boundary=True, original_reference_preserved=True, original_reference_sha256="c"*64,
        original_owners_joined=True, original_helpers_joined=True, opened_client_owners=2, closed_client_owners=2,
        observed_helpers=1, fresh_original_refusal=dict(kind="authentication", certainty="read-only"),
        live_original_refusal=dict(kind="cancelled", certainty="read-only"),
        **{("original_expiry_elapsed" if kind == "expiry" else "original_credential_revoked"): True},
        original_config_unchanged=True, executable_unchanged=True, original_issuance_unchanged=True,
        process_owner=dict(original_supervisor_joined=True, original_workers_joined=True, forced=False, exit_code=0))


class NativeMatrixPlateauProducer(unittest.TestCase):
    def exercise(self, leaked_cycle=None, incomplete_cycle=None):
        clock=[1.0];cycle_index=[1]
        class Sampler:
            roots={"server":(41,111),"helper":(42,222)}
            def __init__(self):self.samples=[]
            def sample(self):
                value=sample(clock[0],"process")
                for group in value["groups"].values():group["sampled_rss_bytes"]=4096 if clock[0]<10 else 8192
                self.samples.append(value);return value
            def summary(self):return dict(sample_count=len(self.samples),groups={})
        with tempfile.TemporaryDirectory() as temporary:
            path=Path(temporary);sampler=Sampler()
            def health():
                return dict(physical_owners=0,owned_tasks=int(cycle_index[0]==leaked_cycle and clock[0]%30>=10),**{"pool:quic":0})
            monitor=NativeMatrixPlateau(path,sampler,health,clock=lambda:clock[0])
            for index in (1,2,3):
                cycle_index[0]=index;start=1+(index-1)*30
                owner=programme_owner();owner["logical_worker_thread_id"]=index
                def boundary(phase,at,**extra):
                    clock[0]=at
                    (path/f"resources-cycle-{index}-{phase}.json").write_text(json.dumps(dict(phase=phase,monotonic_seconds=at,programme_owner=owner,**extra)))
                    monitor.poll()
                boundary("baseline",start);clock[0]=start+1;monitor.poll()
                clock[0]=start+2;monitor.poll()  # Existing numeric AV poll, before terminal.
                boundary("terminal",start+9)
                closed=retirement(start+9);closed["programme_worker"]["owner"]=owner
                closed["programme_worker"]["report_sha256"]=str(index)*64
                closed["workload_observation"]=dict(actual_connection_workload_observed=True,
                    programme_window_observed=index!=incomplete_cycle,seconds=62,routes=6,original_receive_tasks_joined=True)
                closed["original_session_retirement"]={"format":"connect/original-session-retirement/1","passed":True,
                    "sessions":[dict(status="ended",original_operation_sha256=str(index*2+j)*64) for j in (0,1)]}
                boundary("retired",start+9.2,retirement=closed)
                for offset in (10.3,11.4,12.5,13.6,14.7,15.8,16.9,18,19.01):clock[0]=start+offset;monitor.poll()
                self.assertTrue((path/f"resource-observation-cycle-{index}.json").exists())
            return monitor.finish()

    def test_three_native_cycles_accept_cold_warmup_only_with_original_roots_and_session_joins(self):
        observed=self.exercise()
        self.assertTrue(observed["passed"],observed["evaluation"])
        self.assertEqual(len(observed["cycles"]),3)
        from lib.matrix_plateau import runtime_capacity_configuration
        self.assertEqual(observed["server_runtime_capacity"],
            runtime_capacity_configuration(observed["persistent_owners"]["server"]))
        for cycle in observed["cycles"]:
            idle=cycle["idle_resources"]
            workload=[s for s in idle["samples"] if s["stage"]=="workload"]
            self.assertEqual(len(workload),1)
            self.assertLess(workload[0]["monotonic_seconds"],idle["terminal_monotonic_seconds"])
            self.assertNotIn("usage",workload[0],"Original media poll must not query Health")
        self.assertEqual(len({v["workload_evidence_sha256"] for v in observed["cycles"]}),3)
        self.assertTrue(all(v["joins"]["session_owners_joined"] for v in observed["cycles"]))
        self.assertGreater(observed["cycles"][1]["idle_resources"]["baseline"]["monotonic_seconds"],
                           observed["cycles"][0]["idle_resources"]["samples"][-1]["monotonic_seconds"])

    def test_native_cycle_session_leak_remains_failed_while_all_three_diagnostic_cycles_complete(self):
        observed=self.exercise(leaked_cycle=2)
        self.assertEqual(len(observed["cycles"]),3)
        self.assertFalse(observed["passed"])
        self.assertFalse(observed["cycles"][1]["idle_resources"]["passed"])
        self.assertTrue(observed["cycles"][2]["idle_resources"]["passed"])


    def test_three_cycle_verifiers_recompute_original_worker_and_cleanup_bounds(self):
        from lib.transport_matrix_verdict import native_cycle_join_verified
        original = retirement()
        resource = dict(original_programme_owner=programme_owner())
        def both(closed):
            control = dict(retirement=closed, programme_worker=closed["programme_worker"])
            return (native_cycle_retirement_verified(closed, programme_owner(), 20),
                    native_cycle_join_verified(control, resource))
        self.assertEqual(both(original), (True, True))
        for field, value in (("original_helpers_joined", False), ("original_clients_closed", False),
                             ("observed_helpers", 0), ("all_local_owners_elapsed_ms", 10001)):
            changed = copy.deepcopy(original); changed[field] = value
            self.assertEqual(both(changed), (False, False), field)
        for field, value in (("forced", True), ("original_exit_observed", False),
                             ("owner", dict(programme_owner(), logical_worker_thread_id=2))):
            changed = copy.deepcopy(original); changed["programme_worker"][field] = value
            self.assertTrue(changed["passed"])
            self.assertEqual(both(changed), (False, False), field)

    def test_short_original_native_programme_cannot_qualify_three_cycle_resources(self):
        observed=self.exercise(incomplete_cycle=1)
        self.assertEqual(len(observed["cycles"]),3)
        self.assertFalse(observed["cycles"][0]["workload_complete"])
        self.assertTrue(all(v["workload_complete"] for v in observed["cycles"][1:]))
        self.assertFalse(observed["passed"])



class NativeMatrixProducers(unittest.TestCase):
    def test_idle_rejects_retained_memory_fds_threads_owners_and_missing_actual_joins(self):
        self.assertTrue(native_idle_verdict(idle_result()))
        for role in ("server", "helper"):
            for field in ("live_processes", "sampled_threads", "sampled_open_descriptors", "sampled_rss_bytes"):
                value = idle_result();value["samples"][-1]["groups"][role][field] += 1;value["passed"] = True
                self.assertFalse(native_idle_verdict(value), (role, field))
        for field, changed in (("original_helpers_joined", False), ("original_clients_closed", False),
                               ("observed_helpers", 0), ("all_local_owners_elapsed_ms", 10001)):
            value = idle_result();value["retirement"][field] = changed
            self.assertFalse(native_idle_verdict(value), field)
        value=idle_result();value["samples"][-1]["usage"]["owned_tasks"] += 1
        self.assertFalse(native_idle_verdict(value))
        value=idle_result();value["samples"][-1]["monotonic_seconds"] = 33
        self.assertFalse(native_idle_verdict(value), "Late drain must not restart the original deadline")
        value=idle_result();value["policy"]["rss_retention_bytes"] = 4096
        self.assertFalse(native_idle_verdict(value))

    def test_idle_producer_uses_actual_boundaries_and_does_not_poll_health_during_media(self):
        clock=[1.0];health_reads=[]
        class Sampler:
            def __init__(self):self.samples=[]
            def sample(self):
                value=sample(clock[0], "process");self.samples.append(value);return value
            def summary(self):return {"sample_count":len(self.samples)}
        with tempfile.TemporaryDirectory() as temporary:
            path=Path(temporary);sampler=Sampler()
            def health():health_reads.append(clock[0]);return sample(clock[0], "health")["usage"]
            monitor=NativeMatrixIdle(path,sampler,health,clock=lambda:clock[0])
            def boundary(phase,**extra):
                (path/f"resources-{phase}.json").write_text(json.dumps(dict(phase=phase,monotonic_seconds=clock[0],programme_owner=programme_owner(),**extra)))
            boundary("baseline");monitor.poll();clock[0]=2;monitor.poll()
            self.assertTrue(json.loads((path/"resources-baseline-response.json").read_text())["passed"])
            for at in (3,4,5):clock[0]=at;monitor.poll()
            self.assertEqual(health_reads,[1,2], "Media sampling must not add authority reads")
            clock[0]=10;boundary("terminal");monitor.poll()
            clock[0]=10.2;boundary("retired",retirement=retirement(10));monitor.poll()
            for at in (11.1,12.1,13.1,14.1,15.1,16.1,17.1,18.1,19.1,20.01):clock[0]=at;monitor.poll()
            observed=monitor.finish()
            self.assertTrue(observed["passed"])
            self.assertEqual(observed["terminal_monotonic_seconds"],10)
            self.assertTrue(native_idle_verdict(observed))

    def test_unavailable_baseline_answers_false_and_still_releases_cleanup_waiters(self):
        with tempfile.TemporaryDirectory() as temporary:
            path=Path(temporary);clock=[1.0]
            class Sampler:
                def sample(self):raise PermissionError()
            monitor=NativeMatrixIdle(path,Sampler(),lambda:sample(1,"h")["usage"],clock=lambda:clock[0])
            for phase,at in (("baseline",1),("terminal",3),("retired",4)):
                clock[0]=at
                (path/f"resources-{phase}.json").write_text(json.dumps(dict(phase=phase,monotonic_seconds=at,programme_owner=programme_owner(),
                    **({"retirement":retirement()} if phase=="retired" else {}))))
                monitor.poll()
                if phase=="baseline":clock[0]=2;monitor.poll()
                self.assertFalse(json.loads((path/f"resources-{phase}-response.json").read_text())["passed"])
            self.assertFalse(monitor.finish()["passed"])

    def test_worker_join_is_required_and_original_clock_cannot_be_rebased(self):
        for field, changed in (("original_exit_observed", False), ("forced", True), ("exit_code", 1),
                               ("stdio_joined", False), ("report_digest_verified", False),
                               ("report_sha256", "not-a-digest"), ("joined_monotonic_seconds", 30.001),
                               ("original_terminal_monotonic_seconds", 21), ("failure", "worker-error")):
            value = idle_result(); value["retirement"]["programme_worker"][field] = changed
            self.assertFalse(native_idle_verdict(value), field)
        for field in ("original_helpers_joined", "original_clients_closed"):
            value = idle_result(); value["retirement"][field] = False
            self.assertFalse(native_idle_verdict(value), field)
        value = idle_result(); value["format"] = "connect/native-matrix-idle-observation/1"
        self.assertFalse(native_idle_verdict(value), "Old profile cannot qualify")
        value = idle_result(); value["programme_owner"]["started_monotonic_seconds"] = 3
        self.assertFalse(native_idle_verdict(value), "Worker must exist before baseline")
        value = idle_result(); value["retirement"]["programme_worker"]["owner"]["logical_worker_thread_id"] = 2
        self.assertFalse(native_idle_verdict(value), "Another worker is not the original")

    def test_authority_requires_current_original_native_refusal_not_an_aggregate_green_flag(self):
        for kind in ("expiry","revocation"):
            self.assertTrue(authority_verdict(authority_result(kind),kind))
            for change in ("fresh-timeout","second-mutation","changed-issuance","forced-owner","unjoined-helper","wrong-original"):
                value=authority_result(kind)
                if change=="fresh-timeout":value["fresh_original_refusal"]["kind"]="deadline"
                if change=="second-mutation":value["mutation_submissions"]=2
                if change=="changed-issuance":value["original_issuance_unchanged"]=False
                if change=="forced-owner":value["process_owner"]["forced"]=True
                if change=="unjoined-helper":value["original_helpers_joined"]=False
                if change=="wrong-original":value["original_reference_preserved"]=False
                self.assertFalse(authority_verdict(value,kind),change)

    def test_all_32_are_attemptable_while_mac_remote_producer_gaps_remain_honest(self):
        all_ids=set()
        for host in ("macos-arm64","ubuntu24-arm64"):
            # Exercise the explicit incomplete-producer state even after a
            # later cross-host implementation fills those real code paths.
            with patch("lib.transport_matrix_execution.full_producer_gaps", side_effect=lambda cell:
                    ["macos-server-linux-client-remote-producer-not-connected"]
                    if cell["server"]=="macos-arm64" and cell["network"]!="local" else []):
                matrix=configure_execution(inventory(ROOT,"a"*40),dict(server=host,execution_blockers=[],browsers={}),"full")
            all_ids.update(c["id"] for c in matrix["cells"] if c["execution"]["runnable"])
            preflight=full_run_preflight(matrix)
            self.assertFalse(preflight["all_workloads_connected"])
            self.assertEqual(len(preflight["missing"]),8)
            self.assertTrue(all("macos-arm64/remote" in value["cell_id"] for value in preflight["missing"]))
            self.assertFalse(preflight["tests_executed"])
        self.assertEqual(len(all_ids),32)

    def test_native_independent_checks_survive_av_failure_but_missing_original_join_fails_idle(self):
        matrix=configure_execution(inventory(ROOT,"a"*40),dict(server="ubuntu24-arm64",execution_blockers=[],browsers={}),"full")
        cell=next(c for c in matrix["cells"] if c["execution"]["runnable"] and c["transport"]=="native-quic" and c["network"]=="local")
        report=dict(passed=False,source_revision="a"*40,host=dict(platform=cell["server"]),network_profile="local",
            signing_algorithm=cell["signing"],source_inputs_unchanged=True,sdk_dist_inputs_unchanged=True,executables_unchanged=True,
            conversation_control=dict(passed=False,retirement=retirement(),programme_profile=PROFILE,programme_worker=worker_receipt()),native_av_decoded=dict(passed=False),
            current_authority={"expiry":authority_result("expiry"),"revocation":authority_result("revocation"),"conflict":{"passed":True}},
            idle_resources=idle_result(),servers=[dict(original_joined=True)]*2)
        owners=sorted(("controller-1","controller-2","participant-2","participant-3","participant-4","replacement-1"))
        phases=([owner+"-before-revocation" for owner in owners]+["revoke-primary-device","revoked-primary-native-refusal"]
            +[owner+"-after-revocation" for owner in owners]+["original-actor-configs-unchanged"])
        report["late_native_controls"]=dict(attempted=True,status="passed",all_required_calls_attempted=True,
            all_before_exact=True,primary_revoked=True,revoked_primary_refused=True,all_after_same=True,
            configs_unchanged=True,steps=[dict(phase=phase,status="passed") for phase in phases])
        report["checks"]=["revoked-primary-device-refuses-while-independent-replacement-and-other-devices-remain-usable"]
        evaluate_cell(cell,report,"b"*64,environment=matrix["execution_environment"],revision="a"*40)
        self.assertEqual(cell["check_results"]["current-revocation-expiry-and-conflicting-operation-refusals"]["status"],"passed")
        self.assertEqual(cell["check_results"]["owned-drain-and-resource-reclamation"]["status"],"passed")
        self.assertEqual(cell["check_results"]["independently-decoded-duplex-media-and-profile-budgets"]["status"],"failed")
        report["servers"][0]["original_joined"]=False
        evaluate_cell(cell,report,"b"*64,environment=matrix["execution_environment"],revision="a"*40)
        self.assertEqual(cell["check_results"]["owned-drain-and-resource-reclamation"]["status"],"failed")
        report["current_authority"]["expiry"].update(passed=False,executed=False,launcher_attempted=True)
        evaluate_cell(cell,report,"b"*64,environment=matrix["execution_environment"],revision="a"*40)
        self.assertEqual(cell["check_results"]["current-revocation-expiry-and-conflicting-operation-refusals"]["status"],"blocked",
            "Provisioning/import failure is not an executed current-authority case")


class NativeOriginalOwner(unittest.TestCase):
    def wait(self, work, seconds=4):
        until=time.monotonic()+seconds
        while time.monotonic()<until:
            if work():return
            time.sleep(.02)
        self.fail("Original owner deadline")

    def test_original_supervisor_stays_anchored_after_child_exit_until_explicit_join(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory=Path(temporary)
            owner=NativeOwner([sys.executable,"-c","pass"],directory/"owner",directory/"log",cwd=ROOT,env=os.environ.copy())
            try:
                self.wait(lambda:owner.poll() is not None)
                self.assertEqual(owner.code,0);self.assertIsNone(owner.launch.poll())
                self.assertFalse(live_workers(owner.pid))
            finally:result=owner.close()
            self.assertTrue(result["original_supervisor_joined"])
            self.assertTrue(result["original_workers_joined"])
            self.assertFalse(result["forced"])

    def test_abandoned_descendant_is_retired_by_original_supervisor_and_never_counted_clean(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory=Path(temporary)
            script="import subprocess,sys;subprocess.Popen([sys.executable,'-c','import time;time.sleep(30)'])"
            owner=NativeOwner([sys.executable,"-c",script],directory/"owner",directory/"log",cwd=ROOT,env=os.environ.copy())
            try:
                self.wait(lambda:owner.poll() is not None)
                self.assertTrue(live_workers(owner.pid))
            finally:result=owner.close()
            self.assertTrue(result["forced"])
            self.assertFalse(result["original_workers_joined"])
            self.wait(lambda:not live_workers(owner.pid))


if __name__=="__main__":unittest.main()
