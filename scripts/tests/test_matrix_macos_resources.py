"""The Mac idle collector must report original live, retired and unavailable owners."""
from pathlib import Path
import select
import subprocess
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.matrix_macos_resources import MacProcessResources


@unittest.skipUnless(sys.platform == "darwin", "Real macOS libproc observations required")
class MacProcessSummary(unittest.TestCase):
    def setUp(self):
        self.owner = subprocess.Popen([sys.executable, "-c", "import time;print('ready',flush=True);time.sleep(30)"],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, start_new_session=True)
        self.addCleanup(self.retire)
        self.assertTrue(select.select([self.owner.stdout], [], [], 5)[0])
        self.assertEqual(self.owner.stdout.readline(), b"ready\n")
        self.sampler = MacProcessResources({"browser": self.owner.pid}, roles=("browser",))

    def retire(self):
        if self.owner.poll() is None:
            self.owner.terminate()
            self.owner.wait(timeout=5)
        self.owner.stdout.close()

    def test_summary_preserves_live_observation_after_original_owner_retires(self):
        live = self.sampler.sample()
        self.retire()
        retired = self.sampler.sample()
        report = self.sampler.summary()
        self.assertEqual(report["sample_count"], 2)
        self.assertEqual(report["samples"], [live, retired])
        self.assertEqual(retired["groups"]["browser"]["root_state"], "retired")
        self.assertEqual(retired["groups"]["browser"]["live_processes"], 0)
        measured = report["groups"]["browser"]
        self.assertEqual(measured["peak_live_processes"], 1)
        self.assertGreater(measured["peak_sampled_rss_bytes"], 0)
        self.assertGreaterEqual(measured["peak_sampled_open_descriptors"], 3)
        self.assertGreaterEqual(measured["peak_sampled_threads"], 1)
        self.assertEqual(measured["unavailable_samples"], 0)
        self.assertEqual(measured["open_resource_unavailable_samples"], 0)
        self.assertIn("macOS", report["scope"])
        # Summarizing evidence must not resample a possibly reused process ID.
        self.assertEqual(len(self.sampler.samples), 2)

    def test_unavailable_original_observation_is_not_reported_as_zero_usage(self):
        with patch.object(self.sampler, "read", side_effect=PermissionError("fixture observation denied")):
            self.sampler.sample()
        report = self.sampler.summary()
        measured = report["groups"]["browser"]
        self.assertEqual(measured["unavailable_samples"], 1)
        self.assertEqual(measured["open_resource_unavailable_samples"], 1)
        self.assertIsNone(measured["peak_sampled_open_descriptors"])
        self.assertEqual(report["samples"][0]["groups"]["browser"]["root_state"], "unavailable")


if __name__ == "__main__":
    unittest.main()
