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
from lib.native_matrix_idle import NativeMatrixIdle, native_idle_verdict, restoration
from lib.native_matrix_process import NativeOwner
from lib.native_process_owner import live_workers
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


def retirement():
    return dict(passed=True, observed_helpers=8, original_helpers_joined=True, original_clients_closed=True,
        all_local_owners_elapsed_ms=250)


def idle_result():
    return dict(format="connect/native-matrix-idle-observation/1", policy=copy.deepcopy(POLICY),
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
                (path/f"resources-{phase}.json").write_text(json.dumps(dict(phase=phase,monotonic_seconds=clock[0],**extra)))
            boundary("baseline");monitor.poll();clock[0]=2;monitor.poll()
            self.assertTrue(json.loads((path/"resources-baseline-response.json").read_text())["passed"])
            for at in (3,4,5):clock[0]=at;monitor.poll()
            self.assertEqual(health_reads,[1,2], "Media sampling must not add authority reads")
            clock[0]=10;boundary("terminal");monitor.poll()
            clock[0]=10.2;boundary("retired",retirement=retirement());monitor.poll()
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
                (path/f"resources-{phase}.json").write_text(json.dumps(dict(phase=phase,monotonic_seconds=at,
                    **({"retirement":retirement()} if phase=="retired" else {}))))
                monitor.poll()
                if phase=="baseline":clock[0]=2;monitor.poll()
                self.assertFalse(json.loads((path/f"resources-{phase}-response.json").read_text())["passed"])
            self.assertFalse(monitor.finish()["passed"])

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
            conversation_control=dict(passed=False,retirement=retirement()),native_av_decoded=dict(passed=False),
            current_authority={"expiry":authority_result("expiry"),"revocation":authority_result("revocation"),"conflict":{"passed":True}},
            idle_resources=idle_result(),servers=[dict(original_joined=True)]*2)
        evaluate_cell(cell,report,"b"*64,environment=matrix["execution_environment"],revision="a"*40)
        self.assertEqual(cell["check_results"]["current-revocation-expiry-and-conflicting-operation-refusals"]["status"],"passed")
        self.assertEqual(cell["check_results"]["owned-drain-and-resource-reclamation"]["status"],"passed")
        self.assertEqual(cell["check_results"]["independently-decoded-duplex-media-and-profile-budgets"]["status"],"failed")
        report["servers"][0]["original_joined"]=False
        evaluate_cell(cell,report,"b"*64,environment=matrix["execution_environment"],revision="a"*40)
        self.assertEqual(cell["check_results"]["owned-drain-and-resource-reclamation"]["status"],"failed")


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
