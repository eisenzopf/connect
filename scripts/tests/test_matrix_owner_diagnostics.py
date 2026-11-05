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
from lib.matrix_owner_diagnostics import idle_owner_diagnostics, memory_fields, vmmap_summary, mac_vmmap
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


if __name__ == "__main__":
    unittest.main()
