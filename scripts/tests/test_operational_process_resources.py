"""PID reuse, role separation and numeric-only process observation boundaries."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import subprocess
import errno
from unittest import mock

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location("operational_resources",ROOT/"scripts/lib/operational_process_resources.py")
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)

class ProcessBoundary(unittest.TestCase):
    def test_browser_startup_failure_exposes_only_closed_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            private = Path(directory)
            request = private / "input.json"
            request.write_text(json.dumps(dict(dist=str(private / "secret-path-missing"),
                credential="secret-bearer-must-not-escape", endpoint="https://private.invalid")))
            result = subprocess.run(["node", str(ROOT / "scripts/operational-browser.mjs"), str(request)],
                capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(result.stderr, "")
            self.assertEqual(json.loads(result.stdout), dict(phase="failed", stage="browser-sdk", name="Error", code="ENOENT"))
            self.assertNotIn("secret", result.stdout)

    def setUp(self):
        self.temp=tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.proc=Path(self.temp.name)
        for pid in (100,200,300):
            self.write(pid,0,pid,1,2,3)

    def write(self,pid,parent,group,start,cpu,rss,children=""):
        path=self.proc/str(pid);path.mkdir(exist_ok=True)
        fields=["S","0","0"]+["0"]*19
        fields[1]=str(parent);fields[2]=str(group);fields[11]=str(cpu);fields[12]="0"
        fields[19]=str(start);fields[21]=str(rss)
        (path/"stat").write_text(str(pid)+" (private name ) secret) "+" ".join(fields))
        p=path/"task"/str(pid);p.mkdir(parents=True,exist_ok=True);(p/"children").write_text(children)

    def sampler(self):
        return module.ProcessResources(dict(server=100,helper=200,browser=300),self.proc,100,4096)

    def test_separate_owned_groups_preserve_numeric_metadata_without_names(self):
        self.write(300,0,300,1,2,3,"301")
        self.write(301,300,300,4,5,7)
        sample=self.sampler();value=sample.sample()
        self.assertEqual(value["groups"]["browser"]["live_processes"],2)
        self.assertEqual(value["groups"]["browser"]["sampled_rss_bytes"],10*4096)
        self.assertEqual(value["groups"]["browser"]["observed_cpu_seconds"],.07)
        self.assertEqual(value["groups"]["helper"]["live_processes"],1)
        text=json.dumps(sample.summary())
        self.assertNotIn("secret",text);self.assertNotIn("private name",text);self.assertNotIn(str(self.proc),text)

    def test_nonleader_worker_task_children_remain_in_original_helper_group(self):
        task = self.proc / "200/task/207"; task.mkdir(); (task / "children").write_text("201")
        self.write(201, 200, 200, 4, 5, 7)
        sampler = self.sampler(); value = sampler.sample()["groups"]["helper"]
        self.assertEqual(value["live_processes"], 2)
        self.assertEqual(value["sampled_rss_bytes"], 10 * 4096)
        self.assertTrue(any(key[0] == 201 for key in sampler.known))
        # A nonleader's stale PID entry cannot borrow a different current parent.
        self.write(201, 100, 100, 99, 900, 900)
        value = sampler.sample()["groups"]["helper"]
        self.assertEqual(value["live_processes"], 1)
        self.assertFalse((201, 99) in sampler.known)

    def test_reused_root_pid_cannot_adopt_unrelated_descendants(self):
        sample=self.sampler();sample.sample()
        self.write(300,0,300,99,900,900,"301")
        self.write(301,300,300,100,900,900)
        value=sample.sample()["groups"]["browser"]
        self.assertEqual(value["root_state"],"retired")
        self.assertEqual(value["live_processes"],0)
        self.assertEqual(value["observed_cpu_seconds"],.02)
        self.assertFalse(any(key[0]==301 for key in sample.known))

    def test_missing_or_unreadable_metadata_is_not_zero_usage_availability(self):
        sample=self.sampler()
        (self.proc/"100"/"stat").unlink()
        (self.proc/"200"/"stat").write_text("bad")
        value=sample.sample()["groups"]
        self.assertEqual(value["server"]["root_state"],"retired")
        self.assertEqual(value["helper"]["root_state"],"unavailable")
        self.assertEqual(value["helper"]["unavailable_processes"],1)

    def test_duplicate_roots_and_cross_role_children_refuse(self):
        with self.assertRaises(ValueError):
            module.ProcessResources(dict(server=100,helper=100,browser=300),self.proc,100,4096)
        sample=self.sampler()
        self.write(200,300,200,1,2,3)
        self.write(300,0,300,1,2,3,"200")
        with self.assertRaisesRegex(ValueError,"overlap"):sample.sample()

    def test_native_two_roles_preserve_descriptor_unavailability_and_pid_identity(self):
        for pid, count in [(100, 2), (200, 3)]:
            directory = self.proc / str(pid) / "fd"
            directory.mkdir()
            for index in range(count): (directory / str(index)).touch()
        sampler = module.ProcessResources(dict(server=100,helper=200),self.proc,100,4096,
            roles=("server","helper"),open_resources=True,max_samples=3)
        groups = sampler.sample()["groups"]
        self.assertEqual(set(groups),{"server","helper"})
        self.assertEqual(groups["server"]["sampled_open_descriptors"],2)
        self.assertEqual(groups["helper"]["sampled_open_descriptors"],3)
        for p in (self.proc / "200/fd").iterdir(): p.unlink()
        (self.proc / "200/fd").rmdir()
        helper = sampler.sample()["groups"]["helper"]
        self.assertEqual(helper["root_state"],"available")
        self.assertIsNone(helper["sampled_open_descriptors"])
        self.assertEqual(helper["open_resource_unavailable_processes"],1)
        self.write(200,0,200,99,999,999,"201")
        self.write(201,200,200,99,999,999)
        helper = sampler.sample()["groups"]["helper"]
        self.assertEqual(helper["root_state"],"retired")
        self.assertEqual(helper["live_processes"],0)
        self.assertFalse(any(key[0]==201 for key in sampler.known))
        self.assertEqual(sampler.summary()["groups"]["helper"]["open_resource_unavailable_samples"],1)
        with self.assertRaisesRegex(ValueError,"sample budget"): sampler.sample()
        observed=json.dumps(sampler.summary())
        self.assertNotIn("private name",observed)
        self.assertNotIn(str(self.proc),observed)

    def _set_state(self, pid, state, threads, start=None):
        path = self.proc / str(pid) / "stat"
        raw = path.read_text(); boundary = raw.rfind(")") + 2
        fields = raw[boundary:].split(); fields[0] = state; fields[17] = str(threads)
        if start is not None: fields[19] = str(start)
        path.write_text(raw[:boundary] + " ".join(fields))

    def _fd_sampler(self):
        for pid, count in [(100, 2), (200, 3)]:
            self._set_state(pid, "S", 1)
            directory = self.proc / str(pid) / "fd"; directory.mkdir(exist_ok=True)
            for index in range(count): (directory / str(index)).touch()
        return module.ProcessResources(dict(server=100, helper=200), self.proc, 100, 4096,
            roles=("server", "helper"), open_resources=True, max_samples=40)

    def test_original_fd_read_race_requires_confirmed_single_task_terminal_identity(self):
        # OLD classifier sees the same birth after ENOENT and falsely records
        # unavailable FD data. The fixed result must be original retirement,
        # never a successful live owner with an invented zero descriptor count.
        sampler = self._fd_sampler(); real_scan = module.os.scandir
        def scan(path):
            if Path(path) == self.proc / "200/fd":
                self._set_state(200, "Z", 1)
                raise FileNotFoundError(errno.ENOENT, "private secret path", str(path))
            return real_scan(path)
        with mock.patch.object(module.os, "scandir", side_effect=scan):
            helper = sampler.sample()["groups"]["helper"]
        self.assertEqual(helper["root_state"], "retired")
        self.assertEqual(helper["live_processes"], 0)
        self.assertEqual(helper["retired_processes"], 1)
        self.assertEqual(helper["open_resource_unavailable_processes"], 0)
        event = sampler.summary()["fd_observation_diagnostics"]["events"]["retired"][0]
        self.assertEqual((event["pid"], event["original_start_ticks"]), (200, 1))
        self.assertEqual(event["before"]["state"], "S")
        self.assertEqual(event["after"], dict(start_ticks=1, state="Z", threads=1))
        self.assertEqual(event["confirmation"], event["after"])
        self.assertEqual(event["error"], dict(kind="FileNotFoundError", errno=errno.ENOENT))
        self.assertNotIn("joined", event)

    def test_successful_fd_scan_cannot_hide_later_confirmed_original_terminal_state(self):
        sampler = self._fd_sampler(); real_scan = module.os.scandir
        def scan(path):
            if Path(path) == self.proc / "200/fd":
                self._set_state(200, "Z", 1)
            return real_scan(path)
        with mock.patch.object(module.os, "scandir", side_effect=scan):
            helper = sampler.sample()["groups"]["helper"]
        self.assertEqual(helper["root_state"], "retired")
        self.assertEqual(helper["live_processes"], 0)
        self.assertEqual(helper["retired_processes"], 1)
        self.assertEqual(helper["open_resource_unavailable_processes"], 0)
        event = sampler.summary()["fd_observation_diagnostics"]["events"]["retired"][0]
        self.assertEqual(event["phase"], "fd-post-identity")
        self.assertIsNone(event["error"])
        self.assertEqual(event["after"], event["confirmation"])

    def test_zombie_leader_with_live_or_unknown_threads_remains_unavailable(self):
        for threads in (0, 2):
            for transition_during_fd_read in (False, True):
                with self.subTest(threads=threads, transition=transition_during_fd_read):
                    sampler = self._fd_sampler(); real_scan = module.os.scandir
                    if not transition_during_fd_read: self._set_state(200, "Z", threads)
                    def scan(path):
                        if Path(path) == self.proc / "200/fd":
                            self._set_state(200, "Z", threads)
                            raise FileNotFoundError(errno.ENOENT, "not a zero table")
                        return real_scan(path)
                    with mock.patch.object(module.os, "scandir", side_effect=scan):
                        helper = sampler.sample()["groups"]["helper"]
                    self.assertIsNone(helper["sampled_open_descriptors"])
                    self.assertEqual(helper["open_resource_unavailable_processes"], 1)
                    self.assertEqual(helper["retired_processes"], 0)
                    self.assertTrue(sampler.summary()["fd_observation_diagnostics"]["events"]["unavailable"])

    def test_terminal_confirmation_cannot_hide_same_birth_live_exec_or_unknown_identity(self):
        for later in ("live", "permission", "invalid"):
            with self.subTest(later=later):
                sampler = self._fd_sampler(); real_scan = module.os.scandir; real_stat = sampler._stat
                after_fd = False; observed_terminal = False
                def scan(path):
                    nonlocal after_fd
                    if Path(path) == self.proc / "200/fd":
                        after_fd = True; self._set_state(200, "Z", 1)
                        raise FileNotFoundError(errno.ENOENT, "unavailable")
                    return real_scan(path)
                def observed_stat(pid):
                    nonlocal observed_terminal
                    if pid == 200 and after_fd and observed_terminal:
                        if later == "permission": raise PermissionError(errno.EACCES, "private secret")
                        if later == "invalid": raise ValueError("private malformed metadata")
                    value = real_stat(pid)
                    if pid == 200 and after_fd and not observed_terminal:
                        observed_terminal = True
                        if later == "live": self._set_state(200, "S", 1)
                    return value
                with mock.patch.object(module.os, "scandir", side_effect=scan), mock.patch.object(sampler, "_stat", side_effect=observed_stat):
                    if later == "invalid":
                        with self.assertRaisesRegex(ValueError, "malformed metadata"):
                            sampler.sample()
                        helper = None
                    else:
                        helper = sampler.sample()["groups"]["helper"]
                if helper is not None:
                    self.assertIsNone(helper["sampled_open_descriptors"])
                    self.assertEqual(helper["retired_processes"], 0)
                event = sampler.summary()["fd_observation_diagnostics"]["events"]["unavailable"][0]
                if later == "live": self.assertEqual(event["confirmation"]["state"], "S")
                else: self.assertIsNotNone(event["identity_error"])

    def test_fd_permission_io_and_live_missing_fail_without_zero_or_join_inference(self):
        for code in (errno.EACCES, errno.EIO, errno.ENOENT):
            with self.subTest(errno=code):
                sampler = self._fd_sampler(); real_scan = module.os.scandir
                def scan(path):
                    if Path(path) == self.proc / "200/fd":
                        # Even terminal metadata cannot erase permission/EIO.
                        if code != errno.ENOENT: self._set_state(200, "Z", 1)
                        raise OSError(code, "private credential and path", str(path))
                    return real_scan(path)
                with mock.patch.object(module.os, "scandir", side_effect=scan):
                    helper = sampler.sample()["groups"]["helper"]
                self.assertIsNone(helper["sampled_open_descriptors"])
                self.assertEqual(helper["open_resource_unavailable_processes"], 1)
                self.assertEqual(helper["retired_processes"], 0)
                event = sampler.summary()["fd_observation_diagnostics"]["events"]["unavailable"][0]
                self.assertEqual(event["error"]["errno"], code)

        # Initial metadata failure is unknown, even before any FD enumeration.
        for error in (PermissionError(errno.EACCES, "private path"),
                      OSError(errno.EIO, "private path"), ValueError("private stat")):
            with self.subTest(initial_error=type(error).__name__):
                sampler = self._fd_sampler(); original_stat = sampler._stat
                def stat(pid):
                    if pid == 200: raise error
                    return original_stat(pid)
                with mock.patch.object(sampler, "_stat", side_effect=stat):
                    helper = sampler.sample()["groups"]["helper"]
                self.assertEqual(helper["root_state"], "unavailable")
                self.assertEqual(helper["open_resource_unavailable_processes"], 1)
                self.assertIsNone(helper["sampled_open_descriptors"])
                self.assertIsNone(helper["sampled_threads"])
                self.assertEqual(helper["retired_processes"], 0)
                event = sampler.summary()["fd_observation_diagnostics"]["events"]["unavailable"][0]
                self.assertEqual(event["phase"], "initial-process-metadata")
                self.assertEqual(event["error"]["kind"], type(error).__name__)
                self.assertIsNone(event["before"])
                self.assertIsNone(sampler.summary()["groups"]["helper"]["peak_sampled_threads"])
                sampler.sample()  # A later measurable sample cannot erase unknown history.
                summary = sampler.summary()["groups"]["helper"]
                self.assertEqual(summary["peak_sampled_threads"], 1)
                self.assertEqual(summary["open_resource_unavailable_samples"], 1)

    def test_fd_pid_reuse_retires_only_original_and_never_adopts_successor(self):
        sampler = self._fd_sampler(); real_scan = module.os.scandir
        def scan(path):
            if Path(path) == self.proc / "200/fd":
                self.write(200, 0, 200, 99, 999, 999, "201")
                self.write(201, 200, 200, 99, 999, 999)
                raise FileNotFoundError(errno.ENOENT, "original is gone")
            return real_scan(path)
        with mock.patch.object(module.os, "scandir", side_effect=scan):
            helper = sampler.sample()["groups"]["helper"]
        self.assertEqual(helper["root_state"], "retired")
        self.assertEqual(helper["live_processes"], 0)
        self.assertFalse(any(key[0] == 201 for key in sampler.known))
        event = sampler.summary()["fd_observation_diagnostics"]["events"]["retired"][0]
        self.assertEqual(event["original_start_ticks"], 1)
        self.assertEqual(event["after"]["start_ticks"], 99)
        self.assertNotIn("joined", event)

    def test_fd_diagnostics_are_bounded_numeric_and_do_not_recover_unknown_samples(self):
        sampler = self._fd_sampler(); real_scan = module.os.scandir
        def scan(path):
            if Path(path) == self.proc / "200/fd":
                raise PermissionError(errno.EACCES, "private secret-bearer", str(path))
            return real_scan(path)
        with mock.patch.object(module.os, "scandir", side_effect=scan):
            for _ in range(40):
                helper = sampler.sample()["groups"]["helper"]
                self.assertIsNone(helper["sampled_open_descriptors"])
        summary = sampler.summary();d = summary["fd_observation_diagnostics"]
        self.assertEqual(len(d["events"]["unavailable"]), 16)
        self.assertEqual(d["overflow"]["unavailable"], 24)
        self.assertEqual(summary["groups"]["helper"]["open_resource_unavailable_samples"], 40)
        serialized = json.dumps(d)
        for private in ("secret-bearer", "private secret", str(self.proc), "private name"):
            self.assertNotIn(private, serialized)

    def test_fd_enumeration_limit_stays_hard_failure_with_closed_diagnostic(self):
        sampler = self._fd_sampler(); real_scan = module.os.scandir
        class TooMany:
            def __enter__(self): return iter(range(4097))
            def __exit__(self, *args): return False
        def scan(path):
            return TooMany() if Path(path) == self.proc / "200/fd" else real_scan(path)
        with mock.patch.object(module.os, "scandir", side_effect=scan):
            with self.assertRaisesRegex(ValueError, "descriptor observation budget"):
                sampler.sample()
        event = sampler.summary()["fd_observation_diagnostics"]["events"]["unavailable"][0]
        self.assertEqual(event["error"], dict(kind="ValueError", errno=None))
        self.assertEqual(event["phase"], "fd-enumeration")

if __name__=="__main__":unittest.main()
