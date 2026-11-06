"""Original PID custody, bounded private diagnostics, and unchanged verdicts."""
from pathlib import Path
import ctypes
import importlib.util
import json
import os
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from lib.matrix_owner_diagnostics import idle_owner_diagnostics, memory_fields, vmmap_summary, mac_vmmap, browser_labels, executable_label, stat_identity
from lib.operational_process_resources import ProcessResources
from lib.matrix_macos_resources import MacProcessResources


def raw_stat(pid, start):
    values = ["S", "0", str(pid)] + ["0"] * 19
    values[11], values[17], values[19], values[21] = "4", "1", str(start), "3"
    return str(pid) + " (unretained private name) " + " ".join(values)


class DiagnosticsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.proc = Path(self.temp.name)
        self.base = self.proc / "123"
        task = self.base / "task/123"
        task.mkdir(parents=True)
        (self.base / "stat").write_text(raw_stat(123, 17))
        (task / "stat").write_text(raw_stat(123, 17))
        (task / "children").write_text("")
        (task / "comm").write_text("private user-supplied name\n")
        (task / "wchan").write_text("futex_wait_queue")
        (self.base / "fd").mkdir()
        (self.base / "fd/3").symlink_to("/private/fixture-secret-name")
        (self.base / "status").write_text("Name:\tprivate\nVmRSS:\t128 kB\nRssAnon:\t64 kB\n")
        (self.base / "smaps_rollup").write_text("000-fff private mapping name\nRss: 128 kB\nPrivate_Dirty: 64 kB\n")
        self.sampler = ProcessResources({"server": 123}, self.proc, 100, 4096, roles=("server",))

    def test_numeric_thread_memory_and_descriptor_custody_excludes_private_names_and_paths(self):
        result = idle_owner_diagnostics(self.sampler)
        self.assertEqual(result["state"], "available")
        process = result["processes"][0]
        self.assertEqual((process["pid"], process["start_ticks"]), (123, 17))
        self.assertEqual(process["threads"][0]["wait_channel"], "futex_wait_queue")
        self.assertEqual(process["smaps_rollup"]["Private_Dirty_bytes"], 65536)
        self.assertEqual(process["descriptors"][0]["kind"], "file")
        self.assertNotIn("fixture-secret-name", json.dumps(result))
        self.assertNotIn("user-supplied", json.dumps(result))
        self.assertNotIn(str(self.proc), json.dumps(result))

    def test_reused_original_pid_is_not_observed_as_the_owner(self):
        (self.base / "stat").write_text(raw_stat(123, 99))
        result = idle_owner_diagnostics(self.sampler)
        self.assertEqual(result["state"], "partial")
        self.assertEqual(result["processes"], [])
        self.assertEqual(result["original_root_states"]["server"], "retired-or-reused")

    def test_identity_change_mid_snapshot_discards_unbound_process_data(self):
        original = self.sampler._stat
        calls = 0
        def changing(pid):
            nonlocal calls
            calls += 1
            value = original(pid)
            if calls == 2: value["start_ticks"] += 1
            return value
        self.sampler._stat = changing
        result = idle_owner_diagnostics(self.sampler)
        self.assertEqual(result["state"], "unavailable")
        self.assertEqual(result["processes"], [])

    def test_time_limit_refuses_more_observation_without_touching_verdict(self):
        times = iter([0, .06, .06])
        result = idle_owner_diagnostics(self.sampler, clock=lambda: next(times))
        self.assertEqual(result["state"], "unavailable")
        self.assertEqual(result["error"], "ObservationDeadline")
        self.assertNotIn("passed", result)

    def test_cross_host_collector_never_uses_guest_pids_on_mac(self):
        class Composite:
            server = self.sampler
        result = idle_owner_diagnostics(Composite(), roles=("server", "helper"))
        self.assertEqual(result["state"], "partial")
        self.assertEqual(result["unavailable_roles"], ["helper"])
        self.assertEqual({value["role"] for value in result["processes"]}, {"server"})

    def test_snapshot_detail_failure_is_explicit_and_does_not_invent_memory(self):
        (self.base / "smaps_rollup").unlink()
        result = idle_owner_diagnostics(self.sampler)
        memory = result["processes"][0]["smaps_rollup"]
        self.assertEqual(memory["state"], "unavailable")
        self.assertNotIn("Rss_bytes", memory)

    def test_vmmap_category_names_do_not_turn_writable_or_virtual_bytes_into_live_heap(self):
        result = vmmap_summary(b"ReadOnly portion of Libraries: Total=1.5G resident=128M(8%) swapped_out_or_unallocated=1.4G(92%)\nWritable regions: Total=256M written=64M(25%) resident=32M(12%) swapped_out=0K(0%) unallocated=192M(75%)\nPhysical footprint: 40.5M\nprivate mapping path\n")
        self.assertEqual(result["numeric_categories"]["readonly_libraries_resident_bytes"], 128 * 1024 * 1024)
        self.assertEqual(result["numeric_categories"]["writable_regions_resident_bytes"], 32 * 1024 * 1024)
        self.assertEqual(result["numeric_categories"]["physical_footprint_bytes"], int(40.5 * 1024 * 1024))
        self.assertIsNone(result["private_heap_bytes"])
        self.assertIsNone(result["anonymous_bytes"])
        self.assertNotIn("private mapping path", json.dumps(result))
        self.assertEqual(vmmap_summary(b"unavailable: no task access")["numeric_categories"], {})

    def test_native_diagnostics_follow_original_sample_and_do_not_move_its_clock_or_schedule(self):
        from lib.native_matrix_idle import NativeMatrixIdle
        when = [100.0]
        calls = []
        groups = {"server": {"sampled_rss_bytes": 100}, "helper": {"sampled_rss_bytes": 200}}
        class Sampler:
            def sample(self):
                calls.append("original-counters")
                return {"groups": groups}
        def diagnose(*args, **kwargs):
            calls.append("diagnostics")
            when[0] += .02
            return dict(state="unavailable")
        idle = NativeMatrixIdle(self.proc, Sampler(), lambda: {"physical_owners": 0}, clock=lambda: when[0])
        with patch("lib.native_matrix_idle.idle_owner_diagnostics", side_effect=diagnose):
            sample = idle.take("baseline")
            self.assertEqual(calls, ["original-counters", "diagnostics"])
            self.assertEqual(sample["monotonic_seconds"], 100)
            self.assertEqual(idle.next_sample, 101)
            self.assertIs(sample["groups"], groups)
            calls.clear(); idle.take("idle")
            self.assertEqual(calls, ["original-counters"])

    def test_vmmap_timeout_joins_only_its_original_probe_and_keeps_raw_output_private(self):
        class Collector:
            def read(self, *_):
                value = bytearray(136); struct.pack_into("=I", value, 20, os.getuid()); return bytes(value)
            def identity(self, _):
                return {"start": (1, 2)}
        class Probe:
            pid = 456
            killed = False
            waits = []
            def wait(self, timeout=None):
                self.waits.append(timeout)
                if timeout is not None: raise subprocess.TimeoutExpired("owned probe", timeout)
                return -9
            def kill(self): self.killed = True
        probe = Probe()
        def launch(command, **kwargs):
            self.assertEqual(command, ["/usr/bin/vmmap", "-summary", "123"])
            kwargs["stdout"].write(b"private mapping names are never public\n")
            return probe
        with patch("lib.matrix_owner_diagnostics.subprocess.Popen", side_effect=launch), patch("lib.matrix_owner_diagnostics.Path.is_file", return_value=True):
            result = mac_vmmap(Collector(), 123, (1, 2), self.proc / "private-output", "idle-final")
        self.assertEqual(result["reason"], "original-probe-timeout")
        self.assertTrue(result["original_probe_joined"])
        self.assertTrue(result["probe_forced"])
        self.assertEqual(probe.waits, [.5, None])
        self.assertNotIn("private mapping names", json.dumps(result))
        self.assertEqual((self.proc / "private-output" / result["artifact"]).stat().st_mode & 0o077, 0)

    @unittest.skipUnless(sys.platform == "darwin", "actual local macOS libproc boundary")
    def test_original_macos_process_thread_ids_and_descriptors_are_observed_then_child_joins(self):
        child = subprocess.Popen([sys.executable, "-c", "import sys;print('ready',flush=True);sys.stdin.buffer.read()"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        try:
            self.assertEqual(child.stdout.readline(), b"ready\n")
            collector = MacProcessResources({"server": child.pid}, roles=("server",))
            result = idle_owner_diagnostics(collector)
            self.assertEqual(result["state"], "available", result)
            process = result["processes"][0]
            self.assertEqual(process["pid"], child.pid)
            self.assertGreater(len(process["threads"]), 0, process)
            self.assertGreater(len(process["descriptors"]), 0, process)
        finally:
            child.stdin.close()
            self.assertEqual(child.wait(timeout=3), 0)
            child.stdout.close(); child.stderr.close()


class LinuxBrowserAttributionTests(unittest.TestCase):
    setUp = DiagnosticsTests.setUp
    def browser(self):
        (self.base / "cmdline").write_bytes(b"/private/browser\0--type=utility\0--utility-sub-type=network.mojom.NetworkService\0--private-token=never-record\0")
        (self.base / "task/123/comm").write_text("Chrome_IOThread\n")
        (self.base / "fd/3").unlink()
        (self.base / "fd/3").symlink_to(self.base / "status")
        for number, target in [(4, "socket:[471]"), (5, "pipe:[815]"), (6, "anon_inode:[eventpoll]")]:
            (self.base / "fd" / str(number)).symlink_to(target)
        return ProcessResources({"browser": 123}, self.proc, 100, 4096, roles=("browser",))

    def child(self, pid):
        base = self.proc / str(pid)
        task = base / "task" / str(pid)
        task.mkdir(parents=True)
        (base / "stat").write_text(raw_stat(pid, pid).replace(") S 0 ", ") S 123 "))
        (task / "stat").write_text(raw_stat(pid, pid))
        (task / "comm").write_text("private-thread-label\n")
        (task / "wchan").write_text("futex_wait_queue")
        (task / "children").write_text("")
        (base / "fd").mkdir()
        (base / "status").write_text("VmRSS: 1 kB\n")
        (base / "smaps_rollup").write_text("Rss: 1 kB\n")
        (base / "cmdline").write_bytes(b"/private/browser\0--type=renderer\0")
        return base

    def test_original_browser_labels_fd_classes_and_names_are_closed_and_pid_bound(self):
        sampler = self.browser()
        result = idle_owner_diagnostics(sampler, roles=("browser",), clock=lambda: 0)
        self.assertEqual(result["state"], "available")
        process = result["processes"][0]
        self.assertEqual(process["identity_before"], {"pid": 123, "start_ticks": 17})
        self.assertEqual(process["identity_after"], process["identity_before"])
        self.assertEqual(process["browser_labels"], {"state": "available", "process_type": "utility",
            "utility_sub_type": "network.mojom.NetworkService"})
        self.assertEqual(process["descriptor_classes"]["counts"],
            {"socket": 1, "pipe": 1, "anonymous-inode": 1, "regular": 1, "other": 0})
        self.assertEqual(process["threads"][0]["recognized_label"], "Chrome_IOThread")
        self.assertNotIn("private-token", json.dumps(result))
        self.assertNotIn("never-record", json.dumps(result))
        self.assertNotIn(str(self.proc), json.dumps(result))
        self.assertNotIn("/private/browser", json.dumps(result))

    def test_unknown_switch_values_are_redacted_missing_switches_stay_explicit(self):
        result = browser_labels(b"browser\0--type=private-user-label\0--utility-sub-type=private-endpoint\0")
        self.assertEqual(result["process_type"], "unrecognized")
        self.assertEqual(result["utility_sub_type"], "unrecognized")
        self.assertNotIn("private", json.dumps(result))
        absent = browser_labels(b"browser\0--private-token=secret\0")
        self.assertIsNone(absent["process_type"])
        self.assertIsNone(absent["utility_sub_type"])
        with self.assertRaises(ValueError):
            browser_labels(b"browser\0--type=renderer\0--type=gpu-process\0")
        with self.assertRaises(ValueError):
            browser_labels(b"browser\0--type\0")
        with self.assertRaises(ValueError):
            browser_labels(b"browser\0--type=renderer")

    def test_missing_or_oversize_argv_is_unavailable_without_fabricated_class(self):
        sampler = self.browser()
        for content in (None, b"x" * 16385):
            path = self.base / "cmdline"
            if path.exists(): path.unlink()
            if content is not None: path.write_bytes(content)
            result = idle_owner_diagnostics(sampler, roles=("browser",), clock=lambda: 0)
            labels = result["processes"][0]["browser_labels"]
            self.assertEqual(labels["state"], "unavailable")
            self.assertNotIn("process_type", labels)

    def test_only_already_adopted_original_descendants_are_observed(self):
        sampler = self.browser()
        self.child(456); self.child(789)
        (self.base / "task/123/children").write_text("456")
        sampler.sample()
        self.assertIn((456, 456), sampler.known)
        original = sampler._stat
        visited = []
        def tracked(pid):
            visited.append(pid)
            return original(pid)
        sampler._stat = tracked
        result = idle_owner_diagnostics(sampler, roles=("browser",), clock=lambda: 0)
        self.assertEqual({p["pid"] for p in result["processes"]}, {123, 456})
        self.assertNotIn(789, visited)

    def test_pid_reuse_after_labels_discards_all_unbound_browser_details(self):
        sampler = self.browser()
        from lib.matrix_owner_diagnostics import bounded_bytes
        def changing(path, maximum):
            value = bounded_bytes(path, maximum)
            if Path(path).name == "cmdline":
                (self.base / "stat").write_text(raw_stat(123, 99))
            return value
        with patch("lib.matrix_owner_diagnostics.bounded_bytes", side_effect=changing):
            result = idle_owner_diagnostics(sampler, roles=("browser",), clock=lambda: 0)
        self.assertEqual(result["state"], "unavailable")
        self.assertEqual(result["processes"], [])
        self.assertNotIn("NetworkService", json.dumps(result))

    def test_descriptor_retirement_or_unreadable_target_is_partial_never_zero(self):
        sampler = self.browser()
        (self.base / "fd/3").unlink()
        (self.base / "fd/3").symlink_to("/nonexistent/private-target")
        result = idle_owner_diagnostics(sampler, roles=("browser",), clock=lambda: 0)
        classes = result["processes"][0]["descriptor_classes"]
        self.assertEqual(classes["state"], "partial")
        self.assertIsNone(classes["counts"])
        self.assertEqual(classes["unavailable_or_retired"], 1)
        self.assertEqual(classes["observed_counts"]["socket"], 1)
        self.assertNotIn("private-target", json.dumps(result))

    def test_process_and_descriptor_bounds_are_explicit_without_new_discovery(self):
        sampler = self.browser()
        self.child(456); self.child(789)
        (self.base / "task/123/children").write_text("456 789")
        sampler.sample()
        with patch("lib.matrix_owner_diagnostics.MAX_LIVE", 2):
            result = idle_owner_diagnostics(sampler, roles=("browser",), clock=lambda: 0)
        self.assertEqual(result["state"], "partial")
        self.assertTrue(result["truncated"])
        self.assertEqual([p["pid"] for p in result["processes"]], [123, 789])
        self.assertEqual(result["unexamined_known_identities"],
            [{"pid": 456, "birth": 456, "role": "browser"}])
        self.assertNotIn("descriptors", result["unexamined_known_identities"][0])
        with patch("lib.matrix_owner_diagnostics.MAX_DESCRIPTORS", 2):
            result = idle_owner_diagnostics(sampler, roles=("browser",), clock=lambda: 0)
        self.assertEqual(result["state"], "unavailable")
        self.assertEqual(result["processes"], [])
        self.assertNotIn("counts", result)

    def test_browser_diagnostics_remain_post_counter_only_with_same_zero_retention_policy(self):
        from lib.matrix_idle import MatrixIdle, POLICY, restoration
        calls = []
        groups = {role: dict(root_state="available", unavailable_processes=0,
            open_resource_unavailable_processes=0, live_processes=1,
            sampled_threads=2, sampled_open_descriptors=3, sampled_rss_bytes=4)
            for role in ("server", "browser")}
        class Sampler:
            def sample(self):
                calls.append("original-counters")
                return {"groups": groups}
        def diagnose(*args, **kwargs):
            calls.append("diagnostics")
            self.assertEqual(kwargs["roles"], ("server", "browser"))
            return {"state": "unavailable", "truncated": True}
        idle = MatrixIdle(self.proc, 123, 456, "http://127.0.0.1:1/v1/operations",
            "synthetic", sampler=Sampler())
        with patch("lib.matrix_idle.urllib.request.build_opener") as opener, \
                patch("lib.matrix_idle.owned_usage", return_value={"physical_owners": 0}), \
                patch("lib.matrix_idle.time.monotonic", return_value=100), \
                patch("lib.matrix_idle.idle_owner_diagnostics", side_effect=diagnose):
            opener.return_value.open.return_value.__enter__.return_value.read.return_value = \
                json.dumps({"result": {"Ok": {"kind": "health", "data": {}}}}).encode()
            original = idle.take("baseline")
            self.assertEqual(calls, ["original-counters", "diagnostics"])
            self.assertEqual(original["monotonic_seconds"], 100)
            self.assertEqual(idle.next_sample, 101)
            calls.clear()
            idle.take("idle")
            self.assertEqual(calls, ["original-counters"])
        observed = dict(original, owner_diagnostics={"state": "unavailable", "truncated": True})
        self.assertEqual(restoration(original, observed), [])
        self.assertEqual((POLICY["idle_seconds"], POLICY["rss_retention_bytes"],
            POLICY["extra_descriptors"], POLICY["extra_threads"]), (10, 0, 0, 0))


class BrowserTaxonomyAndDescriptorIdentityTests(unittest.TestCase):
    setUp = DiagnosticsTests.setUp
    browser = LinuxBrowserAttributionTests.browser
    def test_closed_executable_labels_redact_paths_and_unknown_names(self):
        self.assertEqual(executable_label("/private/path/chrome"), "chrome")
        self.assertEqual(executable_label("/private/path/chrome (deleted)"), "chrome")
        self.assertEqual(executable_label("/private/path/unknown-secret"), "unrecognized")

    def test_chromium_single_flattened_title_preserves_only_known_title_tokens(self):
        raw = b"/private/path/chrome --type=utility --utility-sub-type=audio.mojom.AudioService --private-token=secret\0\0\0"
        observed = browser_labels(raw, executable="chrome")
        self.assertEqual(observed, {"state": "available", "process_type": "utility",
            "utility_sub_type": "audio.mojom.AudioService", "representation": "flattened-process-title"})
        self.assertNotIn("private", json.dumps(observed))
        self.assertNotIn("secret", json.dumps(observed))
        self.assertIsNone(browser_labels(raw)["process_type"])
        self.assertEqual(browser_labels(b"/private/chrome --profile=secret --type=renderer\0", executable="chrome")["state"], "partial")
        with self.assertRaises(ValueError):
            browser_labels(b"/private/chrome --type=renderer --private=secret --type=utility\0", executable="chrome")

    def test_fd_stat_identity_distinguishes_same_label_different_inodes_without_paths(self):
        from types import SimpleNamespace
        a = SimpleNamespace(st_dev=1, st_ino=2, st_mode=0o100600)
        b = SimpleNamespace(st_dev=1, st_ino=3, st_mode=0o100600)
        self.assertNotEqual(stat_identity(a), stat_identity(b))
        self.assertEqual(stat_identity(a), stat_identity(a))
        sampler = self.browser()
        (self.base / "fd/7").symlink_to(self.base / "status")
        observed = idle_owner_diagnostics(sampler, roles=("browser",), clock=lambda: 0)
        descriptors = {fd["fd"]: fd for fd in observed["processes"][0]["descriptors"]}
        self.assertEqual(descriptors[3]["stat_identity_sha256"], descriptors[7]["stat_identity_sha256"])
        self.assertIsNotNone(descriptors[3]["stat_identity_sha256"])
        self.assertIsNone(descriptors[4]["stat_identity_sha256"])
        self.assertNotIn(str(self.proc), json.dumps(observed))

    def test_changed_executable_on_same_pid_birth_discards_original_process_details(self):
        sampler = self.browser()
        directory = self.proc / "private-bin"
        directory.mkdir()
        one, two = directory / "chrome", directory / "firefox"
        one.write_bytes(b"synthetic-original-executable")
        two.write_bytes(b"synthetic-other-executable")
        exe = self.base / "exe"
        exe.symlink_to(one)
        from lib.matrix_owner_diagnostics import bounded_bytes
        def changing(path, maximum):
            value = bounded_bytes(path, maximum)
            if Path(path).name == "cmdline": exe.unlink(); exe.symlink_to(two)
            return value
        with patch("lib.matrix_owner_diagnostics.bounded_bytes", side_effect=changing):
            observed = idle_owner_diagnostics(sampler, roles=("browser",), clock=lambda: 0)
        self.assertEqual(observed["processes"], [])
        self.assertEqual(observed["state"], "unavailable")

    def test_raced_descriptor_target_keeps_stat_identity_unavailable(self):
        sampler = self.browser()
        original = os.readlink
        reads = 0
        def raced(path):
            nonlocal reads
            value = original(path)
            if str(path).endswith("/fd/3"):
                reads += 1
                if reads > 1: return "/private/changed-file"
            return value
        with patch("lib.matrix_owner_diagnostics.os.readlink", side_effect=raced):
            observed = idle_owner_diagnostics(sampler, roles=("browser",), clock=lambda: 0)
        process = observed["processes"][0]
        self.assertIsNone(process["descriptor_classes"]["counts"])
        descriptor = next(fd for fd in process["descriptors"] if fd["fd"] == 3)
        self.assertIsNone(descriptor["stat_identity_sha256"])
        self.assertEqual(descriptor["classification"], "unavailable")
        self.assertNotIn("changed-file", json.dumps(observed))


if __name__ == "__main__":
    unittest.main()
