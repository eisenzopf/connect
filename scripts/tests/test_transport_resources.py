"""Exact owned-process counterexamples for native resource collection."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from lib.transport_resources import run_profiled


class NativeProfileProcessBoundary(unittest.TestCase):
    def setUp(self):
        self.previous_umask = os.umask(0o077)
        self.addCleanup(os.umask, self.previous_umask)
        (ROOT / ".local").mkdir(exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=ROOT / ".local")
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.server = subprocess.Popen([sys.executable, "-c", "import time;time.sleep(20)"],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            start_new_session=True)
        self.addCleanup(self.stop_server)

    def stop_server(self):
        self.server.terminate()
        self.server.wait(timeout=5)

    def run_command(self, code, budget):
        return run_profiled([sys.executable, "-c", code], self.directory / "command.log",
            dict(os.environ), budget, server_pid=self.server.pid,
            directory=self.directory / "resources", cwd=ROOT)

    def test_joined_original_tree_has_separate_numeric_server_and_helper_observations(self):
        command = "import subprocess,sys;p=subprocess.Popen([sys.executable,'-c','import time;hold=bytearray(2000000);time.sleep(1.4)']);p.wait()"
        code, observed = self.run_command(command, 5)
        self.assertEqual(code, 0)
        self.assertTrue(observed["collection_complete"])
        self.assertTrue(observed["original_supervisor_joined"])
        self.assertTrue(observed["original_workers_joined"])
        self.assertFalse(observed["forced"])
        self.assertEqual(set(observed["groups"]), {"server", "helper"})
        self.assertEqual(observed["groups"]["server"]["peak_live_processes"], 1)
        self.assertGreaterEqual(observed["groups"]["helper"]["peak_live_processes"], 3)
        for group in observed["groups"].values():
            self.assertGreater(group["peak_sampled_rss_bytes"], 0)
            self.assertGreater(group["peak_sampled_open_descriptors"], 0)
            self.assertGreater(group["peak_sampled_threads"], 0)
        private = json.loads((self.directory / "resources/process-resources.json").read_text())
        self.assertEqual(private["observation"]["samples"][-1]["groups"]["helper"]["root_state"], "retired")
        self.assertNotIn(str(self.directory), json.dumps(observed))
        self.assertFalse(observed["resource_reclamation_qualified"])

    def test_early_command_exit_cannot_abandon_an_unsampled_original_worker(self):
        pid_path = self.directory / "original-worker.pid"
        command = ("import subprocess,sys,pathlib;"
            "p=subprocess.Popen([sys.executable,'-c','import time;time.sleep(20)']);"
            f"pathlib.Path({str(pid_path)!r}).write_text(str(p.pid))")
        code, summary = self.run_command(command, 2)
        self.assertEqual(code, 0)  # The controller's exit alone cannot pass.
        self.assertFalse(summary["collection_complete"])
        self.assertEqual(summary["failure"], "ValueError")
        pid = int(pid_path.read_text())
        state = Path(f"/proc/{pid}/stat")
        self.assertTrue(not state.exists() or state.read_text().split(") ", 1)[1].split()[0] in {"Z", "X"})
        observed = json.loads((self.directory / "resources/process-resources.json").read_text())
        self.assertTrue(observed["original_supervisor_joined"])
        self.assertFalse(observed["original_workers_joined"])
        self.assertTrue(observed["forced"])
        self.assertFalse(observed["collection_complete"])


if __name__ == "__main__":
    unittest.main()
