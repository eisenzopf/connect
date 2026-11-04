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


if __name__ == "__main__": unittest.main()
