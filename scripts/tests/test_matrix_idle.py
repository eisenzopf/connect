"""Missing observations and retained owners cannot become an idle pass."""
import copy
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.matrix_idle import POLICY, idle_verdict, owned_usage, restoration


def sample(at, stage):
    group = dict(root_state="available", unavailable_processes=0,
        open_resource_unavailable_processes=0, live_processes=1, sampled_threads=2,
        sampled_open_descriptors=5, sampled_rss_bytes=4096, observed_cpu_seconds=.1)
    return dict(monotonic_seconds=at, stage=stage, usage={"physical_owners": 0,
        "owned_tasks": 2, "pool:webtransport": 0}, groups={"server": dict(group), "browser": dict(group)})


def result():
    return dict(format="connect/matrix-idle-observation/1", policy=POLICY,
        terminal_monotonic_seconds=20, baseline=sample(1, "baseline"),
        samples=[sample(1, "baseline"), sample(2, "baseline"), sample(28, "idle"),
                 sample(29, "idle"), sample(30.01, "idle-final")])


class MatrixIdleTests(unittest.TestCase):
    def test_retained_memory_descriptors_threads_or_runtime_owners_fail_without_allowance(self):
        original = sample(1, "baseline")
        self.assertEqual(restoration(original, sample(10, "idle")), [])
        for role in ("server", "browser"):
            for field in ("live_processes", "sampled_threads", "sampled_open_descriptors", "sampled_rss_bytes"):
                current = sample(10, "idle");current["groups"][role][field] += 1
                self.assertIn(role + ":" + field, restoration(original, current))
        current = sample(10, "idle");current["usage"]["physical_owners"] = 1
        self.assertIn("physical_owners", restoration(original, current))

    def test_missing_reused_or_draining_observation_cannot_prove_idle(self):
        original = sample(1, "baseline")
        for state in ("retired", "unavailable"):
            current = sample(10, "idle");current["groups"]["server"]["root_state"] = state
            self.assertTrue(restoration(original, current))
        self.assertTrue(restoration(original, dict(error="PermissionError")))
        health = dict(operational=dict(runtime=dict(physical_owners=0, owned_tasks=2),
            resource_pools=[dict(kind="webtransport", state="available", usage=dict(used=0, draining=False))]))
        self.assertEqual(owned_usage(health)["pool:webtransport"], 0)
        health["operational"]["resource_pools"][0]["usage"]["draining"] = True
        with self.assertRaises(ValueError): owned_usage(health)

    def test_verifier_requires_original_ten_second_window_and_stable_restoration(self):
        self.assertTrue(idle_verdict(result()))
        for index, change in ((-1, 29.9), (-1, 33), (2, 31), (3, 28)):
            value=result();value["samples"][index]["monotonic_seconds"]=change
            self.assertFalse(idle_verdict(value))
        value=result();value["passed"]=True;value["samples"][-1]["groups"]["browser"]["sampled_open_descriptors"]+=1
        self.assertFalse(idle_verdict(value))

        value=result();value["policy"]=copy.deepcopy(POLICY);value["policy"]["rss_retention_bytes"]=1024
        self.assertFalse(idle_verdict(value))

        # Prospective single-cycle diagnostics use exactly the owner-approved
        # capacity bars, without claiming a completed three-cycle qualification.
        from lib.matrix_plateau import (CYCLE_POLICY, FOCUSED_PROFILE, SERVER_SOURCE_INPUTS,
            runtime_capacity_configuration, focused_capacity_evaluation)
        focused = result(); focused["policy"] = copy.deepcopy(CYCLE_POLICY)
        focused["focused_capacity_profile"] = FOCUSED_PROFILE
        focused["persistent_owners"] = dict(server=dict(pid=101, start_identity=1001),
                                           browser=dict(pid=202, start_identity=2002))
        focused["server_runtime_capacity"] = runtime_capacity_configuration(
            focused["persistent_owners"]["server"], SERVER_SOURCE_INPUTS)
        focused["samples"].insert(2, sample(19, "workload"))
        for observed in [focused["baseline"], *focused["samples"]]:
            observed["usage"]["owned_tasks"] = 0
            if observed["stage"] == "workload":
                for group in observed["groups"].values():
                    group.update(live_processes=2, sampled_threads=20, sampled_open_descriptors=40)
            if observed["stage"] in {"idle", "idle-final"}:
                for group in observed["groups"].values():
                    group.update(sampled_threads=10, sampled_open_descriptors=20,
                                 sampled_rss_bytes=100*1024*1024)
        evaluate = lambda value: focused_capacity_evaluation(value, SERVER_SOURCE_INPUTS)
        positive = evaluate(focused)
        self.assertTrue(positive["passed"], positive)
        self.assertFalse(positive["full_three_cycle_capacity_qualified"])
        self.assertFalse(positive["separate_soak"]["qualified"])
        self.assertFalse(restoration(focused["baseline"], focused["baseline"]))
        self.assertTrue(restoration(focused["baseline"], focused["samples"][-1]),
                        "Legacy cold policy remains distinct; no historical regrade")
        for role, field, value in (("browser", "sampled_rss_bytes", 4*1024**3+1),
                                   ("server", "sampled_rss_bytes", 1024**3+1),
                                   ("browser", "sampled_open_descriptors", 41),
                                   ("server", "sampled_threads", 515)):
            bad = copy.deepcopy(focused); bad["samples"][-1]["groups"][role][field] = value
            self.assertFalse(evaluate(bad)["passed"], (role, field))
        bad = copy.deepcopy(focused); bad["samples"][-1]["usage"]["owned_tasks"] = 1
        self.assertFalse(evaluate(bad)["passed"])
        bad = copy.deepcopy(focused); bad["samples"][-1]["usage"]["pool:webtransport"] = 1
        self.assertFalse(evaluate(bad)["passed"])
        bad = copy.deepcopy(focused); bad["server_runtime_capacity"]["max_blocking_threads"] = 513
        self.assertFalse(evaluate(bad)["passed"])
        bad = copy.deepcopy(focused); bad["samples"] = [s for s in bad["samples"] if s["stage"] != "workload"]
        self.assertFalse(evaluate(bad)["passed"])
        bad = copy.deepcopy(focused); bad["samples"][-1]["monotonic_seconds"] = 29.9
        self.assertFalse(evaluate(bad)["passed"])
        bad = copy.deepcopy(focused); bad["samples"][-1]["groups"]["browser"]["root_state"] = "retired"
        self.assertFalse(evaluate(bad)["passed"])
        # Stable unknown owners must not disappear from budgets merely because
        # original workload samples contain only the two declared roles.
        bad = copy.deepcopy(focused)
        for observed in [bad["baseline"], *bad["samples"]]:
            if observed["stage"] != "workload":
                observed["groups"]["helper"] = copy.deepcopy(observed["groups"]["browser"])
        self.assertFalse(evaluate(bad)["passed"])
        bad = copy.deepcopy(focused); bad["samples"][-1]["groups"]["browser"]["live_processes"] = 3
        self.assertFalse(evaluate(bad)["passed"])
        # A known partial RSS breach is never discarded as unavailable counts.
        bad = copy.deepcopy(focused); bad["samples"].insert(3, dict(stage="workload", monotonic_seconds=19.5,
            groups={"server": {"sampled_rss_bytes": 1024**3+1}}))
        self.assertFalse(evaluate(bad)["passed"])

    def test_idle_check_is_independent_of_failed_media_but_requires_all_original_joins(self):
        from lib.transport_matrix import inventory
        from lib.transport_matrix_execution import configure_execution, record_browser_result
        import test_browser_matrix_execution as browser_tests
        factory = browser_tests.BrowserMatrixExecution()
        for missing in (None, "browser", "server", "queue"):
            matrix = factory.matrix()
            matrix = configure_execution(inventory(Path(__file__).resolve().parents[2], "a" * 40),
                matrix["execution_environment"], "quality")
            cell = next(c for c in matrix["cells"] if c["execution"]["runnable"])
            report = factory.result(matrix, cell);report.update(stage="quality", passed=False,
                idle_resources={**result(), "passed": True}, servers=[dict(original_joined=True)] * 2,
                owned_audio=dict(original_process_joined=True, forced_kill=False))
            observation=report["browser_observation"];observation["passed"]=False
            observation.update(idle_resources=dict(passed=True, localOwnersJoined=True, media_peers=5,
                renderer=dict(joined=True, owners=10), active_bindings=0, queued_packets=0, queued_bytes=0),
                retirement=dict(passed=True, allLocalOwnersElapsedMs=300, originalEndsCompleted=6, originalSessionsEnded=2, originalClientsClosed=7),
                trusted_browser=dict(original_process_joined=True, forced_kill=False))
            if missing=="browser": observation["trusted_browser"]["original_process_joined"]=False
            if missing=="server": report["servers"]=[dict(original_joined=False)] * 2
            if missing=="queue": observation["idle_resources"]["queued_packets"]=1
            record_browser_result(matrix, cell, report, "c" * 64, 1)
            self.assertEqual(cell["status"], "failed")
            self.assertEqual(cell["check_results"]["owned-drain-and-resource-reclamation"]["status"],
                "passed" if missing is None else "failed")
            self.assertFalse(matrix["matrix_qualified"])


class MatrixCycleIdleTests(unittest.TestCase):
    def test_cycle_collector_keeps_fresh_baseline_and_original_session_reference(self):
        from lib.matrix_idle import MatrixIdle
        from lib.matrix_plateau import CYCLE_POLICY
        from unittest.mock import patch
        import json
        import tempfile
        class Sampler:
            roots = {"server": (101, 1001), "browser": (202, 2002)}
            def sample(self):
                value = sample(0, "ignored")["groups"]
                for group in value.values():
                    group["sampled_rss_bytes"] += 8 * 1024 * 1024
                    group["sampled_threads"] += 2
                clock[0] += .001
                return {"groups": value}
            def summary(self): return {}
        original = sample(10, "baseline"); original["usage"]["owned_tasks"] = 0
        for focused in (False, True):
            with tempfile.TemporaryDirectory() as folder:
                clock = [100.0]
                def request(phase, at):
                    Path(folder, "resources-" + phase + ".json").write_text(json.dumps(dict(phase=phase, monotonic_seconds=at)))
                with patch("lib.matrix_idle.time.monotonic", side_effect=lambda: clock[0]), \
                     patch("lib.matrix_idle.owned_usage", return_value=original["usage"]), \
                     patch("lib.matrix_idle.urllib.request.build_opener") as opener, \
                     patch("lib.matrix_idle.idle_owner_diagnostics", return_value={"diagnostic": True}):
                    opener.return_value.open.return_value.__enter__.return_value.read.return_value = b'{"result":{"Ok":{"kind":"health","data":{}}}}'
                    idle = MatrixIdle(folder, 101, 202, "http://fixture", "credential", sampler=Sampler(),
                        **(dict(focused_capacity=True) if focused else dict(cycle_mode=True, original_baseline=original)))
                    request("baseline", 100); idle.poll(); clock[0] = 101; idle.poll()
                    self.assertEqual(idle.baseline["monotonic_seconds"], 101)
                    if focused:
                        self.assertIsNone(idle.reference_baseline)
                    else:
                        self.assertEqual(idle.reference_baseline["monotonic_seconds"], 10)
                    clock[0] = 119; idle.poll()  # The same original workload, before its terminal.
                    request("terminal", 120); request("retired", 120.1)
                    for at in [121, 127, 128, 130.01]: clock[0] = at; idle.poll()
                    observed = idle.finish()
                self.assertTrue(observed["passed"])
                self.assertEqual(observed["policy"], CYCLE_POLICY)
                self.assertEqual(observed["baseline"]["monotonic_seconds"], 101)
                self.assertEqual(observed["persistent_owners"]["server"]["pid"], 101)
                self.assertEqual(observed["persistent_owners"]["browser"]["start_identity"], 2002)
                if focused:
                    self.assertTrue(observed["focused_capacity"]["passed"])
                    self.assertFalse(observed["full_three_cycle_capacity_qualified"])
                    self.assertFalse(observed["focused_capacity"]["separate_soak"]["qualified"])
                current = sample(131, "idle"); current["usage"]["owned_tasks"] = 1
                self.assertIn("owned_tasks", idle.restore(original, current))



if __name__ == "__main__": unittest.main()
